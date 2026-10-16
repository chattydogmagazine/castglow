//! Lighting effects built as frame lists. Each effect can be stored on the
//! microphone (plays with no software running) or streamed live.

use crate::protocol::{Frame, Rgb, MAX_FRAMES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed {
    Slow,
    Normal,
    Fast,
}

impl Speed {
    pub fn parse(s: &str) -> Option<Speed> {
        match s.to_ascii_lowercase().as_str() {
            "slow" => Some(Speed::Slow),
            "normal" | "medium" => Some(Speed::Normal),
            "fast" => Some(Speed::Fast),
            _ => None,
        }
    }
    /// Frames per full cycle of a looping effect.
    fn cycle_len(self) -> usize {
        match self {
            Speed::Slow => 120,
            Speed::Normal => 64,
            Speed::Fast => 32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Solid,
    Blink,
    Breathing,
    Cycle,
    Wave,
    Lightning,
}

impl Effect {
    pub const ALL: [Effect; 6] =
        [Effect::Solid, Effect::Blink, Effect::Breathing, Effect::Cycle, Effect::Wave, Effect::Lightning];

    pub fn name(self) -> &'static str {
        match self {
            Effect::Solid => "solid",
            Effect::Blink => "blink",
            Effect::Breathing => "breathing",
            Effect::Cycle => "cycle",
            Effect::Wave => "wave",
            Effect::Lightning => "lightning",
        }
    }

    pub fn parse(s: &str) -> Option<Effect> {
        Effect::ALL.into_iter().find(|e| e.name() == s.to_ascii_lowercase())
    }
}

pub struct EffectSpec {
    pub effect: Effect,
    /// Top-zone colour (or the only colour).
    pub colour: Rgb,
    /// Bottom-zone colour; defaults to `colour`.
    pub colour2: Option<Rgb>,
    pub speed: Speed,
    /// Overall brightness 0-100.
    pub brightness: u8,
}

pub fn frames(spec: &EffectSpec) -> Vec<Frame> {
    let top = spec.colour;
    let bottom = spec.colour2.unwrap_or(spec.colour);
    let n = spec.speed.cycle_len();
    let raw: Vec<Frame> = match spec.effect {
        Effect::Solid => vec![Frame { top, bottom }],
        Effect::Blink => {
            let half = n / 2;
            (0..n)
                .map(|i| if i < half { Frame { top, bottom } } else { Frame::both(Rgb::OFF) })
                .collect()
        }
        Effect::Breathing => (0..n)
            .map(|i| {
                // Triangle 0 -> 255 -> 0, eased with a square for a softer glow.
                let pos = (i * 510 / n) as u32;
                let lin = if pos <= 255 { pos } else { 510 - pos };
                let level = (lin * lin / 255) as u8;
                Frame { top: Rgb::OFF.mix(top, level), bottom: Rgb::OFF.mix(bottom, level) }
            })
            .collect(),
        Effect::Cycle => (0..n)
            .map(|i| Frame::both(Rgb::hue((i * 360 / n) as u32)))
            .collect(),
        Effect::Wave => (0..n)
            .map(|i| {
                let h = (i * 360 / n) as u32;
                Frame { top: Rgb::hue(h), bottom: Rgb::hue(h + 60) }
            })
            .collect(),
        Effect::Lightning => {
            // Dark sky with an irregular double flash, top then bottom.
            let mut v = vec![Frame::both(Rgb::OFF.mix(top, 20)); n];
            let flashes = [n / 8, n / 8 + 2, n / 2, n / 2 + 1, n * 3 / 4];
            for (k, &at) in flashes.iter().enumerate() {
                if at < n {
                    v[at] = if k % 2 == 0 { Frame { top, bottom: Rgb::OFF } } else { Frame { top: Rgb::OFF, bottom } };
                }
            }
            v
        }
    };
    let mut out: Vec<Frame> = raw.into_iter().map(|f| f.dim(spec.brightness)).collect();
    out.truncate(MAX_FRAMES);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(effect: Effect) -> EffectSpec {
        EffectSpec { effect, colour: Rgb(255, 0, 0), colour2: None, speed: Speed::Normal, brightness: 100 }
    }

    #[test]
    fn every_effect_fits_device() {
        for e in Effect::ALL {
            for sp in [Speed::Slow, Speed::Normal, Speed::Fast] {
                let mut s = spec(e);
                s.speed = sp;
                let f = frames(&s);
                assert!(!f.is_empty() && f.len() <= MAX_FRAMES, "{:?}", e);
            }
        }
    }

    #[test]
    fn brightness_scales() {
        let mut s = spec(Effect::Solid);
        s.brightness = 50;
        assert_eq!(frames(&s)[0].top, Rgb(128, 0, 0));
    }

    #[test]
    fn breathing_starts_dark_and_peaks() {
        let f = frames(&spec(Effect::Breathing));
        assert_eq!(f[0].top, Rgb::OFF);
        assert!(f.iter().any(|x| x.top.0 > 240));
    }
}
