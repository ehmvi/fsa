//! the real-time processing chain. everything reachable from
//! `Engine::process` must be allocation-, lock- and io-free.
//!
//! the gui owns the authoritative chain layout and mirrors every change to
//! the audio thread as a `Command` through an rtrb ring. slots that leave
//! the chain are sent back through a second ring so they are dropped (and
//! deallocated) on the gui thread, never on the audio thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::audio::MAX_BLOCK;
use crate::dsp::eq::Eq;
use crate::dsp::gate::Gate;
use crate::dsp::{Fade, peak};
use crate::shared::Shared;

pub const MAX_SLOTS: usize = 32;

/// anything that can sit in the chain. processes mono audio in place.
pub trait Unit: Send {
    fn process(&mut self, buf: &mut [f32]);
    /// clear internal state; called when a slot is switched back on
    fn reset(&mut self);
}

impl Unit for Eq {
    fn process(&mut self, buf: &mut [f32]) {
        Eq::process(self, buf)
    }

    fn reset(&mut self) {
        Eq::reset(self)
    }
}

impl Unit for Gate {
    fn process(&mut self, buf: &mut [f32]) {
        Gate::process(self, buf)
    }

    fn reset(&mut self) {
        Gate::reset(self)
    }
}

/// per-slot state the gui can flip without going through the command ring
#[derive(Debug)]
pub struct SlotState {
    pub enabled: AtomicBool,
}

impl SlotState {
    pub fn new(enabled: bool) -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(enabled),
        })
    }
}

pub struct Slot {
    pub id: u32,
    pub unit: Box<dyn Unit>,
    state: Arc<SlotState>,
    fade: Fade,
    removing: bool,
}

impl Slot {
    pub fn new(id: u32, unit: Box<dyn Unit>, state: Arc<SlotState>, sample_rate: f32) -> Self {
        let fade = Fade::new(state.enabled.load(Ordering::Relaxed), sample_rate);
        Self {
            id,
            unit,
            state,
            fade,
            removing: false,
        }
    }
}

pub enum Command {
    /// append a slot at the end of the chain
    Insert(Slot),
    /// fade the slot out, then hand it back for dropping
    Remove(u32),
    /// move slot `id` to position `to`
    Move { id: u32, to: usize },
}

/// gui side of the engine
pub struct EngineHandle {
    commands: rtrb::Producer<Command>,
    garbage: rtrb::Consumer<Slot>,
}

impl EngineHandle {
    pub fn send(&mut self, cmd: Command) {
        if self.commands.push(cmd).is_err() {
            log::warn!("engine command queue full, dropping command");
        }
    }

    /// drops slots the audio thread is done with; call regularly
    pub fn collect_garbage(&mut self) -> Vec<Slot> {
        let mut out = Vec::new();
        while let Ok(slot) = self.garbage.pop() {
            out.push(slot);
        }
        out
    }
}

pub struct Engine {
    shared: Arc<Shared>,
    slots: Vec<Slot>,
    commands: rtrb::Consumer<Command>,
    garbage: rtrb::Producer<Slot>,
    bypass: Fade,
    dry: Box<[f32]>,
}

impl Engine {
    pub fn new(shared: Arc<Shared>, initial: Vec<Slot>, sample_rate: f32) -> (Self, EngineHandle) {
        let (cmd_tx, cmd_rx) = rtrb::RingBuffer::new(256);
        let (gc_tx, gc_rx) = rtrb::RingBuffer::new(MAX_SLOTS * 2);
        let mut slots = Vec::with_capacity(MAX_SLOTS);
        slots.extend(initial);
        let bypass = Fade::new(!shared.bypass.load(Ordering::Relaxed), sample_rate);
        let engine = Self {
            shared,
            slots,
            commands: cmd_rx,
            garbage: gc_tx,
            bypass,
            dry: vec![0.0; MAX_BLOCK].into_boxed_slice(),
        };
        (
            engine,
            EngineHandle {
                commands: cmd_tx,
                garbage: gc_rx,
            },
        )
    }

    fn apply_commands(&mut self) {
        while let Ok(cmd) = self.commands.pop() {
            match cmd {
                Command::Insert(slot) => {
                    if self.slots.len() < self.slots.capacity() {
                        self.slots.push(slot);
                    } else {
                        let _ = self.garbage.push(slot);
                    }
                }
                Command::Remove(id) => {
                    if let Some(s) = self.slots.iter_mut().find(|s| s.id == id) {
                        s.removing = true;
                    }
                }
                Command::Move { id, to } => {
                    if let Some(from) = self.slots.iter().position(|s| s.id == id) {
                        // remove + insert within capacity never reallocates
                        let slot = self.slots.remove(from);
                        let to = to.min(self.slots.len());
                        self.slots.insert(to, slot);
                    }
                }
            }
        }
        // hand fully faded-out removed slots back to the gui
        if let Some(pos) = self
            .slots
            .iter()
            .position(|s| s.removing && s.fade.is_silent())
            && self.garbage.slots() > 0
        {
            let slot = self.slots.remove(pos);
            let _ = self.garbage.push(slot);
        }
    }

    /// processes one mono block. `input` and `output` have the same length,
    /// at most `MAX_BLOCK`.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        self.apply_commands();
        let n = input.len();
        let dry = &mut self.dry[..n];
        output.copy_from_slice(input);
        self.shared.in_peak.raise(peak(input));

        for slot in &mut self.slots {
            let on = slot.state.enabled.load(Ordering::Relaxed) && !slot.removing;
            slot.fade.set(on);
            if slot.fade.is_silent() {
                continue;
            }
            if slot.fade.gain == 0.0 {
                // coming back from fully off: don't replay stale filter state
                slot.unit.reset();
            }
            dry.copy_from_slice(output);
            slot.unit.process(output);
            slot.fade.mix(dry, output);
        }

        self.bypass.set(!self.shared.bypass.load(Ordering::Relaxed));
        self.bypass.mix(input, output);
        self.shared.out_peak.raise(peak(output));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Gain(f32);
    impl Unit for Gain {
        fn process(&mut self, buf: &mut [f32]) {
            buf.iter_mut().for_each(|s| *s *= self.0);
        }
        fn reset(&mut self) {}
    }

    fn engine_with(units: Vec<(u32, f32)>) -> (Engine, EngineHandle, Vec<Arc<SlotState>>) {
        let shared = Arc::new(Shared::default());
        let states: Vec<_> = units.iter().map(|_| SlotState::new(true)).collect();
        let slots = units
            .iter()
            .zip(&states)
            .map(|(&(id, g), st)| Slot::new(id, Box::new(Gain(g)), st.clone(), 48_000.0))
            .collect();
        let (e, h) = Engine::new(shared, slots, 48_000.0);
        (e, h, states)
    }

    fn run(e: &mut Engine, blocks: usize) -> f32 {
        let input = [1.0f32; 256];
        let mut out = [0.0f32; 256];
        for _ in 0..blocks {
            e.process(&input, &mut out);
        }
        out[255]
    }

    #[test]
    fn chain_applies_in_order_and_toggles() {
        let (mut e, _h, states) = engine_with(vec![(1, 2.0), (2, 3.0)]);
        assert_eq!(run(&mut e, 1), 6.0);
        states[1].enabled.store(false, Ordering::Relaxed);
        assert_eq!(run(&mut e, 4), 2.0);
    }

    #[test]
    fn remove_fades_out_and_returns_slot() {
        let (mut e, mut h, _states) = engine_with(vec![(1, 2.0), (2, 3.0)]);
        h.send(Command::Remove(1));
        run(&mut e, 4);
        assert_eq!(run(&mut e, 1), 3.0);
        let gc = h.collect_garbage();
        assert_eq!(gc.len(), 1);
        assert_eq!(gc[0].id, 1);
    }

    #[test]
    fn move_reorders() {
        let (mut e, mut h, _s) = engine_with(vec![(1, 1.0), (2, 1.0), (3, 1.0)]);
        h.send(Command::Move { id: 3, to: 0 });
        run(&mut e, 1);
        let ids: Vec<_> = e.slots.iter().map(|s| s.id).collect();
        assert_eq!(ids, [3, 1, 2]);
    }
}
