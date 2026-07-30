use crate::battery::ControllerStatus;
use tray_icon::Icon;

const SIZE: u32 = 32;

type Rgba = [u8; 4];

const TRANSPARENT: Rgba = [0, 0, 0, 0];
const BODY: Rgba = [235, 235, 240, 255];
const BODY_DIM: Rgba = [120, 120, 128, 255];
const SHADE: Rgba = [40, 40, 48, 255];
const SHADE_DIM: Rgba = [70, 70, 78, 255];
const ACCENT: Rgba = [0, 90, 255, 255]; // DualSense-ish lightbar hint
const ACCENT_DIM: Rgba = [80, 80, 90, 255];

pub fn icon_for_controllers(controllers: &[ControllerStatus]) -> Icon {
    let connected = !controllers.is_empty();
    controller_icon(connected)
}

pub fn tooltip_for_controllers(controllers: &[ControllerStatus]) -> String {
    let count = controllers.len();
    match count {
        0 => "No controllers connected".into(),
        1 => "1 controller connected".into(),
        n => format!("{n} controllers connected"),
    }
}

fn controller_icon(connected: bool) -> Icon {
    let mut px = vec![TRANSPARENT; (SIZE * SIZE) as usize];
    let body = if connected { BODY } else { BODY_DIM };
    let shade = if connected { SHADE } else { SHADE_DIM };
    let accent = if connected { ACCENT } else { ACCENT_DIM };
    draw_dualsense(&mut px, body, shade, accent);
    rgba_icon(px)
}

/// Compact DualSense silhouette for a 32×32 tray icon.
fn draw_dualsense(px: &mut [Rgba], body: Rgba, shade: Rgba, accent: Rgba) {
    // Main body (center rectangle with soft corners).
    fill_rect(px, 8, 10, 23, 20, body);

    // Left grip (downward lobe).
    fill_rect(px, 3, 12, 9, 22, body);
    fill_rect(px, 2, 16, 6, 24, body);
    put(px, 3, 25, body);
    put(px, 4, 25, body);

    // Right grip (downward lobe).
    fill_rect(px, 22, 12, 28, 22, body);
    fill_rect(px, 25, 16, 29, 24, body);
    put(px, 27, 25, body);
    put(px, 28, 25, body);

    // Upper shoulders / triggers silhouette.
    fill_rect(px, 6, 8, 12, 10, body);
    fill_rect(px, 19, 8, 25, 10, body);

    // Touchpad / center plate.
    fill_rect(px, 12, 11, 19, 15, shade);

    // Lightbar strip under the touchpad.
    fill_rect(px, 12, 16, 19, 17, accent);

    // Left stick.
    fill_circle(px, 9, 18, 2, shade);
    // Right stick.
    fill_circle(px, 22, 18, 2, shade);

    // D-pad hint (left).
    put(px, 7, 14, shade);
    put(px, 6, 15, shade);
    put(px, 7, 15, shade);
    put(px, 8, 15, shade);
    put(px, 7, 16, shade);

    // Face-button hints (right).
    put(px, 23, 13, shade);
    put(px, 22, 14, shade);
    put(px, 24, 14, shade);
    put(px, 23, 15, shade);
}

fn fill_rect(px: &mut [Rgba], x0: i32, y0: i32, x1: i32, y1: i32, color: Rgba) {
    for y in y0..=y1 {
        for x in x0..=x1 {
            put(px, x, y, color);
        }
    }
}

fn fill_circle(px: &mut [Rgba], cx: i32, cy: i32, radius: i32, color: Rgba) {
    for y in (cy - radius)..=(cy + radius) {
        for x in (cx - radius)..=(cx + radius) {
            let dx = x - cx;
            let dy = y - cy;
            if dx * dx + dy * dy <= radius * radius {
                put(px, x, y, color);
            }
        }
    }
}

fn put(px: &mut [Rgba], x: i32, y: i32, color: Rgba) {
    if (0..SIZE as i32).contains(&x) && (0..SIZE as i32).contains(&y) {
        px[(y as u32 * SIZE + x as u32) as usize] = color;
    }
}

fn rgba_icon(px: Vec<Rgba>) -> Icon {
    let flat: Vec<u8> = px.iter().flat_map(|c| *c).collect();
    Icon::from_rgba(flat, SIZE, SIZE).expect("valid 32x32 rgba icon")
}
