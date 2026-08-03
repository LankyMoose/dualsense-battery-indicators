//! Battery-driven lightbar colors.

use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const WHITE: Self = Self::new(255, 255, 255);
    pub const ORANGE: Self = Self::new(255, 100, 0);

    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            r: lerp_u8(self.r, other.r, t),
            g: lerp_u8(self.g, other.g, t),
            b: lerp_u8(self.b, other.b, t),
        }
    }

    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    pub fn to_hsv(self) -> (f32, f32, f32) {
        let r = self.r as f32 / 255.0;
        let g = self.g as f32 / 255.0;
        let b = self.b as f32 / 255.0;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;

        let h = if delta < f32::EPSILON {
            0.0
        } else if max == r {
            60.0 * (((g - b) / delta) % 6.0)
        } else if max == g {
            60.0 * (((b - r) / delta) + 2.0)
        } else {
            60.0 * (((r - g) / delta) + 4.0)
        };
        let h = if h < 0.0 { h + 360.0 } else { h };
        let s = if max < f32::EPSILON { 0.0 } else { delta / max };
        (h, s, max)
    }
}

/// Three-stop spectrum: full (100%) → mid (50%) → empty (0%).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatterySpectrum {
    pub full: Rgb,
    pub mid: Rgb,
    pub empty: Rgb,
}

impl Default for BatterySpectrum {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl BatterySpectrum {
    /// Default blue → purple → red stops.
    pub const DEFAULT: Self = Self {
        full: Rgb::new(0x41, 0x41, 0xFB),
        mid: Rgb::new(0x66, 0x05, 0xBE),
        empty: Rgb::new(0xBE, 0x00, 0x00),
    };

    pub fn color_at_percent(self, percent: u8) -> Rgb {
        let percent = percent.min(100);
        if percent >= 50 {
            // 100% → 50%: full → mid
            let t = (100 - percent) as f32 / 50.0;
            self.full.lerp(self.mid, t)
        } else {
            // 50% → 0%: mid → empty
            let t = (50 - percent) as f32 / 50.0;
            self.mid.lerp(self.empty, t)
        }
    }
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn active_spectrum() -> &'static Mutex<BatterySpectrum> {
    static SPECTRUM: OnceLock<Mutex<BatterySpectrum>> = OnceLock::new();
    SPECTRUM.get_or_init(|| Mutex::new(BatterySpectrum::DEFAULT))
}

/// Install the spectrum used by [`color_for_battery_percent`].
pub fn set_active_spectrum(spectrum: BatterySpectrum) {
    if let Ok(mut guard) = active_spectrum().lock() {
        *guard = spectrum;
    }
}

/// Map battery percent through the active three-stop RGB spectrum.
pub fn color_for_battery_percent(percent: u8) -> Rgb {
    let spectrum = active_spectrum()
        .lock()
        .map(|g| *g)
        .unwrap_or(BatterySpectrum::DEFAULT);
    spectrum.color_at_percent(percent)
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Rgb {
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let c = v * s;
    let h_prime = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());
    let m = v - c;

    let (r1, g1, b1) = match h_prime as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    Rgb {
        r: ((r1 + m) * 255.0).round() as u8,
        g: ((g1 + m) * 255.0).round() as u8,
        b: ((b1 + m) * 255.0).round() as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_stops_match_expected_hex() {
        let s = BatterySpectrum::DEFAULT;
        assert_eq!(s.color_at_percent(100), Rgb::new(0x41, 0x41, 0xFB));
        assert_eq!(s.color_at_percent(50), Rgb::new(0x66, 0x05, 0xBE));
        assert_eq!(s.color_at_percent(0), Rgb::new(0xBE, 0x00, 0x00));
    }

    #[test]
    fn full_battery_is_blueish() {
        let c = BatterySpectrum::DEFAULT.color_at_percent(100);
        assert!(c.b > c.r && c.b > c.g, "expected blue-dominant, got {c:?}");
    }

    #[test]
    fn mid_battery_is_purpleish() {
        let c = BatterySpectrum::DEFAULT.color_at_percent(50);
        assert!(c.r > 0 && c.b > 0, "expected purple-ish, got {c:?}");
        assert!(c.g < c.r && c.g < c.b, "expected low green, got {c:?}");
    }

    #[test]
    fn empty_battery_is_redish() {
        for percent in [0u8, 5] {
            let c = BatterySpectrum::DEFAULT.color_at_percent(percent);
            assert!(
                c.r > c.g && c.r >= c.b,
                "expected red-dominant at {percent}%, got {c:?}"
            );
        }
    }

    #[test]
    fn lerp_midpoint_between_full_and_mid() {
        let s = BatterySpectrum::DEFAULT;
        let c = s.color_at_percent(75);
        // Halfway from #4141FB to #6605BE
        assert_eq!(c, Rgb::new(84, 35, 221));
    }

    #[test]
    fn active_spectrum_is_used() {
        set_active_spectrum(BatterySpectrum {
            full: Rgb::new(0, 255, 0),
            mid: Rgb::new(255, 255, 0),
            empty: Rgb::new(255, 0, 0),
        });
        assert_eq!(color_for_battery_percent(100), Rgb::new(0, 255, 0));
        set_active_spectrum(BatterySpectrum::DEFAULT);
    }
}
