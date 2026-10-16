//! Detects the QuadCast S's own mute (tap-to-mute) from its sound.
//!
//! The microphone does not report its mute button to the computer. While
//! muted, it sends only a tiny constant residue (measured peaks of 4-6 out
//! of 32768), far below even a silent room with the gain knob at minimum
//! (13 or more). A full second at or below MUTED_PEAK means "muted on the mic".
//!
//! Audio is analysed in memory and immediately discarded. Nothing is
//! recorded or sent anywhere.
#![allow(non_upper_case_globals, non_snake_case)]

use super::cf::CFStringRef;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

#[repr(C)]
struct Asbd {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels: u32,
    bits: u32,
    reserved: u32,
}

#[repr(C)]
struct AqBuffer {
    capacity: u32,
    data: *mut c_void,
    byte_size: u32,
    user: *mut c_void,
    pd_capacity: u32,
    pds: *mut c_void,
    pd_count: u32,
}

type Aq = *mut c_void;
type InputCallback = extern "C" fn(*mut c_void, Aq, *mut AqBuffer, *const c_void, u32, *const c_void);

#[link(name = "AudioToolbox", kind = "framework")]
extern "C" {
    fn AudioQueueNewInput(
        fmt: *const Asbd,
        cb: InputCallback,
        user: *mut c_void,
        run_loop: *const c_void,
        mode: *const c_void,
        flags: u32,
        out: *mut Aq,
    ) -> i32;
    fn AudioQueueAllocateBuffer(q: Aq, size: u32, out: *mut *mut AqBuffer) -> i32;
    fn AudioQueueEnqueueBuffer(q: Aq, b: *mut AqBuffer, n: u32, pd: *const c_void) -> i32;
    fn AudioQueueSetProperty(q: Aq, id: u32, data: *const c_void, size: u32) -> i32;
    fn AudioQueueStart(q: Aq, t: *const c_void) -> i32;
    fn AudioQueueStop(q: Aq, immediate: u8) -> i32;
    fn AudioQueueDispose(q: Aq, immediate: u8) -> i32;
}

const RATE: f64 = 48_000.0;
/// While muted on the mic, the QuadCast S still sends a tiny constant
/// residue: measured peaks of 4-6 (out of 32768). A live capsule in a
/// quiet room measured 49 or more, and 13 or more with the gain knob at
/// its minimum. Anything at or below this for a full
/// second counts as "muted on the mic".
const MUTED_PEAK: u32 = 8;
const SILENT_SAMPLES: u64 = 48_000;
const kAudioFormatLinearPCM: u32 = u32::from_be_bytes(*b"lpcm");
const kAudioQueueProperty_CurrentDevice: u32 = u32::from_be_bytes(*b"aqcd");

static RUNNING: AtomicBool = AtomicBool::new(false);
static ZERO_RUN: AtomicU64 = AtomicU64::new(0);
static BUFFERS_SEEN: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU32 = AtomicU32::new(0);
static QUEUE: AtomicUsize = AtomicUsize::new(0);

extern "C" fn on_buffer(_u: *mut c_void, q: Aq, buf: *mut AqBuffer, _t: *const c_void, _n: u32, _pd: *const c_void) {
    let b = unsafe { &*buf };
    let n = (b.byte_size / 2) as usize;
    if n > 0 && !b.data.is_null() {
        let samples = unsafe { std::slice::from_raw_parts(b.data as *const i16, n) };
        let peak = samples.iter().map(|s| s.unsigned_abs() as u32).max().unwrap_or(0);
        if peak <= MUTED_PEAK {
            ZERO_RUN.fetch_add(n as u64, Ordering::Relaxed);
        } else {
            ZERO_RUN.store(0, Ordering::Relaxed);
        }
        PEAK.store(peak, Ordering::Relaxed);
        BUFFERS_SEEN.fetch_add(1, Ordering::Relaxed);
    }
    if RUNNING.load(Ordering::Relaxed) {
        unsafe { AudioQueueEnqueueBuffer(q, buf, 0, std::ptr::null()) };
    }
}

/// Starts listening to the device with the given CoreAudio UID.
pub fn start(device_uid: CFStringRef) -> Result<(), String> {
    if RUNNING.load(Ordering::Relaxed) {
        return Ok(());
    }
    let fmt = Asbd {
        sample_rate: RATE,
        format_id: kAudioFormatLinearPCM,
        format_flags: 4 | 8, // signed integer, packed
        bytes_per_packet: 2,
        frames_per_packet: 1,
        bytes_per_frame: 2,
        channels: 1,
        bits: 16,
        reserved: 0,
    };
    let mut q: Aq = std::ptr::null_mut();
    unsafe {
        let rc = AudioQueueNewInput(&fmt, on_buffer, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), 0, &mut q);
        if rc != 0 {
            return Err(format!("could not open the microphone (OSStatus {})", rc));
        }
        let uid = device_uid;
        AudioQueueSetProperty(q, kAudioQueueProperty_CurrentDevice, &uid as *const CFStringRef as *const c_void, std::mem::size_of::<CFStringRef>() as u32);
        for _ in 0..3 {
            let mut b: *mut AqBuffer = std::ptr::null_mut();
            if AudioQueueAllocateBuffer(q, 9_600, &mut b) == 0 {
                AudioQueueEnqueueBuffer(q, b, 0, std::ptr::null());
            }
        }
        ZERO_RUN.store(0, Ordering::Relaxed);
        BUFFERS_SEEN.store(0, Ordering::Relaxed);
        RUNNING.store(true, Ordering::Relaxed);
        let rc = AudioQueueStart(q, std::ptr::null());
        if rc != 0 {
            RUNNING.store(false, Ordering::Relaxed);
            AudioQueueDispose(q, 1);
            return Err(format!("could not start listening (OSStatus {})", rc));
        }
    }
    QUEUE.store(q as usize, Ordering::Relaxed);
    Ok(())
}

pub fn stop() {
    RUNNING.store(false, Ordering::Relaxed);
    let q = QUEUE.swap(0, Ordering::Relaxed) as Aq;
    if !q.is_null() {
        unsafe {
            AudioQueueStop(q, 1);
            AudioQueueDispose(q, 1);
        }
    }
}

pub fn running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

/// Some(true) when the mic is sending pure silence (muted on the mic),
/// Some(false) when live, None until enough audio has been seen.
pub fn muted_on_mic() -> Option<bool> {
    if !running() || BUFFERS_SEEN.load(Ordering::Relaxed) < 5 {
        return None;
    }
    Some(ZERO_RUN.load(Ordering::Relaxed) >= SILENT_SAMPLES)
}

/// One-line state summary, for diagnostics.
pub fn debug_line() -> String {
    format!(
        "running={} buffers={} zero_run={} peak={} muted_on_mic={:?}",
        running(),
        BUFFERS_SEEN.load(Ordering::Relaxed),
        ZERO_RUN.load(Ordering::Relaxed),
        PEAK.load(Ordering::Relaxed),
        muted_on_mic()
    )
}

/// Loudest sample in the last buffer (0-32768), for diagnostics.
pub fn last_peak() -> u32 {
    PEAK.load(Ordering::Relaxed)
}
