//! Audio controls through CoreAudio: mic level and mute, headphone level and
//! mute, and hardware monitoring (play-through) where the device exposes it.
#![allow(non_upper_case_globals, non_snake_case)]

use super::cf::{to_string, CFRelease, CFStringRef};
use std::ffi::c_void;

type AudioObjectID = u32;
type OSStatus = i32;

#[repr(C)]
struct Addr {
    selector: u32,
    scope: u32,
    element: u32,
}

const fn cc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | s[3] as u32
}

const kAudioObjectSystemObject: AudioObjectID = 1;
const kHardwareDevices: u32 = cc(b"dev#");
const kObjectName: u32 = cc(b"lnam");
const kDeviceStreams: u32 = cc(b"stm#");
const kVolumeScalar: u32 = cc(b"volm");
const kMute: u32 = cc(b"mute");
const kPlayThru: u32 = cc(b"thru");
const kPlayThruVolume: u32 = cc(b"mvsc");

const SCOPE_GLOBAL: u32 = cc(b"glob");
const SCOPE_INPUT: u32 = cc(b"inpt");
const SCOPE_OUTPUT: u32 = cc(b"outp");

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectHasProperty(id: AudioObjectID, a: *const Addr) -> u8;
    fn AudioObjectIsPropertySettable(id: AudioObjectID, a: *const Addr, out: *mut u8) -> OSStatus;
    fn AudioObjectGetPropertyDataSize(id: AudioObjectID, a: *const Addr, qs: u32, q: *const c_void, out: *mut u32) -> OSStatus;
    fn AudioObjectGetPropertyData(
        id: AudioObjectID,
        a: *const Addr,
        qs: u32,
        q: *const c_void,
        io_size: *mut u32,
        out: *mut c_void,
    ) -> OSStatus;
    fn AudioObjectSetPropertyData(
        id: AudioObjectID,
        a: *const Addr,
        qs: u32,
        q: *const c_void,
        size: u32,
        data: *const c_void,
    ) -> OSStatus;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The microphone capsule (input scope).
    Mic,
    /// The headphone jack on the microphone (output scope).
    Headphones,
}

impl Side {
    fn scope(self) -> u32 {
        match self {
            Side::Mic => SCOPE_INPUT,
            Side::Headphones => SCOPE_OUTPUT,
        }
    }
}

pub struct AudioDevice {
    id: AudioObjectID,
    pub name: String,
}

fn has(id: AudioObjectID, sel: u32, scope: u32, el: u32) -> bool {
    let a = Addr { selector: sel, scope, element: el };
    unsafe { AudioObjectHasProperty(id, &a) != 0 }
}

fn settable(id: AudioObjectID, sel: u32, scope: u32, el: u32) -> bool {
    let a = Addr { selector: sel, scope, element: el };
    let mut s: u8 = 0;
    unsafe { AudioObjectIsPropertySettable(id, &a, &mut s) == 0 && s != 0 }
}

fn get<T: Copy + Default>(id: AudioObjectID, sel: u32, scope: u32, el: u32) -> Option<T> {
    let a = Addr { selector: sel, scope, element: el };
    let mut v = T::default();
    let mut size = std::mem::size_of::<T>() as u32;
    let rc = unsafe { AudioObjectGetPropertyData(id, &a, 0, std::ptr::null(), &mut size, &mut v as *mut T as *mut c_void) };
    (rc == 0).then_some(v)
}

fn set<T: Copy>(id: AudioObjectID, sel: u32, scope: u32, el: u32, v: T) -> Result<(), String> {
    let a = Addr { selector: sel, scope, element: el };
    let rc = unsafe {
        AudioObjectSetPropertyData(id, &a, 0, std::ptr::null(), std::mem::size_of::<T>() as u32, &v as *const T as *const c_void)
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(format!("CoreAudio refused the change (OSStatus {})", rc))
    }
}

fn device_ids() -> Vec<AudioObjectID> {
    let a = Addr { selector: kHardwareDevices, scope: SCOPE_GLOBAL, element: 0 };
    let mut size: u32 = 0;
    unsafe {
        if AudioObjectGetPropertyDataSize(kAudioObjectSystemObject, &a, 0, std::ptr::null(), &mut size) != 0 {
            return vec![];
        }
        let mut ids = vec![0u32; size as usize / 4];
        if AudioObjectGetPropertyData(kAudioObjectSystemObject, &a, 0, std::ptr::null(), &mut size, ids.as_mut_ptr() as *mut c_void) != 0 {
            return vec![];
        }
        ids.truncate(size as usize / 4);
        ids
    }
}

fn device_name(id: AudioObjectID) -> Option<String> {
    let s: CFStringRef = get::<usize>(id, kObjectName, SCOPE_GLOBAL, 0)? as CFStringRef;
    let out = to_string(s);
    if !s.is_null() {
        unsafe { CFRelease(s) }
    }
    out
}

/// The elements (channel numbers) that carry `sel` in `scope`: the main
/// element 0 if present, otherwise the individual channels.
fn elements(id: AudioObjectID, sel: u32, scope: u32) -> Vec<u32> {
    if has(id, sel, scope, 0) {
        return vec![0];
    }
    (1..=8).filter(|&e| has(id, sel, scope, e)).collect()
}

impl AudioDevice {
    pub fn find() -> Result<AudioDevice, String> {
        for id in device_ids() {
            if let Some(name) = device_name(id) {
                if name.to_ascii_lowercase().contains("quadcast") {
                    return Ok(AudioDevice { id, name });
                }
            }
        }
        Err("QuadCast audio device not found in macOS sound devices".into())
    }

    /// Level 0-100, or None if macOS exposes no level control.
    pub fn volume(&self, side: Side) -> Option<u8> {
        let el = *elements(self.id, kVolumeScalar, side.scope()).first()?;
        let v: f32 = get(self.id, kVolumeScalar, side.scope(), el)?;
        Some((v * 100.0).round().clamp(0.0, 100.0) as u8)
    }

    pub fn set_volume(&self, side: Side, percent: u8) -> Result<(), String> {
        let els = elements(self.id, kVolumeScalar, side.scope());
        if els.is_empty() {
            return Err(format!("{:?} level is not adjustable on this device under macOS", side));
        }
        let v = percent.min(100) as f32 / 100.0;
        for e in els {
            set(self.id, kVolumeScalar, side.scope(), e, v)?;
        }
        Ok(())
    }

    pub fn muted(&self, side: Side) -> Option<bool> {
        let el = *elements(self.id, kMute, side.scope()).first()?;
        get::<u32>(self.id, kMute, side.scope(), el).map(|v| v != 0)
    }

    pub fn set_muted(&self, side: Side, on: bool) -> Result<(), String> {
        let els = elements(self.id, kMute, side.scope());
        if els.is_empty() {
            return Err(format!("{:?} mute is not exposed by this device under macOS", side));
        }
        for e in els {
            set(self.id, kMute, side.scope(), e, on as u32)?;
        }
        Ok(())
    }

    /// Hardware monitoring (mic into headphones): on/off.
    pub fn monitoring(&self) -> Option<bool> {
        let el = *elements(self.id, kPlayThru, SCOPE_INPUT).first()?;
        get::<u32>(self.id, kPlayThru, SCOPE_INPUT, el).map(|v| v != 0)
    }

    pub fn set_monitoring(&self, on: bool) -> Result<(), String> {
        let els = elements(self.id, kPlayThru, SCOPE_INPUT);
        if els.is_empty() {
            return Err("this device does not expose hardware monitoring to macOS".into());
        }
        for e in els {
            set(self.id, kPlayThru, SCOPE_INPUT, e, on as u32)?;
        }
        Ok(())
    }

    pub fn monitoring_level(&self) -> Option<u8> {
        let el = *elements(self.id, kPlayThruVolume, SCOPE_INPUT).first()?;
        let v: f32 = get(self.id, kPlayThruVolume, SCOPE_INPUT, el)?;
        Some((v * 100.0).round().clamp(0.0, 100.0) as u8)
    }

    pub fn set_monitoring_level(&self, percent: u8) -> Result<(), String> {
        let els = elements(self.id, kPlayThruVolume, SCOPE_INPUT);
        if els.is_empty() {
            return Err("this device does not expose a monitoring level to macOS".into());
        }
        for e in els {
            set(self.id, kPlayThruVolume, SCOPE_INPUT, e, percent.min(100) as f32 / 100.0)?;
        }
        Ok(())
    }

    /// Lists every control macOS exposes for this device (diagnostics).
    pub fn describe_controls(&self) -> Vec<String> {
        let mut out = vec![];
        let sels = [("level", kVolumeScalar), ("mute", kMute), ("monitor", kPlayThru), ("monitor level", kPlayThruVolume)];
        let scopes = [("global", SCOPE_GLOBAL), ("mic", SCOPE_INPUT), ("headphones", SCOPE_OUTPUT)];
        for (sn, sc) in scopes {
            let streams = has(self.id, kDeviceStreams, sc, 0);
            for (name, sel) in sels {
                for e in 0..=8 {
                    if has(self.id, sel, sc, e) {
                        let w = if settable(self.id, sel, sc, e) { "settable" } else { "read-only" };
                        out.push(format!("{:<10} {:<13} channel {}  {}  (streams: {})", sn, name, e, w, streams));
                    }
                }
            }
        }
        out
    }
}
