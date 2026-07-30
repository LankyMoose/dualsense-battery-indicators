//! Battery-driven lightbar colors.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

/// Map battery percent to a blue → purple → red hue.
/// 100% → hue 240° (blue), 0% → hue 360° (red), passing through purple.
pub fn color_for_battery_percent(percent: u8) -> Rgb {
    let t = (100u8.saturating_sub(percent)) as f32 / 100.0;
    let hue = 240.0 + t * 120.0; // 240..360
    hsv_to_rgb(hue, 1.0, 1.0)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Rgb {
    let c = v * s;
    let h_prime = (h % 360.0) / 60.0;
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
    fn full_battery_is_blueish() {
        let c = color_for_battery_percent(100);
        assert!(c.b > c.r && c.b > c.g, "expected blue-dominant, got {c:?}");
    }

    #[test]
    fn mid_battery_is_purpleish() {
        let c = color_for_battery_percent(50);
        assert!(c.r > 0 && c.b > 0, "expected purple-ish, got {c:?}");
        assert!(c.g < c.r && c.g < c.b, "expected low green, got {c:?}");
    }

    #[test]
    fn empty_battery_is_redish() {
        for percent in [0u8, 5] {
            let c = color_for_battery_percent(percent);
            assert!(
                c.r > c.g && c.r >= c.b,
                "expected red-dominant at {percent}%, got {c:?}"
            );
        }
    }
}
