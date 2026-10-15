//! third-party plugin hosting.

pub mod clap;
pub mod scan;
#[cfg(target_os = "linux")]
mod x11;
