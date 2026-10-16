//! HyperX QuadCast S control for macOS.
//!
//! * `protocol` and `effects` are platform-independent and unit-tested.
//! * `mac` talks to the hardware through IOKit (lighting) and CoreAudio
//!   (mic, headphone and monitoring levels).

pub mod effects;
pub mod protocol;

#[cfg(target_os = "macos")]
pub mod mac;
