//! Shared software-rendered UI theme and drawing primitives.

use crate::color::Rgb;
use crate::icon_draw;
use fontdue::Font;

pub const BG: u32 = rgb(23, 26, 33);
pub const PANEL: u32 = rgb(32, 37, 45);
pub const PANEL_HOVER: u32 = rgb(40, 46, 56);
pub const LINE: u32 = rgb(55, 63, 75);
pub const INK: u32 = rgb(241, 243, 245);
pub const MUTED: u32 = rgb(170, 178, 189);
pub const DIM: u32 = rgb(113, 122, 135);

pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

pub const fn rgb_of(color: Rgb) -> u32 {
    rgb(color.r, color.g, color.b)
}

pub fn load_system_ui_font() -> Result<Font, String> {
    use font_kit::family_name::FamilyName;
    use font_kit::properties::Properties;
    use font_kit::source::SystemSource;

    let handle = SystemSource::new()
        .select_best_match(&[FamilyName::SansSerif], &Properties::new())
        .map_err(|e| format!("select system UI font: {e}"))?;
    let font = handle
        .load()
        .map_err(|e| format!("load system UI font: {e}"))?;
    let data = font
        .copy_font_data()
        .ok_or_else(|| "system UI font has no accessible font data".to_string())?;
    Font::from_bytes(data.as_slice(), fontdue::FontSettings::default())
        .map_err(|e| format!("parse system UI font: {e}"))
}

pub struct Framebuffer<'a> {
    buf: &'a mut [u32],
    width: usize,
    height: usize,
    scale: f64,
    font: &'a Font,
}

impl<'a> Framebuffer<'a> {
    pub fn new(
        buf: &'a mut [u32],
        width: usize,
        height: usize,
        scale: f64,
        font: &'a Font,
    ) -> Self {
        Self {
            buf,
            width,
            height,
            scale,
            font,
        }
    }

    pub fn clear(&mut self, color: u32) {
        self.buf.fill(color);
    }

    pub fn to_phys(&self, value: f64) -> i32 {
        (value * self.scale).round() as i32
    }

    pub fn put_phys(&mut self, x: i32, y: i32, color: u32) {
        if x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height {
            self.buf[y as usize * self.width + x as usize] = color;
        }
    }

    pub fn fill_rect_phys(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u32) {
        let x0 = x0.max(0);
        let y0 = y0.max(0);
        let x1 = x1.min(self.width as i32);
        let y1 = y1.min(self.height as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                self.put_phys(x, y, color);
            }
        }
    }

    pub fn fill_rect(&mut self, x: f64, y: f64, width: f64, height: f64, color: u32) {
        self.fill_rect_phys(
            self.to_phys(x),
            self.to_phys(y),
            self.to_phys(x + width),
            self.to_phys(y + height),
            color,
        );
    }

    pub fn round_rect(
        &mut self,
        bounds: (f64, f64, f64, f64),
        radius: f64,
        fill: u32,
        border: Option<u32>,
    ) {
        let (x, y, width, height) = bounds;
        let x0 = self.to_phys(x);
        let y0 = self.to_phys(y);
        let x1 = self.to_phys(x + width);
        let y1 = self.to_phys(y + height);
        let radius = self.to_phys(radius).max(2);
        for py in y0..y1 {
            for px in x0..x1 {
                if inside_rounded(px, py, x0, y0, x1, y1, radius) {
                    self.put_phys(px, py, fill);
                }
            }
        }
        if let Some(border) = border {
            let thickness = self.to_phys(1.0).max(1);
            for py in y0..y1 {
                for px in x0..x1 {
                    if inside_rounded(px, py, x0, y0, x1, y1, radius)
                        && !inside_rounded(
                            px,
                            py,
                            x0 + thickness,
                            y0 + thickness,
                            x1 - thickness,
                            y1 - thickness,
                            (radius - thickness).max(0),
                        )
                    {
                        self.put_phys(px, py, border);
                    }
                }
            }
        }
    }

    pub fn text_width(&self, text: &str, logical_size: f32) -> f64 {
        let px = (logical_size as f64 * self.scale) as f32;
        text.chars()
            .map(|ch| self.font.metrics(ch, px).advance_width as f64)
            .sum::<f64>()
            / self.scale
    }

    pub fn text(&mut self, x: f64, y: f64, text: &str, color: u32, logical_size: f32) {
        let px = (logical_size as f64 * self.scale) as f32;
        let ascent = self
            .font
            .horizontal_line_metrics(px)
            .map(|metrics| metrics.ascent)
            .unwrap_or(px * 0.8);
        let baseline = self.to_phys(y) as f32 + ascent;
        let mut pen_x = self.to_phys(x) as f32;

        for ch in text.chars() {
            let (metrics, bitmap) = self.font.rasterize(ch, px);
            if metrics.width > 0 && metrics.height > 0 {
                let glyph_x = (pen_x + metrics.xmin as f32).round() as i32;
                let glyph_y =
                    (baseline - metrics.ymin as f32 - metrics.height as f32).round() as i32;
                for row in 0..metrics.height {
                    for col in 0..metrics.width {
                        let cover = bitmap[row * metrics.width + col];
                        if cover != 0 {
                            self.blend_phys(
                                glyph_x + col as i32,
                                glyph_y + row as i32,
                                color,
                                cover,
                            );
                        }
                    }
                }
            }
            pen_x += metrics.advance_width;
        }
    }

    pub fn icon(&mut self, x: f64, y: f64, size: f64, accent: Rgb) {
        let pixels = icon_draw::render(
            icon_draw::BODY,
            icon_draw::SHADE,
            [accent.r, accent.g, accent.b, 255],
        );
        let x0 = self.to_phys(x);
        let y0 = self.to_phys(y);
        let out = self.to_phys(size).max(1);
        for dy in 0..out {
            for dx in 0..out {
                let sx = dx * icon_draw::SIZE as i32 / out;
                let sy = dy * icon_draw::SIZE as i32 / out;
                let source = pixels[(sy as u32 * icon_draw::SIZE + sx as u32) as usize];
                if source[3] != 0 {
                    self.put_phys(x0 + dx, y0 + dy, rgb(source[0], source[1], source[2]));
                }
            }
        }
    }

    fn blend_phys(&mut self, x: i32, y: i32, color: u32, alpha: u8) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return;
        }
        let index = y as usize * self.width + x as usize;
        let dst = self.buf[index];
        let alpha = alpha as u32;
        let inverse = 255 - alpha;
        let r = (((color >> 16) & 0xff) * alpha + ((dst >> 16) & 0xff) * inverse) / 255;
        let g = (((color >> 8) & 0xff) * alpha + ((dst >> 8) & 0xff) * inverse) / 255;
        let b = ((color & 0xff) * alpha + (dst & 0xff) * inverse) / 255;
        self.buf[index] = (r << 16) | (g << 8) | b;
    }
}

fn inside_rounded(x: i32, y: i32, x0: i32, y0: i32, x1: i32, y1: i32, radius: i32) -> bool {
    if x < x0 || y < y0 || x >= x1 || y >= y1 {
        return false;
    }
    let corners = [
        (x0 + radius, y0 + radius, x < x0 + radius && y < y0 + radius),
        (
            x1 - radius - 1,
            y0 + radius,
            x >= x1 - radius && y < y0 + radius,
        ),
        (
            x0 + radius,
            y1 - radius - 1,
            x < x0 + radius && y >= y1 - radius,
        ),
        (
            x1 - radius - 1,
            y1 - radius - 1,
            x >= x1 - radius && y >= y1 - radius,
        ),
    ];
    for (cx, cy, in_corner) in corners {
        if in_corner {
            let dx = x - cx;
            let dy = y - cy;
            return dx * dx + dy * dy <= radius * radius;
        }
    }
    true
}
