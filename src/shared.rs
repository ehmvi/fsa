//! state shared between the gui, the pipewire main loop and the audio thread.
//! the audio thread only ever touches atomics here.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

use crate::audio::Device;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputChannel {
    #[default]
    Left,
    Right,
    Mix,
}

impl InputChannel {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Right,
            2 => Self::Mix,
            _ => Self::Left,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "left" | "l" => Some(Self::Left),
            "right" | "r" => Some(Self::Right),
            "mix" | "m" => Some(Self::Mix),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct Shared {
    /// which channel of the (stereo) capture to use, see `InputChannel`
    pub input_channel: AtomicU8,
    /// playback underruns, for diagnostics
    pub xruns: AtomicU32,
    /// audio sources currently known to pipewire (non-rt only)
    pub devices: Mutex<Vec<Device>>,
    /// human readable backend state (non-rt only)
    pub status: Mutex<String>,
}

impl Shared {
    pub fn input_channel(&self) -> InputChannel {
        InputChannel::from_u8(self.input_channel.load(Ordering::Relaxed))
    }

    pub fn set_input_channel(&self, ch: InputChannel) {
        self.input_channel.store(ch as u8, Ordering::Relaxed);
    }

    pub fn set_status(&self, s: impl Into<String>) {
        *self.status.lock().unwrap() = s.into();
    }
}
