use crate::battery::{ControllerStatus, PowerState};
use tray_icon::Icon;

const SIZE: u32 = 32;

type Rgba = [u8; 4];

const TRANSPARENT: Rgba = [0, 0, 0, 0];
const OUTLINE: Rgba = [240, 240, 240, 255];
const EMPTY_FILL: Rgba = [55, 55, 55, 220];
const GREEN: Rgba = [76, 185, 68, 255];
const YELLOW: Rgba = [230, 180, 40, 255];
const RED: Rgba = [220, 70, 60, 255];
const BLUE: Rgba = [70, 150, 255, 255];
const GRAY: Rgba = [140, 140, 140, 255];
const BOLT: Rgba = [255, 220, 60, 255];

pub fn icon_for_controllers(controllers: &[ControllerStatus]) -> Icon {
    match representative(controllers) {
        Some(c) => battery_icon(c.percent, c.state),
        None => disconnected_icon(),
    }
}

pub fn tooltip_for_controllers(controllers: &[ControllerStatus]) -> String {
    if controllers.is_empty() {
        return "No DualSense".into();
    }

    let lowest = controllers.iter().map(|c| c.percent).min().unwrap_or(0);
    let count = controllers.len();
    let noun = if count == 1 {
        "controller"
    } else {
        "controllers"
    };
    format!("Lowest {lowest}% · {count} {noun}")
}

fn representative(controllers: &[ControllerStatus]) -> Option<&ControllerStatus> {
    // Prefer the lowest battery so the tray reflects the controller that needs attention.
    controllers.iter().min_by_key(|c| c.percent)
}

fn battery_icon(percent: u8, state: PowerState) -> Icon {
    let mut px = vec![TRANSPARENT; (SIZE * SIZE) as usize];
    let fill = fill_color(percent, state);
    draw_battery(&mut px, percent, fill, state.is_charging());
    rgba_icon(px)
}

fn disconnected_icon() -> Icon {
    let mut px = vec![TRANSPARENT; (SIZE * SIZE) as usize];
    draw_battery(&mut px, 0, GRAY, false);
    // Small X to show disconnected.
    for i in 0..8 {
        put(&mut px, 12 + i, 12 + i, RED);
        put(&mut px, 19 - i, 12 + i, RED);
    }
    rgba_icon(px)
}

fn fill_color(percent: u8, state: PowerState) -> Rgba {
    if state.is_charging() {
        return BLUE;
    }
    match percent {
        0..=19 => RED,
        20..=49 => YELLOW,
        _ => GREEN,
    }
}

fn draw_battery(px: &mut [Rgba], percent: u8, fill: Rgba, charging: bool) {
    // Body outline: x=4..25, y=9..22
    for x in 4..=25 {
        put(px, x, 9, OUTLINE);
        put(px, x, 22, OUTLINE);
    }
    for y in 9..=22 {
        put(px, 4, y, OUTLINE);
        put(px, 25, y, OUTLINE);
    }
    // Terminal nub
    for x in 26..=28 {
        for y in 12..=19 {
            put(px, x, y, OUTLINE);
        }
    }

    // Inner empty area
    for x in 6..=23 {
        for y in 11..=20 {
            put(px, x, y, EMPTY_FILL);
        }
    }

    // Charge fill (left → right)
    let inner_w = 18i32; // x=6..23 inclusive = 18
    let filled = ((i32::from(percent) * inner_w) / 100).clamp(0, inner_w);
    for i in 0..filled {
        for y in 11..=20 {
            put(px, 6 + i, y, fill);
        }
    }

    if charging {
        draw_bolt(px);
    }
}

fn draw_bolt(px: &mut [Rgba]) {
    // Simple lightning bolt centered in the battery body.
    let points = [
        (16, 11),
        (15, 12),
        (14, 13),
        (13, 14),
        (15, 14),
        (14, 15),
        (13, 16),
        (12, 17),
        (14, 17),
        (13, 18),
        (12, 19),
        (15, 15),
        (16, 14),
        (17, 13),
        (16, 16),
        (15, 17),
    ];
    for (x, y) in points {
        put(px, x, y, BOLT);
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
