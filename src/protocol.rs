//! Wire format for the HyperX QuadCast S lighting controller.
//!
//! Every message is one HID *feature* report on USB device 03F0:028C,
//! interface 0 (vendor usage page 0xFF90). A report is 65 bytes: byte 0 is
//! the report ID (always 0) followed by a 64-byte payload. On macOS the
//! report ID is passed separately, so the transport sends bytes 1..65.
//!
//! The layout was confirmed on a real QuadCast S from Windows (HidD_SetFeature,
//! 1,024 reports accepted, 0 errors) before this crate was written.

pub const VENDOR_ID: u16 = 0x03F0;
pub const LIGHT_PRODUCT_ID: u16 = 0x028C;
pub const LIGHT_USAGE_PAGE: u16 = 0xFF90;
/// Report length including the leading report-ID byte.
pub const REPORT_LEN: usize = 65;
/// Frames (top+bottom colour pairs) carried by one colour report.
pub const FRAMES_PER_REPORT: usize = 8;
/// Largest animation the device header can describe (frame count is one byte).
pub const MAX_FRAMES: usize = 255;
/// Delay the device needs between reports.
pub const REPORT_GAP_MS: u64 = 15;
/// In live (direct) mode the colour must be refreshed at least this often.
pub const LIVE_REFRESH_MS: u64 = 50;

const COLOUR_MARK: u8 = 0x81;
const FRAME_STRIDE: usize = 8;

const REG_SAVE_START: u8 = 0x53;
const REG_POST_SAVE: u8 = 0x02;
const REG_STOP_SAVE: u8 = 0x23;
const REG_APPLY: u8 = 0xF2;

pub type Report = [u8; REPORT_LEN];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const OFF: Rgb = Rgb(0, 0, 0);

    /// Parses `ff8800`, `#ff8800` or a colour name.
    pub fn parse(s: &str) -> Option<Rgb> {
        let named = match s.to_ascii_lowercase().as_str() {
            "red" => Some(Rgb(255, 0, 0)),
            "green" => Some(Rgb(0, 255, 0)),
            "blue" => Some(Rgb(0, 0, 255)),
            "white" => Some(Rgb(255, 255, 255)),
            "yellow" => Some(Rgb(255, 255, 0)),
            "cyan" => Some(Rgb(0, 255, 255)),
            "magenta" | "pink" => Some(Rgb(255, 0, 255)),
            "orange" => Some(Rgb(255, 128, 0)),
            "purple" => Some(Rgb(128, 0, 255)),
            "off" | "black" => Some(Rgb::OFF),
            _ => None,
        };
        if named.is_some() {
            return named;
        }
        let h = s.trim_start_matches('#');
        if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let v = u32::from_str_radix(h, 16).ok()?;
        Some(Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    /// Scales the colour by `percent` (0-100).
    pub fn dim(self, percent: u8) -> Rgb {
        let p = percent.min(100) as u32;
        let f = |c: u8| ((c as u32 * p + 50) / 100) as u8;
        Rgb(f(self.0), f(self.1), f(self.2))
    }

    /// Linear blend: t = 0 gives self, t = 255 gives other.
    pub fn mix(self, other: Rgb, t: u8) -> Rgb {
        let t = t as u32;
        let f = |a: u8, b: u8| ((a as u32 * (255 - t) + b as u32 * t + 127) / 255) as u8;
        Rgb(f(self.0, other.0), f(self.1, other.1), f(self.2, other.2))
    }

    /// Hue wheel position 0-359 at full saturation and value.
    pub fn hue(deg: u32) -> Rgb {
        let h = deg % 360;
        let x = ((h % 60) * 255 / 60) as u8;
        match h / 60 {
            0 => Rgb(255, x, 0),
            1 => Rgb(255 - x, 255, 0),
            2 => Rgb(0, 255, x),
            3 => Rgb(0, 255 - x, 255),
            4 => Rgb(x, 0, 255),
            _ => Rgb(255, 0, 255 - x),
        }
    }
}

/// One animation step: colour of the top and bottom light zones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub top: Rgb,
    pub bottom: Rgb,
}

impl Frame {
    pub fn both(c: Rgb) -> Frame {
        Frame { top: c, bottom: c }
    }
    pub fn dim(self, percent: u8) -> Frame {
        Frame { top: self.top.dim(percent), bottom: self.bottom.dim(percent) }
    }
}

/// Colour report holding up to eight frames.
pub fn colour_report(frames: &[Frame]) -> Report {
    assert!(frames.len() <= FRAMES_PER_REPORT, "at most 8 frames per report");
    let mut r = [0u8; REPORT_LEN];
    for (i, f) in frames.iter().enumerate() {
        let b = FRAME_STRIDE * i;
        r[b + 1] = COLOUR_MARK;
        r[b + 2] = f.top.0;
        r[b + 3] = f.top.1;
        r[b + 4] = f.top.2;
        r[b + 5] = COLOUR_MARK;
        r[b + 6] = f.bottom.0;
        r[b + 7] = f.bottom.1;
        r[b + 8] = f.bottom.2;
    }
    r
}

pub fn register_report(register: u8, p1: u8, p2: u8) -> Report {
    let mut r = [0u8; REPORT_LEN];
    r[1] = 0x04;
    r[2] = register;
    r[8] = p1;
    r[9] = p2;
    r
}

/// "Show this colour now" (live/direct mode).
pub fn apply_report() -> Report {
    register_report(REG_APPLY, 0, 1)
}

pub fn end_report(frame_count: u8) -> Report {
    let mut r = [0u8; REPORT_LEN];
    r[0x01] = 0x08;
    r[0x3C] = 0x28;
    r[0x3D] = frame_count;
    r[0x3E] = 0x00;
    r[0x3F] = 0xAA;
    r[0x40] = 0x55;
    r
}

/// Reports that show a single frame immediately. Must be resent every
/// `LIVE_REFRESH_MS` or the device falls back to its stored effect.
pub fn live_reports(frame: Frame) -> [Report; 2] {
    [colour_report(&[frame]), apply_report()]
}

/// Reports that store an animation in the microphone's memory so it plays
/// with no software running. Send in order with `REPORT_GAP_MS` between.
pub fn save_reports(frames: &[Frame]) -> Result<Vec<Report>, String> {
    if frames.is_empty() {
        return Err("no frames to save".into());
    }
    if frames.len() > MAX_FRAMES {
        return Err(format!("{} frames is more than the device limit of {}", frames.len(), MAX_FRAMES));
    }
    let packets = frames.len().div_ceil(FRAMES_PER_REPORT);
    let mut out = Vec::with_capacity(packets + 6);
    out.push(register_report(REG_SAVE_START, packets as u8, 0));
    for chunk in frames.chunks(FRAMES_PER_REPORT) {
        out.push(colour_report(chunk));
    }
    out.push(register_report(REG_POST_SAVE, 0, 0));
    out.push(register_report(REG_STOP_SAVE, 1, 0));
    out.push(end_report(frames.len() as u8));
    out.push(register_report(REG_POST_SAVE, 0, 0));
    // Show the first frame straight away, as the Windows software does.
    out.push(colour_report(&frames[..1]));
    out.push(apply_report());
    Ok(out)
}

pub fn hex(r: &Report) -> String {
    r.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_layout_matches_device() {
        let r = colour_report(&[Frame { top: Rgb(1, 2, 3), bottom: Rgb(4, 5, 6) }]);
        assert_eq!(&r[..9], &[0, 0x81, 1, 2, 3, 0x81, 4, 5, 6]);
        assert!(r[9..].iter().all(|&b| b == 0));
    }

    #[test]
    fn eighth_frame_fits() {
        let f = Frame::both(Rgb(9, 9, 9));
        let r = colour_report(&[f; 8]);
        assert_eq!(r[57], 0x81);
        assert_eq!(&r[62..65], &[9, 9, 9]);
    }

    #[test]
    fn apply_matches_tested_bytes() {
        let r = apply_report();
        assert_eq!((r[1], r[2], r[8], r[9]), (0x04, 0xF2, 0, 1));
    }

    #[test]
    fn save_sequence_shape() {
        let frames = vec![Frame::both(Rgb(255, 0, 0)); 10];
        let s = save_reports(&frames).unwrap();
        assert_eq!(s[0][2], 0x53);
        assert_eq!(s[0][8], 2); // two colour packets
        assert_eq!(s[3][2], 0x02);
        assert_eq!(s[4][2], 0x23);
        assert_eq!(s[5][1], 0x08);
        assert_eq!(s[5][0x3D], 10);
        assert_eq!((s[5][0x3F], s[5][0x40]), (0xAA, 0x55));
        assert_eq!(s.len(), 9);
    }

    #[test]
    fn parse_colours() {
        assert_eq!(Rgb::parse("#FF8800"), Some(Rgb(255, 136, 0)));
        assert_eq!(Rgb::parse("blue"), Some(Rgb(0, 0, 255)));
        assert_eq!(Rgb::parse("nope"), None);
    }
}
