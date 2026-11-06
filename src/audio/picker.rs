//! which source to capture from.
//!
//! in auto mode (the default) fsa follows the mic that was plugged in most
//! recently: the backend reports every hardware source that shows up while
//! it's running, and the capture moves to the newest one that's present.
//! when that one goes away it falls back to the previous one, then to the
//! session default. picking a device by hand pins it until auto is chosen
//! again.

use std::sync::Arc;

use super::Device;
use crate::shared::Shared;

/// how many recently connected mics are remembered
const MAX_RECENT: usize = 8;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum InputChoice {
    /// follow the most recently plugged in mic
    #[default]
    Auto,
    /// whatever the system default source is
    Default,
    /// a specific device by name, kept even while it's missing
    Device(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Picker {
    pub choice: InputChoice,
    /// most recently plugged in (or picked) first
    pub recent: Vec<String>,
}

impl Picker {
    pub fn new(choice: InputChoice, recent: Vec<String>) -> Self {
        let mut recent = recent;
        recent.truncate(MAX_RECENT);
        Self { choice, recent }
    }

    pub fn set_choice(&mut self, choice: InputChoice) {
        if let InputChoice::Device(name) = &choice {
            // so going back to auto keeps this mic until another one shows up
            self.remember(name);
        }
        self.choice = choice;
    }

    /// a mic was just plugged in
    pub fn remember(&mut self, name: &str) {
        self.recent.retain(|n| n != name);
        self.recent.insert(0, name.to_string());
        self.recent.truncate(MAX_RECENT);
    }

    /// the source to capture from, `None` = session default
    pub fn target(&self, devices: &[Device]) -> Option<String> {
        match &self.choice {
            InputChoice::Default => None,
            InputChoice::Device(name) => Some(name.clone()),
            InputChoice::Auto => self
                .recent
                .iter()
                .find(|n| devices.iter().any(|d| &d.name == *n))
                .cloned(),
        }
    }

    /// resolves the target against the devices currently known and tells the
    /// gui about it. returns the target.
    pub fn resolve(&self, shared: &Arc<Shared>) -> Option<String> {
        let target = self.target(&shared.devices.lock().unwrap());
        *shared.active_input.lock().unwrap() = target.clone();
        *shared.recent_inputs.lock().unwrap() = self.recent.clone();
        target
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devices(names: &[&str]) -> Vec<Device> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| Device {
                id: i as u32,
                name: n.to_string(),
                description: n.to_string(),
            })
            .collect()
    }

    #[test]
    fn auto_follows_the_newest_present_mic() {
        let mut p = Picker::default();
        let all = devices(&["builtin", "usb", "headset"]);
        // nothing plugged in while running yet: session default
        assert_eq!(p.target(&all), None);
        p.remember("usb");
        assert_eq!(p.target(&all).as_deref(), Some("usb"));
        p.remember("headset");
        assert_eq!(p.target(&all).as_deref(), Some("headset"));
        // headset unplugged: back to the usb mic, then to the default
        assert_eq!(
            p.target(&devices(&["builtin", "usb"])).as_deref(),
            Some("usb")
        );
        assert_eq!(p.target(&devices(&["builtin"])), None);
        // replugging the usb mic makes it the newest again
        p.remember("usb");
        assert_eq!(p.target(&all).as_deref(), Some("usb"));
    }

    #[test]
    fn manual_choice_is_pinned_and_remembered() {
        let mut p = Picker::default();
        p.remember("usb");
        p.set_choice(InputChoice::Device("builtin".into()));
        p.remember("headset");
        let all = devices(&["builtin", "usb", "headset"]);
        assert_eq!(p.target(&all).as_deref(), Some("builtin"));
        // stays pinned even while missing
        assert_eq!(p.target(&devices(&[])).as_deref(), Some("builtin"));
        p.set_choice(InputChoice::Default);
        assert_eq!(p.target(&all), None);
        // back to auto: the headset was plugged in after the manual pick
        p.set_choice(InputChoice::Auto);
        assert_eq!(p.target(&all).as_deref(), Some("headset"));
        assert_eq!(p.recent, ["headset", "builtin", "usb"]);
    }

    #[test]
    fn history_is_bounded() {
        let mut p = Picker::default();
        for i in 0..20 {
            p.remember(&format!("mic{i}"));
        }
        assert_eq!(p.recent.len(), MAX_RECENT);
        assert_eq!(p.recent[0], "mic19");
    }
}
