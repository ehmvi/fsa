//! macos backend: coreaudio via cpal.
//!
//! macos has no way for an app to publish a virtual mic by itself, so fsa
//! captures the chosen input, processes it, and plays the result into a
//! virtual loopback device (blackhole by default). discord then uses
//! "BlackHole 2ch" as its input.
//!
//! the input and output devices run on different clocks, so the output
//! callback keeps the ring between them at a fixed fill level by stretching
//! or squeezing a block by one sample (linear interpolation) whenever it
//! drifts. that keeps latency constant without periodic clicks.
//!
//! the engine and the ring ends live in `Exclusive` cells so streams can
//! be rebuilt on device changes without moving state out of a callback.

use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Device as CpalDevice, DeviceId, SampleFormat, Stream, StreamConfig,
    SupportedBufferSize,
};

use super::{Ctrl, Device, MAX_BLOCK, QUANTUM, SAMPLE_RATE};
use crate::engine::Engine;
use crate::shared::{InputChannel, Shared};

/// pseudo input device that generates a test signal instead of capturing
pub const TEST_SIGNAL: &str = "fsa.test-signal";
/// ring capacity between input and output, in samples
const RING: usize = 16384;
/// fill level the output keeps in the ring (~10.7 ms), covers block-size
/// mismatches between the two devices
const TARGET: usize = 512;
/// drift tolerance around the target before a sample gets slipped
const SLACK: usize = 128;
const REFRESH: Duration = Duration::from_secs(2);

/// a value only one callback may use at a time. acquiring never waits: if
/// it's busy (only possible for an instant while streams are swapped) the
/// caller skips the block.
struct Exclusive<T> {
    busy: AtomicBool,
    value: UnsafeCell<T>,
}

// safety: access to `value` is serialised by `busy`
unsafe impl<T: Send> Sync for Exclusive<T> {}

impl<T> Exclusive<T> {
    fn new(value: T) -> Arc<Self> {
        Arc::new(Self {
            busy: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        })
    }

    fn try_with(&self, f: impl FnOnce(&mut T)) -> bool {
        if self
            .busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return false;
        }
        // safety: we hold the busy flag
        f(unsafe { &mut *self.value.get() });
        self.busy.store(false, Ordering::Release);
        true
    }
}

struct Processor {
    engine: Engine,
    shared: Arc<Shared>,
    producer: rtrb::Producer<f32>,
    mono: Box<[f32]>,
    out: Box<[f32]>,
}

impl Processor {
    /// interleaved input -> mono -> engine -> ring
    fn process(&mut self, data: &[f32], channels: usize) {
        let channel = self.shared.input_channel();
        for frames in data.chunks(MAX_BLOCK * channels.max(1)) {
            let n = frames.len() / channels.max(1);
            for (i, frame) in frames.chunks_exact(channels.max(1)).enumerate() {
                let l = frame[0];
                let r = frame.get(1).copied().unwrap_or(l);
                self.mono[i] = match channel {
                    InputChannel::Left => l,
                    InputChannel::Right => r,
                    InputChannel::Mix => 0.5 * (l + r),
                };
            }
            self.engine.process(&self.mono[..n], &mut self.out[..n]);
            let _ = self.producer.push_partial_slice(&self.out[..n]);
        }
    }
}

struct Output {
    consumer: rtrb::Consumer<f32>,
    shared: Arc<Shared>,
    priming: bool,
}

impl Output {
    /// fills one interleaved output block, mono copied to the first two channels
    fn render(&mut self, out: &mut [f32], channels: usize) {
        out.fill(0.0);
        let channels = channels.max(1);
        let frames = out.len() / channels;
        if frames == 0 {
            return;
        }
        let avail = self.consumer.slots();
        if self.priming {
            if avail < frames + TARGET {
                return;
            }
            self.priming = false;
        }
        // a long stall (e.g. output switched) left too much buffered: drop it
        if avail > frames + TARGET + 8 * SLACK
            && let Ok(chunk) = self.consumer.read_chunk(avail - frames - TARGET)
        {
            chunk.commit_all();
        }
        let avail = self.consumer.slots();
        // slip one sample when the two clocks drift apart
        let take = if avail > frames + TARGET + SLACK {
            frames + 1
        } else if avail + SLACK < frames + TARGET && frames > 1 {
            frames - 1
        } else {
            frames
        };
        let Ok(chunk) = self.consumer.read_chunk(take.min(avail)) else {
            return;
        };
        let got = chunk.len();
        let (a, b) = chunk.as_slices();
        let at = |j: usize| if j < a.len() { a[j] } else { b[j - a.len()] };
        if got < take {
            // underrun: play what we have and rebuild the cushion
            for (i, frame) in out.chunks_exact_mut(channels).take(got).enumerate() {
                write_frame(frame, at(i));
            }
            self.priming = true;
            self.shared.xruns.fetch_add(1, Ordering::Relaxed);
        } else if take == frames {
            for (i, frame) in out.chunks_exact_mut(channels).enumerate() {
                write_frame(frame, at(i));
            }
        } else {
            // resample `take` samples onto `frames` slots
            let step = (take - 1) as f32 / (frames - 1).max(1) as f32;
            for (i, frame) in out.chunks_exact_mut(channels).enumerate() {
                let pos = i as f32 * step;
                let j = pos as usize;
                let frac = pos - j as f32;
                let s0 = at(j);
                let s1 = if j + 1 < take { at(j + 1) } else { s0 };
                write_frame(frame, s0 + (s1 - s0) * frac);
            }
        }
        chunk.commit_all();
    }
}

fn write_frame(frame: &mut [f32], s: f32) {
    for c in frame.iter_mut().take(2) {
        *c = s;
    }
}

pub struct Backend {
    tx: Sender<Ctrl>,
    thread: Option<JoinHandle<()>>,
}

impl Backend {
    pub fn start(
        shared: Arc<Shared>,
        engine: Engine,
        input: Option<String>,
        output: Option<String>,
    ) -> anyhow::Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("coreaudio-ctl".into())
            .spawn(move || control(shared, engine, input, output, rx))?;
        Ok(Self {
            tx,
            thread: Some(thread),
        })
    }

    pub fn send(&self, ctrl: Ctrl) {
        let _ = self.tx.send(ctrl);
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.tx.send(Ctrl::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// what's currently feeding the processor
enum Source {
    None,
    /// kept alive: the stream stops when dropped
    Device(#[allow(dead_code)] Stream),
    TestSignal {
        quit: Arc<AtomicBool>,
        thread: JoinHandle<()>,
    },
}

impl Source {
    fn stop(self) {
        if let Source::TestSignal { quit, thread } = self {
            quit.store(true, Ordering::Relaxed);
            let _ = thread.join();
        }
        // a cpal stream stops when dropped
    }
}

struct State {
    host: cpal::Host,
    shared: Arc<Shared>,
    processor: Arc<Exclusive<Processor>>,
    output: Arc<Exclusive<Output>>,
    input_name: Option<String>,
    output_name: Option<String>,
    source: Source,
    sink: Option<Stream>,
    /// set by stream error callbacks (device unplugged etc.)
    failed: Arc<AtomicBool>,
    input_status: String,
    output_status: String,
}

fn control(
    shared: Arc<Shared>,
    engine: Engine,
    input: Option<String>,
    output: Option<String>,
    rx: Receiver<Ctrl>,
) {
    let (producer, consumer) = rtrb::RingBuffer::<f32>::new(RING);
    let mut st = State {
        host: cpal::default_host(),
        processor: Exclusive::new(Processor {
            engine,
            shared: shared.clone(),
            producer,
            mono: vec![0.0; MAX_BLOCK].into_boxed_slice(),
            out: vec![0.0; MAX_BLOCK].into_boxed_slice(),
        }),
        output: Exclusive::new(Output {
            consumer,
            shared: shared.clone(),
            priming: true,
        }),
        shared,
        input_name: input,
        output_name: output,
        source: Source::None,
        sink: None,
        failed: Arc::new(AtomicBool::new(false)),
        input_status: String::new(),
        output_status: String::new(),
    };
    refresh_devices(&st);
    rebuild_input(&mut st);
    rebuild_output(&mut st);
    let mut last_refresh = Instant::now();
    loop {
        match rx.recv_timeout(REFRESH) {
            Ok(Ctrl::Quit) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Ctrl::SetInput(name)) => {
                st.input_name = name;
                rebuild_input(&mut st);
            }
            Ok(Ctrl::SetOutput(name)) => {
                st.output_name = name;
                rebuild_output(&mut st);
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        if last_refresh.elapsed() >= REFRESH {
            last_refresh = Instant::now();
            refresh_devices(&st);
            // retry after errors (device unplugged) and while a device is missing
            let input_down = matches!(st.source, Source::None);
            if st.failed.swap(false, Ordering::Relaxed) || input_down {
                rebuild_input(&mut st);
            }
            if st.sink.is_none() || input_down {
                rebuild_output(&mut st);
            }
        }
    }
    std::mem::replace(&mut st.source, Source::None).stop();
    st.sink = None;
}

fn set_status(st: &State) {
    let s = match (st.input_status.as_str(), st.output_status.as_str()) {
        ("", "") => "running".to_string(),
        (i, "") => i.to_string(),
        ("", o) => o.to_string(),
        (i, o) => format!("{i} · {o}"),
    };
    st.shared.set_status(s);
}

fn device_info(d: &CpalDevice) -> Option<Device> {
    let id = d.id().ok()?.to_string();
    let description = d.to_string();
    Some(Device {
        id: 0,
        name: id,
        description,
    })
}

fn refresh_devices(st: &State) {
    let mut inputs: Vec<Device> = st
        .host
        .input_devices()
        .map(|it| it.filter_map(|d| device_info(&d)).collect())
        .unwrap_or_default();
    inputs.push(Device {
        id: 0,
        name: TEST_SIGNAL.into(),
        description: "Test signal (no mic)".into(),
    });
    let outputs: Vec<Device> = st
        .host
        .output_devices()
        .map(|it| it.filter_map(|d| device_info(&d)).collect())
        .unwrap_or_default();
    *st.shared.devices.lock().unwrap() = inputs;
    *st.shared.outputs.lock().unwrap() = outputs;
}

fn find(st: &State, name: &str) -> Option<CpalDevice> {
    let id: DeviceId = name.parse().ok()?;
    st.host.device_by_id(&id)
}

/// picks an f32 config at 48 khz, preferring stereo or mono, with a small fixed buffer
fn pick_config(
    configs: impl Iterator<Item = cpal::SupportedStreamConfigRange>,
) -> anyhow::Result<StreamConfig> {
    let mut best: Option<cpal::SupportedStreamConfigRange> = None;
    for c in configs {
        if c.sample_format() != SampleFormat::F32
            || c.min_sample_rate() > SAMPLE_RATE
            || c.max_sample_rate() < SAMPLE_RATE
        {
            continue;
        }
        let score = |ch: u16| {
            if ch == 2 {
                0
            } else if ch == 1 {
                1
            } else {
                2 + ch
            }
        };
        if best
            .as_ref()
            .is_none_or(|b| score(c.channels()) < score(b.channels()))
        {
            best = Some(c);
        }
    }
    let c = best.context("device doesn't offer 32-bit float at 48 kHz")?;
    let buffer_size = match c.buffer_size() {
        SupportedBufferSize::Range { min, max } if (*min..=*max).contains(&QUANTUM) => {
            BufferSize::Fixed(QUANTUM)
        }
        _ => BufferSize::Default,
    };
    Ok(StreamConfig {
        channels: c.channels(),
        sample_rate: SAMPLE_RATE,
        buffer_size,
    })
}

fn rebuild_input(st: &mut State) {
    std::mem::replace(&mut st.source, Source::None).stop();
    st.input_status = match start_input(st) {
        Ok(source) => {
            st.source = source;
            String::new()
        }
        Err(e) => {
            log::warn!("input: {e:#}");
            format!("input: {e:#}")
        }
    };
    set_status(st);
}

fn start_input(st: &State) -> anyhow::Result<Source> {
    if st.input_name.as_deref() == Some(TEST_SIGNAL) {
        return Ok(start_test_signal(st.processor.clone()));
    }
    let device = match &st.input_name {
        Some(name) => {
            find(st, name).with_context(|| "selected input device not found".to_string())?
        }
        None => st
            .host
            .default_input_device()
            .context("no default input device")?,
    };
    let config = pick_config(device.supported_input_configs()?)?;
    let channels = config.channels as usize;
    let processor = st.processor.clone();
    let failed = st.failed.clone();
    let stream = device.build_input_stream::<f32, _, _>(
        config,
        move |data, _| {
            processor.try_with(|p| p.process(data, channels));
        },
        move |e| {
            log::warn!("input stream: {e}");
            failed.store(true, Ordering::Relaxed);
        },
        None,
    )?;
    stream.play()?;
    log::info!("capturing from {device}");
    Ok(Source::Device(stream))
}

fn rebuild_output(st: &mut State) {
    st.sink = None;
    st.output_status = match start_output(st) {
        Ok(stream) => {
            st.sink = Some(stream);
            String::new()
        }
        Err(e) => {
            log::warn!("output: {e:#}");
            format!("output: {e:#}")
        }
    };
    set_status(st);
}

fn start_output(st: &State) -> anyhow::Result<Stream> {
    let device = match &st.output_name {
        Some(name) => find(st, name).context("selected output device not found")?,
        // never fall back to the speakers: that would feed the mic back into itself
        None => st
            .host
            .output_devices()?
            .find(|d| d.to_string().to_lowercase().contains("blackhole"))
            .ok_or_else(|| anyhow!("no BlackHole device, install it or pick an output"))?,
    };
    let config = pick_config(device.supported_output_configs()?)?;
    if config.channels == 0 {
        bail!("output device has no channels");
    }
    let channels = config.channels as usize;
    let output = st.output.clone();
    output.try_with(|o| o.priming = true);
    let failed = st.failed.clone();
    let stream = device.build_output_stream::<f32, _, _>(
        config,
        move |data, _| {
            if !output.try_with(|o| o.render(data, channels)) {
                data.fill(0.0);
            }
        },
        move |e| {
            log::warn!("output stream: {e}");
            failed.store(true, Ordering::Relaxed);
        },
        None,
    )?;
    stream.play()?;
    log::info!("playing into {device}");
    Ok(stream)
}

/// feeds the processor a noise floor plus periodic tone bursts in real time
fn start_test_signal(processor: Arc<Exclusive<Processor>>) -> Source {
    let quit = Arc::new(AtomicBool::new(false));
    let thread = std::thread::spawn({
        let quit = quit.clone();
        move || {
            let n = QUANTUM as usize;
            let period = Duration::from_secs_f64(n as f64 / SAMPLE_RATE as f64);
            let mut input = vec![0.0f32; n];
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
                processor.try_with(|p| p.process(&input, 1));
                next += period;
                std::thread::sleep(next.saturating_duration_since(Instant::now()));
            }
        }
    });
    Source::TestSignal { quit, thread }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(fill: usize) -> (Output, rtrb::Producer<f32>) {
        let (mut p, c) = rtrb::RingBuffer::new(RING);
        for i in 0..fill {
            p.push(i as f32).unwrap();
        }
        (
            Output {
                consumer: c,
                shared: Arc::new(Shared::default()),
                priming: false,
            },
            p,
        )
    }

    #[test]
    fn steady_state_passes_samples_through() {
        let (mut o, _p) = output(256 + TARGET);
        let mut buf = [0.0f32; 512];
        o.render(&mut buf, 2);
        let left: Vec<f32> = buf.chunks(2).map(|f| f[0]).collect();
        assert_eq!(left, (0..256).map(|i| i as f32).collect::<Vec<_>>());
        assert_eq!(o.consumer.slots(), TARGET);
    }

    #[test]
    fn surplus_slips_one_sample() {
        let (mut o, _p) = output(256 + TARGET + SLACK + 10);
        let mut buf = [0.0f32; 256];
        o.render(&mut buf, 1);
        assert_eq!(o.consumer.slots(), TARGET + SLACK + 10 - 1);
        // stretched ramp: monotonic, ends at the 257th sample
        assert!(buf.windows(2).all(|w| w[1] > w[0]));
        assert!((buf[255] - 256.0).abs() < 1e-3);
    }

    #[test]
    fn underrun_primes_and_counts() {
        let (mut o, mut p) = output(100);
        let mut buf = [1.0f32; 256];
        o.render(&mut buf, 1);
        assert!(o.priming);
        assert_eq!(o.shared.xruns.load(Ordering::Relaxed), 1);
        // stays silent until the cushion is back
        for i in 0..300 {
            p.push(i as f32).unwrap();
        }
        o.render(&mut buf, 1);
        assert!(buf.iter().all(|&s| s == 0.0));
    }
}
