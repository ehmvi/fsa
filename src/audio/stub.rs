//! fake backend for non-linux hosts: drives the engine with a synthetic
//! signal (noise floor + periodic tone bursts) so the gui and dsp can be
//! developed without pipewire.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{Ctrl, Device, QUANTUM, SAMPLE_RATE, SOURCE_DESCRIPTION};
use crate::engine::Engine;
use crate::shared::Shared;

pub struct Backend {
    quit: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Backend {
    pub fn start(
        shared: Arc<Shared>,
        mut engine: Engine,
        _input: Option<String>,
        _output: Option<String>,
    ) -> anyhow::Result<Self> {
        shared.set_status(format!(
            "no pipewire on this platform: test signal into \"{SOURCE_DESCRIPTION}\""
        ));
        shared.devices.lock().unwrap().push(Device {
            id: 0,
            name: "test.signal".into(),
            description: "Test signal".into(),
        });
        let quit = Arc::new(AtomicBool::new(false));
        let thread = std::thread::spawn({
            let quit = quit.clone();
            move || {
                let n = QUANTUM as usize;
                let period = Duration::from_secs_f64(n as f64 / SAMPLE_RATE as f64);
                let (mut input, mut output) = (vec![0.0f32; n], vec![0.0f32; n]);
                let (mut t, mut seed) = (0u64, 0x1234_5678u32);
                let mut next = Instant::now();
                while !quit.load(Ordering::Relaxed) {
                    for s in input.iter_mut() {
                        seed ^= seed << 13;
                        seed ^= seed >> 17;
                        seed ^= seed << 5;
                        let noise = (seed as f32 / u32::MAX as f32 - 0.5) * 0.004;
                        let secs = t as f32 / SAMPLE_RATE as f32;
                        let tone = if secs % 2.0 < 1.0 {
                            0.25 * (secs * 220.0 * std::f32::consts::TAU).sin()
                        } else {
                            0.0
                        };
                        *s = noise + tone;
                        t += 1;
                    }
                    engine.process(&input, &mut output);
                    next += period;
                    std::thread::sleep(next.saturating_duration_since(Instant::now()));
                }
            }
        });
        Ok(Self {
            quit,
            thread: Some(thread),
        })
    }

    pub fn send(&self, _ctrl: Ctrl) {}
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.quit.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
