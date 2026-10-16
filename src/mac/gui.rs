//! Native macOS window (AppKit through the Objective-C runtime, no extra crates).
#![allow(non_snake_case)]

use super::audio::{AudioDevice, Side};
use super::hid::Light;
use super::listen;
use crate::effects::{frames, Effect, EffectSpec, Speed};
use crate::protocol::{self, Frame, Rgb};
use std::ffi::{c_char, c_void, CString};
use std::sync::OnceLock;

type Id = *mut c_void;
type Sel = *const c_void;
const NIL: Id = std::ptr::null_mut();

#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
    fn objc_allocateClassPair(sup: Id, name: *const c_char, extra: usize) -> Id;
    fn objc_registerClassPair(cls: Id);
    fn class_addMethod(cls: Id, name: Sel, imp: *const c_void, types: *const c_char) -> i8;
}

#[link(name = "AppKit", kind = "framework")]
extern "C" {
    fn NSBeep();
    fn NSRectFillUsingOperation(rect: Rect, op: usize);
}



fn cls(n: &str) -> Id {
    let c = CString::new(n).unwrap();
    unsafe { objc_getClass(c.as_ptr()) }
}

fn sel(n: &str) -> Sel {
    let c = CString::new(n).unwrap();
    unsafe { sel_registerName(c.as_ptr()) }
}

macro_rules! msg {
    ($ret:ty; $obj:expr, $sel:expr $(, $a:expr => $t:ty)* ) => {{
        let f: unsafe extern "C" fn(Id, Sel $(, $t)*) -> $ret =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        unsafe { f($obj, sel($sel) $(, $a)*) }
    }};
}

fn nsstr(s: &str) -> Id {
    let c = CString::new(s.replace('\0', "")).unwrap();
    msg![Id; cls("NSString"), "stringWithUTF8String:", c.as_ptr() => *const c_char]
}

// ---- controls -------------------------------------------------------------

const H: f64 = 632.0;
const W: f64 = 480.0;

fn r(x: f64, top: f64, w: f64, h: f64) -> Rect {
    Rect { x, y: H - top - h, w, h }
}

const T_APPLY: isize = 1;
const T_MIC: isize = 10;
const T_MIC_MUTE: isize = 11;
const T_HP: isize = 20;
const T_HP_MUTE: isize = 21;
const T_MON: isize = 30;
const T_MON_LEVEL: isize = 31;
const T_SYNC: isize = 40;
const T_DETECT: isize = 41;
const T_TINT: isize = 42;

struct Ui {
    effect: usize,
    top: usize,
    bottom: usize,
    speed: usize,
    bright: usize,
    mic: usize,
    mic_mute: usize,
    hp: usize,
    hp_mute: usize,
    mon: usize,
    mon_level: usize,
    status: usize,
    win: usize,
    mute_item: usize,
    login_item: usize,
    sync_box: usize,
    detect_box: usize,
    tint_box: usize,
    bar_button: usize,
}

static UI: OnceLock<Ui> = OnceLock::new();

const EFFECTS: [&str; 7] = ["Solid", "Blink", "Breathing", "Cycle (rainbow)", "Wave", "Lightning", "Off"];

fn add(view: Id, v: Id, frame: Rect) -> Id {
    msg![(); v, "setFrame:", frame => Rect];
    msg![(); view, "addSubview:", v => Id];
    v
}

fn label(view: Id, text: &str, frame: Rect, bold: bool) -> Id {
    let l = msg![Id; cls("NSTextField"), "wrappingLabelWithString:", nsstr(text) => Id];
    if bold {
        let f = msg![Id; cls("NSFont"), "boldSystemFontOfSize:", 14.0 => f64];
        msg![(); l, "setFont:", f => Id];
    }
    add(view, l, frame)
}

fn wire(v: Id, ctrl: Id, tag: isize) {
    msg![(); v, "setTarget:", ctrl => Id];
    msg![(); v, "setAction:", sel("onAction:") => Sel];
    msg![(); v, "setTag:", tag => isize];
}

fn popup(view: Id, items: &[&str], frame: Rect, selected: isize) -> Id {
    let p = msg![Id; cls("NSPopUpButton"), "alloc"];
    let p = msg![Id; p, "initWithFrame:pullsDown:", frame => Rect, 0i8 => i8];
    for it in items {
        msg![(); p, "addItemWithTitle:", nsstr(it) => Id];
    }
    msg![(); p, "selectItemAtIndex:", selected => isize];
    add(view, p, frame)
}

fn slider(view: Id, frame: Rect, value: f64, ctrl: Id, tag: isize) -> Id {
    let s = msg![Id; cls("NSSlider"), "alloc"];
    let s = msg![Id; s, "initWithFrame:", frame => Rect];
    msg![(); s, "setMinValue:", 0.0 => f64];
    msg![(); s, "setMaxValue:", 100.0 => f64];
    msg![(); s, "setDoubleValue:", value => f64];
    if ctrl != NIL {
        wire(s, ctrl, tag);
    }
    add(view, s, frame)
}

fn checkbox(view: Id, title: &str, frame: Rect, ctrl: Id, tag: isize) -> Id {
    let b = msg![Id; cls("NSButton"), "checkboxWithTitle:target:action:", nsstr(title) => Id, ctrl => Id, sel("onAction:") => Sel];
    msg![(); b, "setTag:", tag => isize];
    add(view, b, frame)
}

fn colour_well(view: Id, frame: Rect, c: Rgb) -> Id {
    let w = msg![Id; cls("NSColorWell"), "alloc"];
    let w = msg![Id; w, "initWithFrame:", frame => Rect];
    let col = msg![Id; cls("NSColor"), "colorWithSRGBRed:green:blue:alpha:",
        c.0 as f64 / 255.0 => f64, c.1 as f64 / 255.0 => f64, c.2 as f64 / 255.0 => f64, 1.0 => f64];
    msg![(); w, "setColor:", col => Id];
    add(view, w, frame)
}

fn well_colour(w: Id) -> Rgb {
    let col = msg![Id; w, "color"];
    let space = msg![Id; cls("NSColorSpace"), "sRGBColorSpace"];
    let c = msg![Id; col, "colorUsingColorSpace:", space => Id];
    if c == NIL {
        return Rgb(255, 255, 255);
    }
    let ch = |name: &str| (msg![f64; c, name].clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgb(ch("redComponent"), ch("greenComponent"), ch("blueComponent"))
}

fn set_enabled(v: Id, on: bool) {
    msg![(); v, "setEnabled:", on as i8 => i8];
}

fn set_status(text: &str) {
    if let Some(ui) = UI.get() {
        msg![(); ui.status as Id, "setStringValue:", nsstr(text) => Id];
    }
}

fn idx(v: usize) -> isize {
    msg![isize; v as Id, "indexOfSelectedItem"]
}

fn value(v: usize) -> u8 {
    msg![f64; v as Id, "doubleValue"].round().clamp(0.0, 100.0) as u8
}

fn checked(v: usize) -> bool {
    msg![isize; v as Id, "state"] != 0
}

// ---- preferences -----------------------------------------------------------

const PREF_SYNC: &str = "keepInSync";
const PREF_DETECT: &str = "detectMicMuteButton";
const PREF_TINT: &str = "colourMenuBarIcon";
const PREF_TOP: &str = "topColour";
const PREF_BOTTOM: &str = "bottomColour";

fn pref_str(key: &str) -> Option<String> {
    let v = msg![Id; defaults(), "stringForKey:", nsstr(key) => Id];
    if v == NIL {
        return None;
    }
    let p = msg![*const c_char; v, "UTF8String"];
    if p.is_null() {
        return None;
    }
    Some(unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

fn set_pref_str(key: &str, v: &str) {
    msg![(); defaults(), "setObject:forKey:", nsstr(v) => Id, nsstr(key) => Id];
}

fn hex_of(c: Rgb) -> String {
    format!("{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

fn nscolor(c: Rgb) -> Id {
    msg![Id; cls("NSColor"), "colorWithSRGBRed:green:blue:alpha:",
        c.0 as f64 / 255.0 => f64, c.1 as f64 / 255.0 => f64, c.2 as f64 / 255.0 => f64, 1.0 => f64]
}

fn defaults() -> Id {
    msg![Id; cls("NSUserDefaults"), "standardUserDefaults"]
}

fn pref(key: &str, default: bool) -> bool {
    let d = defaults();
    if msg![Id; d, "objectForKey:", nsstr(key) => Id] == NIL {
        return default;
    }
    msg![i8; d, "boolForKey:", nsstr(key) => Id] != 0
}

fn set_pref(key: &str, v: bool) {
    msg![(); defaults(), "setBool:forKey:", v as i8 => i8, nsstr(key) => Id];
}

/// 3 = allowed, 2 = denied, 1 = restricted, 0 = not asked yet, -1 = unknown.
fn mic_permission() -> isize {
    let path = nsstr("/System/Library/Frameworks/AVFoundation.framework");
    let bundle = msg![Id; cls("NSBundle"), "bundleWithPath:", path => Id];
    if bundle != NIL {
        msg![i8; bundle, "load"];
    }
    let c = cls("AVCaptureDevice");
    if c == NIL {
        return -1;
    }
    msg![isize; c, "authorizationStatusForMediaType:", nsstr("soun") => Id]
}

fn start_detection() -> Result<String, String> {
    if mic_permission() == 2 || mic_permission() == 1 {
        return Err("CastGlow isn't allowed to use the microphone. Turn it on in System Settings > Privacy & Security > Microphone, then tick this again.".into());
    }
    let a = AudioDevice::find()?;
    let uid = a.uid().ok_or("could not identify the QuadCast audio device")?;
    let r = listen::start(uid);
    unsafe { super::cf::CFRelease(uid) };
    r.map(|_| "Watching for the mic's own mute button. Tap the top of the mic to try it.".to_string())
}

/// Messages from background work, shown by the next tick on the main thread.
static PENDING_STATUS: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static DETECT_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Starts mute detection on a background thread. Opening the microphone
/// can wait for the user to answer macOS's permission question, and the
/// window must not freeze while that happens.
fn start_detection_in_background() {
    std::thread::spawn(|| {
        let msg = match start_detection() {
            Ok(m) => m,
            Err(e) => {
                DETECT_FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
                format!("Problem: {}", e)
            }
        };
        if let Ok(mut p) = PENDING_STATUS.lock() {
            *p = Some(msg);
        }
    });
}

static LAST_MUTED: std::sync::atomic::AtomicI8 = std::sync::atomic::AtomicI8::new(-1);
static LAST_HW: std::sync::atomic::AtomicI8 = std::sync::atomic::AtomicI8::new(-1);

#[repr(C)]
#[derive(Clone, Copy)]
struct Size {
    w: f64,
    h: f64,
}

/// Returns a copy of a template symbol image painted in `colour`.
/// The macOS menu bar ignores tint colours on status items and draws
/// template images in black or white, so coloured icons have to be
/// real, non-template images.
fn painted(img: Id, colour: Id) -> Id {
    let size = msg![Size; img, "size"];
    let out = msg![Id; msg![Id; cls("NSImage"), "alloc"], "initWithSize:", size => Size];
    let rect = Rect { x: 0.0, y: 0.0, w: size.w, h: size.h };
    let zero = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
    msg![(); out, "lockFocus"];
    msg![(); img, "drawInRect:fromRect:operation:fraction:", rect => Rect, zero => Rect, 2usize => usize, 1.0 => f64];
    msg![(); colour, "set"];
    unsafe { NSRectFillUsingOperation(rect, 5) }; // source-atop: colour only where the symbol is
    msg![(); out, "unlockFocus"];
    msg![(); out, "setTemplate:", 0i8 => i8];
    out
}

/// Menu bar icon. `state`: Some(false) = live, Some(true) = muted,
/// None = unknown (mic not found). With "Colour the menu bar icon" on,
/// live uses the mic's top colour and muted its bottom colour; otherwise,
/// or when the state is unknown, the normal system colour is used.
fn set_bar_icon(state: Option<bool>) {
    let Some(ui) = UI.get() else { return };
    if ui.bar_button == 0 {
        return;
    }
    let muted = state == Some(true);
    let name = if muted { "mic.slash.fill" } else { "mic.fill" };
    let img = msg![Id; cls("NSImage"), "imageWithSystemSymbolName:accessibilityDescription:", nsstr(name) => Id, nsstr("CastGlow") => Id];
    if img != NIL {
        msg![(); img, "setTemplate:", 1i8 => i8];
        msg![(); ui.bar_button as Id, "setImage:", img => Id];
    }
    let tint = match state {
        Some(m) if checked(ui.tint_box) => {
            let c = well_colour(if m { ui.bottom } else { ui.top } as Id);
            // Dark colours vanish on a dark menu bar: keep the shade but lift
            // its brightest channel to at least 200. Near-black has no shade
            // to keep, so it uses the system colour.
            let max = c.0.max(c.1).max(c.2) as u32;
            if max < 24 {
                NIL
            } else if max < 200 {
                let k = |v: u8| ((v as u32 * 200 + max / 2) / max).min(255) as u8;
                nscolor(Rgb(k(c.0), k(c.1), k(c.2)))
            } else {
                nscolor(c)
            }
        }
        _ => NIL,
    };
    if img != NIL {
        if tint != NIL {
            msg![(); ui.bar_button as Id, "setImage:", painted(img, tint) => Id];
        } else {
            msg![(); ui.bar_button as Id, "setImage:", img => Id];
        }
    }
    msg![(); ui.bar_button as Id, "setContentTintColor:", NIL => Id];
    if std::path::Path::new("/tmp/castglow-debug").exists() {
        use std::io::Write;
        let applied = tint;
        let desc = if applied == NIL { "system default".to_string() } else {
            let d = msg![Id; applied, "description"];
            let p = msg![*const c_char; d, "UTF8String"];
            unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned()
        };
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/castglow-icon.log") {
            let w = msg![Id; ui.bar_button as Id, "window"];
            let fr = if w != NIL { msg![Rect; w, "frame"] } else { Rect { x: -1.0, y: -1.0, w: 0.0, h: 0.0 } };
            let shown = msg![Id; ui.bar_button as Id, "image"];
            let tmpl = if shown != NIL { msg![i8; shown, "isTemplate"] } else { -1 };
            let _ = writeln!(f, "state={:?} icon={} tint={} frame=({},{} {}x{}) template={}", state, name, desc, fr.x, fr.y, fr.w, fr.h, tmpl);
        }
    }
    let tip = match state {
        Some(true) => "CastGlow: mic muted",
        Some(false) => "CastGlow: mic live",
        None => "CastGlow: mic not found",
    };
    msg![(); ui.bar_button as Id, "setToolTip:", nsstr(tip) => Id];
}

/// Runs every second: brings the window and menu bar icon up to date.
extern "C" fn on_tick(_this: Id, _cmd: Sel, _timer: Id) {
    let Some(ui) = UI.get() else { return };
    if let Some(m) = PENDING_STATUS.lock().ok().and_then(|mut p| p.take()) {
        set_status(&m);
    }
    if DETECT_FAILED.swap(false, std::sync::atomic::Ordering::Relaxed) {
        msg![(); ui.detect_box as Id, "setState:", 0isize => isize];
        set_pref(PREF_DETECT, false);
    }
    let sync = checked(ui.sync_box);
    let hw = if checked(ui.detect_box) { listen::muted_on_mic() } else { None };
    if std::env::var_os("CASTGLOW_DEBUG").is_some() || std::path::Path::new("/tmp/castglow-debug").exists() {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/castglow-listen.log") {
            let _ = writeln!(f, "{:?} perm={} {}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0), mic_permission(), listen::debug_line());
        }
    }
    let audio = if sync || hw.is_some() { AudioDevice::find().ok() } else { None };
    let soft = audio.as_ref().and_then(|a| a.muted(Side::Mic)).unwrap_or(false);

    if sync {
        if let Some(a) = &audio {
            // Don't fight the user while they drag a slider.
            let dragging = msg![usize; cls("NSEvent"), "pressedMouseButtons"] != 0;
            if !dragging {
                for (v, s) in [(a.volume(Side::Mic), ui.mic), (a.volume(Side::Headphones), ui.hp), (a.monitoring_level(), ui.mon_level)] {
                    if let Some(x) = v {
                        if value(s) != x {
                            msg![(); s as Id, "setDoubleValue:", x as f64 => f64];
                        }
                    }
                }
            }
            if let Some(m) = a.muted(Side::Headphones) {
                msg![(); ui.hp_mute as Id, "setState:", m as isize => isize];
            }
            if let Some(m) = a.monitoring() {
                msg![(); ui.mon as Id, "setState:", m as isize => isize];
            }
        }
    }

    let muted = soft || hw == Some(true);
    if sync || hw.is_some() {
        msg![(); ui.mic_mute as Id, "setState:", muted as isize => isize];
    }
    let _ = LAST_MUTED.swap(muted as i8, std::sync::atomic::Ordering::Relaxed);
    let known = audio.is_some() || hw.is_some() || AudioDevice::find().is_ok();
    set_bar_icon(if known { Some(muted) } else { None });
    if let Some(h) = hw {
        let prev = LAST_HW.swap(h as i8, std::sync::atomic::Ordering::Relaxed);
        if prev != -1 && prev != h as i8 {
            set_status(if h { "Muted with the button on the mic. Tap the top of the mic to unmute." } else { "Mic unmuted." });
        }
    } else {
        LAST_HW.store(-1, std::sync::atomic::Ordering::Relaxed);
    }
}

// ---- actions ----------------------------------------------------------------

fn apply_lighting(ui: &Ui) -> Result<String, String> {
    let i = idx(ui.effect) as usize;
    let speed = [Speed::Slow, Speed::Normal, Speed::Fast][idx(ui.speed).clamp(0, 2) as usize];
    let brightness = value(ui.bright);
    let (top, bottom) = (well_colour(ui.top as Id), well_colour(ui.bottom as Id));
    let fr: Vec<Frame> = if i >= Effect::ALL.len() {
        vec![Frame::both(Rgb::OFF)]
    } else {
        frames(&EffectSpec { effect: Effect::ALL[i], colour: top, colour2: Some(bottom), speed, brightness })
    };
    let light = Light::open()?;
    light.send_all(&protocol::save_reports(&fr)?)?;
    set_pref_str(PREF_TOP, &hex_of(top));
    set_pref_str(PREF_BOTTOM, &hex_of(bottom));
    Ok(format!("Lighting set: {} - saved on the mic.", EFFECTS[i.min(EFFECTS.len() - 1)]))
}

fn handle(tag: isize) {
    let Some(ui) = UI.get() else { return };
    let res: Result<String, String> = match tag {
        T_APPLY => apply_lighting(ui),
        T_SYNC => {
            set_pref(PREF_SYNC, checked(ui.sync_box));
            Ok((if checked(ui.sync_box) { "Keeping in sync with the mic and macOS." } else { "Sync off: the window shows only changes made here." }).to_string())
        }
        T_TINT => {
            set_pref(PREF_TINT, checked(ui.tint_box));
            Ok((if checked(ui.tint_box) { "Menu bar icon: top colour when live, bottom colour when muted." } else { "Menu bar icon uses the normal system colour." }).to_string())
        }
        T_DETECT => {
            let on = checked(ui.detect_box);
            set_pref(PREF_DETECT, on);
            if on {
                start_detection_in_background();
                Ok("Starting… if macOS asks to use the microphone, click Allow.".to_string())
            } else {
                listen::stop();
                Ok("Stopped watching for the mic's mute button.".to_string())
            }
        }
        T_MIC | T_HP | T_MIC_MUTE | T_HP_MUTE | T_MON | T_MON_LEVEL => AudioDevice::find().and_then(|a| match tag {
            T_MIC => a.set_volume(Side::Mic, value(ui.mic)).map(|_| format!("Mic level {}%", value(ui.mic))),
            T_HP => a.set_volume(Side::Headphones, value(ui.hp)).map(|_| format!("Headphone level {}%", value(ui.hp))),
            T_MIC_MUTE => a.set_muted(Side::Mic, checked(ui.mic_mute)).map(|_| (if checked(ui.mic_mute) { "Mic muted" } else { "Mic unmuted" }).into()),
            T_HP_MUTE => a.set_muted(Side::Headphones, checked(ui.hp_mute)).map(|_| (if checked(ui.hp_mute) { "Headphones muted" } else { "Headphones unmuted" }).into()),
            T_MON => a.set_monitoring(checked(ui.mon)).map(|_| (if checked(ui.mon) { "Monitoring on" } else { "Monitoring off" }).into()),
            _ => a.set_monitoring_level(value(ui.mon_level)).map(|_| format!("Monitoring level {}%", value(ui.mon_level))),
        }),
        _ => return,
    };
    match res {
        Ok(s) => set_status(&s),
        Err(e) => set_status(&format!("Problem: {}", e)),
    }
}

extern "C" fn on_action(_this: Id, _cmd: Sel, sender: Id) {
    let tag = msg![isize; sender, "tag"];
    handle(tag);
}

// Keep running in the menu bar when the window is closed.
extern "C" fn quit_after_last_window(_this: Id, _cmd: Sel, _app: Id) -> i8 {
    0
}

fn show_window() {
    if let Some(ui) = UI.get() {
        let app = msg![Id; cls("NSApplication"), "sharedApplication"];
        msg![(); app, "activateIgnoringOtherApps:", 1i8 => i8];
        msg![(); ui.win as Id, "makeKeyAndOrderFront:", NIL => Id];
    }
}

// Clicking the Dock icon brings the window back.
extern "C" fn on_reopen(_this: Id, _cmd: Sel, _app: Id, _visible: i8) -> i8 {
    show_window();
    1
}

// ---- menu bar item ------------------------------------------------------------

const M_SHOW: isize = 200;
const M_MUTE: isize = 201;
const M_OFF: isize = 202;
const M_RAINBOW: isize = 203;
const M_APPLY: isize = 204;
const M_LOGIN: isize = 205;

fn login_service() -> Id {
    let path = nsstr("/System/Library/Frameworks/ServiceManagement.framework");
    let bundle = msg![Id; cls("NSBundle"), "bundleWithPath:", path => Id];
    if bundle != NIL {
        msg![i8; bundle, "load"];
    }
    let c = cls("SMAppService");
    if c == NIL {
        return NIL;
    }
    msg![Id; c, "mainAppService"]
}

fn login_enabled() -> bool {
    let s = login_service();
    s != NIL && msg![isize; s, "status"] == 1
}

fn refresh_menu() {
    let Some(ui) = UI.get() else { return };
    let muted = AudioDevice::find().ok().and_then(|a| a.muted(Side::Mic)).unwrap_or(false);
    msg![(); ui.mute_item as Id, "setState:", muted as isize => isize];
    msg![(); ui.login_item as Id, "setState:", login_enabled() as isize => isize];
}

extern "C" fn menu_will_open(_this: Id, _cmd: Sel, _menu: Id) {
    refresh_menu();
}

fn store_frames(fr: &[Frame], label: &str) -> Result<String, String> {
    let light = Light::open()?;
    light.send_all(&protocol::save_reports(fr)?)?;
    Ok(format!("Lighting set: {} - saved on the mic.", label))
}

extern "C" fn on_menu(_this: Id, _cmd: Sel, sender: Id) {
    let tag = msg![isize; sender, "tag"];
    let Some(ui) = UI.get() else { return };
    let res: Result<String, String> = match tag {
        M_SHOW => {
            show_window();
            return;
        }
        M_MUTE => AudioDevice::find().and_then(|a| {
            let now = !a.muted(Side::Mic).unwrap_or(false);
            a.set_muted(Side::Mic, now)?;
            msg![(); ui.mic_mute as Id, "setState:", now as isize => isize];
            Ok((if now { "Mic muted" } else { "Mic unmuted" }).to_string())
        }),
        M_OFF => store_frames(&[Frame::both(Rgb::OFF)], "Off"),
        M_RAINBOW => store_frames(
            &frames(&EffectSpec { effect: Effect::Cycle, colour: Rgb(255, 0, 0), colour2: None, speed: Speed::Normal, brightness: value(ui.bright) }),
            "Cycle (rainbow)",
        ),
        M_APPLY => apply_lighting(ui),
        M_LOGIN => {
            let s = login_service();
            if s == NIL {
                Err("start at login needs macOS 13 or later".to_string())
            } else {
                let mut err: Id = NIL;
                let on = login_enabled();
                let ok = if on {
                    msg![i8; s, "unregisterAndReturnError:", &mut err as *mut Id => *mut Id]
                } else {
                    msg![i8; s, "registerAndReturnError:", &mut err as *mut Id => *mut Id]
                };
                if ok != 0 {
                    Ok((if on { "Will not start at login" } else { "Will start at login" }).to_string())
                } else {
                    Err("macOS did not allow start at login - check System Settings > General > Login Items".to_string())
                }
            }
        }
        _ => return,
    };
    match res {
        Ok(s) => set_status(&s),
        Err(e) => set_status(&format!("Problem: {}", e)),
    }
    refresh_menu();
}

fn menu_item(menu: Id, title: &str, ctrl: Id, tag: isize, key: &str) -> Id {
    let it = msg![Id; msg![Id; cls("NSMenuItem"), "alloc"], "initWithTitle:action:keyEquivalent:",
        nsstr(title) => Id, sel("onMenu:") => Sel, nsstr(key) => Id];
    msg![(); it, "setTarget:", ctrl => Id];
    msg![(); it, "setTag:", tag => isize];
    msg![(); menu, "addItem:", it => Id];
    it
}

fn separator(menu: Id) {
    let s = msg![Id; cls("NSMenuItem"), "separatorItem"];
    msg![(); menu, "addItem:", s => Id];
}

/// Builds the mic icon in the menu bar. Returns (mute item, login item, button).
fn build_status_item(ctrl: Id) -> (Id, Id, Id) {
    let bar = msg![Id; cls("NSStatusBar"), "systemStatusBar"];
    let item = msg![Id; bar, "statusItemWithLength:", -1.0 => f64];
    msg![Id; item, "retain"];
    let button = msg![Id; item, "button"];
    let img = msg![Id; cls("NSImage"), "imageWithSystemSymbolName:accessibilityDescription:",
        nsstr("mic.fill") => Id, nsstr("QuadCast S") => Id];
    if img != NIL {
        msg![(); img, "setTemplate:", 1i8 => i8];
        msg![(); button, "setImage:", img => Id];
    } else {
        msg![(); button, "setTitle:", nsstr("QC") => Id];
    }
    msg![(); button, "setToolTip:", nsstr("CastGlow") => Id];

    let menu = msg![Id; cls("NSMenu"), "new"];
    msg![(); menu, "setDelegate:", ctrl => Id];
    menu_item(menu, "Open CastGlow…", ctrl, M_SHOW, ",");
    separator(menu);
    let mute = menu_item(menu, "Mute Mic", ctrl, M_MUTE, "");
    separator(menu);
    menu_item(menu, "Apply Current Lighting", ctrl, M_APPLY, "");
    menu_item(menu, "Rainbow Lighting", ctrl, M_RAINBOW, "");
    menu_item(menu, "Lights Off", ctrl, M_OFF, "");
    separator(menu);
    let login = menu_item(menu, "Start at Login", ctrl, M_LOGIN, "");
    separator(menu);
    let quit = msg![Id; msg![Id; cls("NSMenuItem"), "alloc"], "initWithTitle:action:keyEquivalent:",
        nsstr("Quit CastGlow") => Id, sel("terminate:") => Sel, nsstr("q") => Id];
    msg![(); menu, "addItem:", quit => Id];
    msg![(); item, "setMenu:", menu => Id];
    (mute, login, button)
}

// ---- window -----------------------------------------------------------------

pub fn run() {
    // Keep AppKit linked even though classes are looked up by name.
    std::hint::black_box(NSBeep as unsafe extern "C" fn());

    let app = msg![Id; cls("NSApplication"), "sharedApplication"];
    msg![i8; app, "setActivationPolicy:", 0isize => isize];

    let name = CString::new("QCController").unwrap();
    let ctrl_class = unsafe { objc_allocateClassPair(cls("NSObject"), name.as_ptr(), 0) };
    unsafe {
        class_addMethod(ctrl_class, sel("onAction:"), on_action as *const c_void, c"v@:@".as_ptr());
        class_addMethod(
            ctrl_class,
            sel("applicationShouldTerminateAfterLastWindowClosed:"),
            quit_after_last_window as *const c_void,
            c"c@:@".as_ptr(),
        );
        class_addMethod(ctrl_class, sel("onMenu:"), on_menu as *const c_void, c"v@:@".as_ptr());
        class_addMethod(ctrl_class, sel("onTick:"), on_tick as *const c_void, c"v@:@".as_ptr());
        class_addMethod(ctrl_class, sel("menuWillOpen:"), menu_will_open as *const c_void, c"v@:@".as_ptr());
        class_addMethod(
            ctrl_class,
            sel("applicationShouldHandleReopen:hasVisibleWindows:"),
            on_reopen as *const c_void,
            c"c@:@c".as_ptr(),
        );
        objc_registerClassPair(ctrl_class);
    }
    let ctrl = msg![Id; msg![Id; ctrl_class, "alloc"], "init"];
    msg![(); app, "setDelegate:", ctrl => Id];

    // Menu bar with Quit (Cmd+Q).
    let main_menu = msg![Id; cls("NSMenu"), "new"];
    let app_item = msg![Id; cls("NSMenuItem"), "new"];
    msg![(); main_menu, "addItem:", app_item => Id];
    let app_menu = msg![Id; cls("NSMenu"), "new"];
    let quit = msg![Id; msg![Id; cls("NSMenuItem"), "alloc"], "initWithTitle:action:keyEquivalent:",
        nsstr("Quit CastGlow") => Id, sel("terminate:") => Sel, nsstr("q") => Id];
    msg![(); app_menu, "addItem:", quit => Id];
    msg![(); app_item, "setSubmenu:", app_menu => Id];
    msg![(); app, "setMainMenu:", main_menu => Id];

    let win = msg![Id; cls("NSWindow"), "alloc"];
    let win = msg![Id; win, "initWithContentRect:styleMask:backing:defer:",
        Rect { x: 0.0, y: 0.0, w: W, h: H } => Rect, 7usize => usize, 2usize => usize, 0i8 => i8];
    msg![(); win, "setTitle:", nsstr("CastGlow — HyperX QuadCast S") => Id];
    msg![(); win, "setReleasedWhenClosed:", 0i8 => i8];
    let v = msg![Id; win, "contentView"];

    // Lighting
    label(v, "Lighting", r(20.0, 18.0, 300.0, 20.0), true);
    label(v, "Effect", r(20.0, 52.0, 110.0, 20.0), false);
    let effect = popup(v, &EFFECTS, r(140.0, 48.0, 310.0, 26.0), 0);
    label(v, "Top colour", r(20.0, 92.0, 110.0, 20.0), false);
    let top = colour_well(v, r(140.0, 86.0, 70.0, 30.0), pref_str(PREF_TOP).and_then(|h| Rgb::parse(&h)).unwrap_or(Rgb(255, 0, 0)));
    label(v, "Bottom colour", r(240.0, 92.0, 110.0, 20.0), false);
    let bottom = colour_well(v, r(350.0, 86.0, 70.0, 30.0), pref_str(PREF_BOTTOM).and_then(|h| Rgb::parse(&h)).unwrap_or(Rgb(0, 0, 255)));
    label(v, "Speed", r(20.0, 134.0, 110.0, 20.0), false);
    let speed = popup(v, &["Slow", "Normal", "Fast"], r(140.0, 130.0, 310.0, 26.0), 1);
    label(v, "Brightness", r(20.0, 174.0, 110.0, 20.0), false);
    let bright = slider(v, r(140.0, 172.0, 310.0, 24.0), 100.0, NIL, 0);
    let apply = msg![Id; cls("NSButton"), "buttonWithTitle:target:action:", nsstr("Apply lighting") => Id, ctrl => Id, sel("onAction:") => Sel];
    msg![(); apply, "setTag:", T_APPLY => isize];
    msg![(); apply, "setKeyEquivalent:", nsstr("\r") => Id];
    add(v, apply, r(140.0, 210.0, 310.0, 32.0));

    // Audio
    label(v, "Audio", r(20.0, 270.0, 300.0, 20.0), true);
    label(v, "Mic level", r(20.0, 304.0, 110.0, 20.0), false);
    let mic = slider(v, r(140.0, 302.0, 230.0, 24.0), 50.0, ctrl, T_MIC);
    let mic_mute = checkbox(v, "Mute", r(385.0, 302.0, 80.0, 24.0), ctrl, T_MIC_MUTE);
    label(v, "Headphones", r(20.0, 342.0, 110.0, 20.0), false);
    let hp = slider(v, r(140.0, 340.0, 230.0, 24.0), 50.0, ctrl, T_HP);
    let hp_mute = checkbox(v, "Mute", r(385.0, 340.0, 80.0, 24.0), ctrl, T_HP_MUTE);
    label(v, "Hear yourself", r(20.0, 380.0, 110.0, 20.0), false);
    let mon_level = slider(v, r(140.0, 378.0, 230.0, 24.0), 50.0, ctrl, T_MON_LEVEL);
    let mon = checkbox(v, "On", r(385.0, 378.0, 80.0, 24.0), ctrl, T_MON);

    // Options
    label(v, "Options", r(20.0, 425.0, 300.0, 20.0), true);
    let sync_box = checkbox(v, "Keep in sync with the mic and macOS (updates every second)", r(20.0, 455.0, 450.0, 22.0), ctrl, T_SYNC);
    let detect_box = checkbox(v, "Detect the mute button on top of the mic", r(20.0, 483.0, 450.0, 22.0), ctrl, T_DETECT);
    label(v, "Listens for the silence the mic sends when muted. macOS shows its orange microphone dot while this is on. Nothing is recorded.", r(40.0, 506.0, 420.0, 32.0), false);
    let tint_box = checkbox(v, "Menu bar icon in mic colours: top = live, bottom = muted", r(20.0, 546.0, 450.0, 22.0), ctrl, T_TINT);
    msg![(); tint_box, "setState:", pref(PREF_TINT, true) as isize => isize];
    msg![(); sync_box, "setState:", pref(PREF_SYNC, true) as isize => isize];
    msg![(); detect_box, "setState:", pref(PREF_DETECT, false) as isize => isize];

    let status = label(v, "", r(20.0, 580.0, 440.0, 44.0), false);

    let ui = Ui {
        effect: effect as usize,
        top: top as usize,
        bottom: bottom as usize,
        speed: speed as usize,
        bright: bright as usize,
        mic: mic as usize,
        mic_mute: mic_mute as usize,
        hp: hp as usize,
        hp_mute: hp_mute as usize,
        mon: mon as usize,
        mon_level: mon_level as usize,
        status: status as usize,
        win: win as usize,
        mute_item: 0,
        login_item: 0,
        sync_box: sync_box as usize,
        detect_box: detect_box as usize,
        tint_box: tint_box as usize,
        bar_button: 0,
    };
    let (mute_item, login_item, bar_button) = build_status_item(ctrl);
    let ui = Ui { mute_item: mute_item as usize, login_item: login_item as usize, bar_button: bar_button as usize, ..ui };

    // Load current audio settings; grey out what macOS doesn't offer.
    let mut notes: Vec<String> = vec![];
    match Light::open() {
        Ok(_) => notes.push("Mic lighting found.".into()),
        Err(e) => {
            notes.push(format!("Lighting: {}", e));
            set_enabled(apply, false);
        }
    }
    match AudioDevice::find() {
        Ok(a) => {
            let setup = |vol: Option<u8>, s: Id| match vol {
                Some(x) => msg![(); s, "setDoubleValue:", x as f64 => f64],
                None => set_enabled(s, false),
            };
            setup(a.volume(Side::Mic), mic);
            setup(a.volume(Side::Headphones), hp);
            setup(a.monitoring_level(), mon_level);
            let tick = |m: Option<bool>, b: Id| match m {
                Some(x) => msg![(); b, "setState:", x as isize => isize],
                None => set_enabled(b, false),
            };
            tick(a.muted(Side::Mic), mic_mute);
            tick(a.muted(Side::Headphones), hp_mute);
            tick(a.monitoring(), mon);
        }
        Err(e) => {
            notes.push(e);
            for c in [mic, mic_mute, hp, hp_mute, mon, mon_level] {
                set_enabled(c, false);
            }
        }
    }
    let _ = UI.set(ui);
    set_status(&notes.join("  "));
    if pref(PREF_DETECT, false) {
        start_detection_in_background();
    }
    let timer = msg![Id; cls("NSTimer"), "scheduledTimerWithTimeInterval:target:selector:userInfo:repeats:",
        1.0 => f64, ctrl => Id, sel("onTick:") => Sel, NIL => Id, 1i8 => i8];
    msg![Id; timer, "retain"];

    msg![(); win, "center"];
    msg![(); win, "makeKeyAndOrderFront:", NIL => Id];
    msg![(); app, "activateIgnoringOtherApps:", 1i8 => i8];
    msg![(); app, "run"];
}
