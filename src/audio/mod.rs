//! audio backend: pipewire on linux, coreaudio (into blackhole) on macos,
//! and a test-signal stub elsewhere.

#[cfg(target_os = "linux")]
mod pipewire;
#[cfg(target_os = "linux")]
pub use self::pipewire::Backend;

#[cfg(target_os = "macos")]
mod coreaudio;
#[cfg(target_os = "macos")]
pub use self::coreaudio::Backend;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod stub;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub use self::stub::Backend;

/// whether the backend plays into a user-chosen output device (macos)
pub const HAS_OUTPUT_PICKER: bool = cfg!(target_os = "macos");

pub const SAMPLE_RATE: u32 = 48_000;
/// requested graph quantum. 256 frames is ~5.3 ms; with the ring between
/// capture and playback the end-to-end latency stays around 10 ms.
pub const QUANTUM: u32 = 256;
/// largest block the engine will ever be asked to process
pub const MAX_BLOCK: usize = 8192;

/// node.name of the virtual source
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const SOURCE_NODE_NAME: &str = "fsa.processed";
/// node.description of the virtual source, what discord shows
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const SOURCE_DESCRIPTION: &str = "Mic (processed)";
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const CAPTURE_NODE_NAME: &str = "fsa.capture";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: u32,
    /// node.name, stable across restarts; used as target.object
    pub name: String,
    pub description: String,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub enum Ctrl {
    /// switch the capture to another source (`None` = session default)
    SetInput(Option<String>),
    /// switch the device the processed signal is played into (macos only;
    /// `None` = the first blackhole device)
    SetOutput(#[cfg_attr(target_os = "linux", allow(dead_code))] Option<String>),
    Quit,
}
