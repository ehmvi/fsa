//! audio backend. on linux this is pipewire; elsewhere a stub so the gui
//! and dsp can be developed and tested without a pipewire daemon.

#[cfg(target_os = "linux")]
mod pipewire;
#[cfg(target_os = "linux")]
pub use self::pipewire::Backend;

#[cfg(not(target_os = "linux"))]
mod stub;
#[cfg(not(target_os = "linux"))]
pub use self::stub::Backend;

pub const SAMPLE_RATE: u32 = 48_000;
/// requested graph quantum. 256 frames is ~5.3 ms; with the ring between
/// capture and playback the end-to-end latency stays around 10 ms.
pub const QUANTUM: u32 = 256;
/// largest block the engine will ever be asked to process
pub const MAX_BLOCK: usize = 8192;

/// node.name of the virtual source
pub const SOURCE_NODE_NAME: &str = "fsa.processed";
/// node.description of the virtual source, what discord shows
pub const SOURCE_DESCRIPTION: &str = "Mic (processed)";
pub const CAPTURE_NODE_NAME: &str = "fsa.capture";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: u32,
    /// node.name, stable across restarts; used as target.object
    pub name: String,
    pub description: String,
}

pub enum Ctrl {
    /// switch the capture to another source (`None` = session default)
    SetInput(Option<String>),
    Quit,
}
