//! pipewire backend.
//!
//! two streams, the same pattern as module-loopback / pw-loopback:
//! - a capture stream (Stream/Input/Audio) linked to the chosen source,
//! - a mono playback stream with media.class Audio/Source, which is what
//!   shows up as "Mic (processed)" in discord.
//!
//! a plain pw_filter is not used because wireplumber neither auto-links a
//! filter's input nor offers it to applications as a source.
//!
//! the engine runs in the capture callback (so meters keep moving even
//! when nobody is recording) and hands samples to the playback callback
//! through an rtrb ring. both streams share node.group so they run on the
//! same driver and cannot drift apart.

use std::ffi::CString;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;

use anyhow::{Context as _, anyhow};
use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use pw::spa::pod::Pod;
use pw::stream::{StreamFlags, StreamRc, StreamState};

use super::{
    CAPTURE_NODE_NAME, Ctrl, Device, MAX_BLOCK, QUANTUM, SAMPLE_RATE, SOURCE_DESCRIPTION,
    SOURCE_NODE_NAME,
};
use crate::engine::Engine;
use crate::shared::{InputChannel, Shared};

/// ring capacity between capture and playback, in samples
const RING: usize = 16384;
/// safety margin kept in the ring after an underrun (~1.3 ms)
const MARGIN: usize = 64;
const GROUP: &str = "fsa";

pub struct Backend {
    tx: pw::channel::Sender<Ctrl>,
    thread: Option<JoinHandle<()>>,
}

impl Backend {
    pub fn start(
        shared: Arc<Shared>,
        engine: Engine,
        input: Option<String>,
    ) -> anyhow::Result<Self> {
        let (tx, rx) = pw::channel::channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let thread = std::thread::Builder::new()
            .name("pw-main".into())
            .spawn(move || {
                let ready = ready_tx.clone();
                if let Err(e) = run(shared.clone(), engine, input, rx, ready_tx) {
                    log::error!("pipewire: {e:#}");
                    shared.set_status(format!("pipewire error: {e:#}"));
                    let _ = ready.send(Err(format!("{e:#}")));
                }
            })?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                tx,
                thread: Some(thread),
            }),
            Ok(Err(e)) => Err(anyhow!(e)),
            Err(_) => Err(anyhow!("pipewire thread died during startup")),
        }
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

struct CaptureData {
    engine: Engine,
    shared: Arc<Shared>,
    producer: rtrb::Producer<f32>,
    input: Box<[f32]>,
    output: Box<[f32]>,
}

struct PlaybackData {
    shared: Arc<Shared>,
    consumer: rtrb::Consumer<f32>,
    priming: bool,
}

fn run(
    shared: Arc<Shared>,
    engine: Engine,
    input: Option<String>,
    rx: pw::channel::Receiver<Ctrl>,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
) -> anyhow::Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).context("main loop")?;
    let context = pw::context::ContextRc::new(&mainloop, None).context("context")?;
    let core = context
        .connect_rc(None)
        .context("cannot connect to pipewire, is it running?")?;
    let registry = core.get_registry_rc().context("registry")?;

    // track audio sources for the device picker
    let _registry_listener = registry
        .add_listener_local()
        .global({
            let shared = shared.clone();
            move |global| {
                if global.type_ != pw::types::ObjectType::Node {
                    return;
                }
                let Some(props) = global.props else { return };
                let class = props.get("media.class").unwrap_or_default();
                let name = props.get("node.name").unwrap_or_default();
                if !class.starts_with("Audio/Source") || name == SOURCE_NODE_NAME || name.is_empty()
                {
                    return;
                }
                let description = props
                    .get("node.description")
                    .or_else(|| props.get("node.nick"))
                    .unwrap_or(name)
                    .to_string();
                let mut devices = shared.devices.lock().unwrap();
                devices.retain(|d| d.id != global.id);
                devices.push(Device {
                    id: global.id,
                    name: name.to_string(),
                    description,
                });
                devices.sort_by(|a, b| a.description.cmp(&b.description));
            }
        })
        .global_remove({
            let shared = shared.clone();
            move |id| shared.devices.lock().unwrap().retain(|d| d.id != id)
        })
        .register();

    let (producer, consumer) = rtrb::RingBuffer::<f32>::new(RING);
    let latency = format!("{QUANTUM}/{SAMPLE_RATE}");

    // capture side
    let mut capture_props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Production",
        *pw::keys::NODE_NAME => CAPTURE_NODE_NAME,
        *pw::keys::NODE_DESCRIPTION => "fsa input",
        *pw::keys::NODE_LATENCY => latency.as_str(),
        "node.group" => GROUP,
        // nodes in the same link group are never linked to each other, so the
        // capture can't pick up our own virtual source if it becomes default
        "node.link-group" => GROUP,
        "node.always-process" => "true",
    };
    if let Some(target) = &input {
        capture_props.insert(*pw::keys::TARGET_OBJECT, target.as_str());
    }
    let capture =
        StreamRc::new(core.clone(), "fsa-capture", capture_props).context("capture stream")?;

    let _capture_listener = capture
        .add_local_listener_with_user_data(CaptureData {
            engine,
            shared: shared.clone(),
            producer,
            input: vec![0.0; MAX_BLOCK].into_boxed_slice(),
            output: vec![0.0; MAX_BLOCK].into_boxed_slice(),
        })
        .state_changed(|_, data, _old, new| {
            let s = match &new {
                StreamState::Error(e) => format!("capture error: {e}"),
                StreamState::Unconnected => "capture unconnected".into(),
                StreamState::Connecting => "connecting…".into(),
                StreamState::Paused => "capture paused".into(),
                StreamState::Streaming => "running".into(),
            };
            log::info!("{s}");
            data.shared.set_status(s);
        })
        .process(capture_process)
        .register()
        .context("capture listener")?;

    // playback side: the virtual mic
    let playback_props = properties! {
        *pw::keys::MEDIA_CLASS => "Audio/Source",
        *pw::keys::NODE_NAME => SOURCE_NODE_NAME,
        *pw::keys::NODE_DESCRIPTION => SOURCE_DESCRIPTION,
        *pw::keys::NODE_LATENCY => latency.as_str(),
        "node.group" => GROUP,
        "node.link-group" => GROUP,
        "audio.position" => "[ MONO ]",
        "device.icon-name" => "audio-input-microphone",
    };
    let playback =
        StreamRc::new(core.clone(), "fsa-playback", playback_props).context("playback stream")?;
    let _playback_listener = playback
        .add_local_listener_with_user_data(PlaybackData {
            shared: shared.clone(),
            consumer,
            priming: true,
        })
        .process(playback_process)
        .register()
        .context("playback listener")?;

    let capture_format = format_pod(
        2,
        &[
            spa::sys::SPA_AUDIO_CHANNEL_FL,
            spa::sys::SPA_AUDIO_CHANNEL_FR,
        ],
    )?;
    let playback_format = format_pod(1, &[spa::sys::SPA_AUDIO_CHANNEL_MONO])?;

    let flags = StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS;
    capture
        .connect(
            spa::utils::Direction::Input,
            None,
            flags | StreamFlags::AUTOCONNECT,
            &mut [Pod::from_bytes(&capture_format).unwrap()],
        )
        .context("connect capture")?;
    playback
        .connect(
            spa::utils::Direction::Output,
            None,
            flags,
            &mut [Pod::from_bytes(&playback_format).unwrap()],
        )
        .context("connect playback")?;

    let _rx = rx.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        let capture = capture.clone();
        move |ctrl| match ctrl {
            Ctrl::Quit => mainloop.quit(),
            Ctrl::SetInput(target) => {
                log::info!("switching input to {target:?}");
                if let Err(e) = retarget(&capture, target.as_deref(), &capture_format) {
                    log::error!("switching input: {e:#}");
                }
            }
        }
    });

    let _ = ready.send(Ok(()));
    // keep the registry proxy alive for the listener
    let _registry = registry;
    let _context = context;
    mainloop.run();
    let _ = capture.disconnect();
    let _ = playback.disconnect();
    Ok(())
}

/// points the capture stream at another source by updating target.object
/// and reconnecting. the listener (and thus the engine state) is kept.
fn retarget(capture: &StreamRc, target: Option<&str>, format: &[u8]) -> anyhow::Result<()> {
    let value = target.map(CString::new).transpose()?;
    let item = spa::sys::spa_dict_item {
        key: c"target.object".as_ptr(),
        // a null value removes the key, i.e. follow the default source
        value: value.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
    };
    let dict = spa::sys::spa_dict {
        flags: 0,
        n_items: 1,
        items: &item,
    };
    // safety: the stream is alive and the dict outlives the call
    unsafe {
        pw::sys::pw_stream_update_properties(capture.as_raw_ptr(), &dict);
    }
    capture.disconnect()?;
    capture.connect(
        spa::utils::Direction::Input,
        None,
        StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS | StreamFlags::AUTOCONNECT,
        &mut [Pod::from_bytes(format).unwrap()],
    )?;
    Ok(())
}

fn format_pod(channels: u32, positions: &[u32]) -> anyhow::Result<Vec<u8>> {
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_rate(SAMPLE_RATE);
    info.set_channels(channels);
    let mut pos = [0u32; spa::sys::SPA_AUDIO_MAX_CHANNELS as usize];
    pos[..positions.len()].copy_from_slice(positions);
    info.set_position(pos);
    let obj = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: info.into(),
    };
    let bytes = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(obj),
    )
    .map_err(|e| anyhow!("serializing format: {e:?}"))?
    .0
    .into_inner();
    Ok(bytes)
}

// --- real-time callbacks: no allocation, no locks, no io below this line ---

fn capture_process(stream: &pw::stream::Stream, d: &mut CaptureData) {
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let datas = buffer.datas_mut();
    let Some(data) = datas.first_mut() else {
        return;
    };
    let offset = data.chunk().offset() as usize;
    let size = data.chunk().size() as usize;
    let Some(bytes) = data.data() else { return };
    let start = offset.min(bytes.len());
    let end = (offset + size).min(bytes.len());
    let bytes = &bytes[start..end];

    // interleaved stereo f32 -> mono
    let channel = d.shared.input_channel();
    let frames = (bytes.len() / 8).min(MAX_BLOCK);
    for (i, frame) in bytes.as_chunks::<8>().0.iter().take(frames).enumerate() {
        let l = f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]);
        let r = f32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]);
        d.input[i] = match channel {
            InputChannel::Left => l,
            InputChannel::Right => r,
            InputChannel::Mix => 0.5 * (l + r),
        };
    }

    d.engine
        .process(&d.input[..frames], &mut d.output[..frames]);
    // if nobody consumes the virtual source the ring fills up; drop the rest
    let _ = d.producer.push_partial_slice(&d.output[..frames]);
}

fn playback_process(stream: &pw::stream::Stream, d: &mut PlaybackData) {
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let requested = buffer.requested() as usize;
    let datas = buffer.datas_mut();
    let Some(data) = datas.first_mut() else {
        return;
    };
    let Some(bytes) = data.data() else { return };

    let max_frames = bytes.len() / 4;
    let n = if requested == 0 {
        max_frames.min(QUANTUM as usize)
    } else {
        requested.min(max_frames)
    };
    let out = &mut bytes[..n * 4];

    let avail = d.consumer.slots();
    if d.priming && avail >= n + MARGIN {
        d.priming = false;
    }
    let mut written = 0;
    if !d.priming {
        if let Ok(chunk) = d.consumer.read_chunk(avail.min(n)) {
            let (a, b) = chunk.as_slices();
            for (dst, s) in out.as_chunks_mut::<4>().0.iter_mut().zip(a.iter().chain(b)) {
                dst.copy_from_slice(&s.to_le_bytes());
            }
            written = chunk.len();
            chunk.commit_all();
        }
        if written < n {
            d.priming = true;
            d.shared.xruns.fetch_add(1, Ordering::Relaxed);
        }
        // latency crept up (e.g. after a period without consumers): drop the excess
        let left = d.consumer.slots();
        if left > n + 2 * MARGIN
            && let Ok(chunk) = d.consumer.read_chunk(left - MARGIN)
        {
            chunk.commit_all();
        }
    }
    out[written * 4..].fill(0);

    let chunk = data.chunk_mut();
    *chunk.offset_mut() = 0;
    *chunk.stride_mut() = 4;
    *chunk.size_mut() = (n * 4) as u32;
}
