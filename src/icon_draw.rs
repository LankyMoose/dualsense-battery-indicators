//! Shared DualSense silhouette pixel art (32×32). Used by the tray and by `build.rs` for the .exe icon.

pub const SIZE: u32 = 32;

pub type Rgba = [u8; 4];

pub const TRANSPARENT: Rgba = [0, 0, 0, 0];
pub const BODY: Rgba = [235, 235, 240, 255];
pub const BODY_DIM: Rgba = [120, 120, 128, 255];
pub const SHADE: Rgba = [40, 40, 48, 255];
pub const SHADE_DIM: Rgba = [70, 70, 78, 255];
pub const ACCENT: Rgba = [0, 90, 255, 255];
pub const ACCENT_DIM: Rgba = [80, 80, 90, 255];

pub fn render_connected_rgba() -> Vec<u8> {
    flatten(render(BODY, SHADE, ACCENT))
}

pub fn render_dim_rgba() -> Vec<u8> {
    flatten(render(BODY_DIM, SHADE_DIM, ACCENT_DIM))
}

pub fn render(body: Rgba, shade: Rgba, accent: Rgba) -> Vec<Rgba> {
    let mut px = vec![TRANSPARENT; (SIZE * SIZE) as usize];
    draw_dualsense(&mut px, body, shade, accent);
    px
}

fn flatten(px: Vec<Rgba>) -> Vec<u8> {
    px.into_iter().flatten().collect()
}

/// Nearest-neighbor scale of a SIZE×SIZE RGBA buffer to `out_size`×`out_size`.
pub fn scale_rgba_nn(src: &[u8], out_size: u32) -> Vec<u8> {
    assert_eq!(src.len(), (SIZE * SIZE * 4) as usize);
    let mut out = vec![0u8; (out_size * out_size * 4) as usize];
    for y in 0..out_size {
        for x in 0..out_size {
            let sx = x * SIZE / out_size;
            let sy = y * SIZE / out_size;
            let si = ((sy * SIZE + sx) * 4) as usize;
            let di = ((y * out_size + x) * 4) as usize;
            out[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    out
}

fn draw_dualsense(px: &mut [Rgba], body: Rgba, shade: Rgba, accent: Rgba) {
    fill_rect(px, 8, 10, 23, 20, body);

    fill_rect(px, 3, 12, 9, 22, body);
    fill_rect(px, 2, 16, 6, 24, body);
    put(px, 3, 25, body);
    put(px, 4, 25, body);

    fill_rect(px, 22, 12, 28, 22, body);
    fill_rect(px, 25, 16, 29, 24, body);
    put(px, 27, 25, body);
    put(px, 28, 25, body);

    fill_rect(px, 6, 8, 12, 10, body);
    fill_rect(px, 19, 8, 25, 10, body);

    fill_rect(px, 12, 11, 19, 15, shade);
    fill_rect(px, 12, 16, 19, 17, accent);

    fill_circle(px, 9, 18, 2, shade);
    fill_circle(px, 22, 18, 2, shade);

    put(px, 7, 14, shade);
    put(px, 6, 15, shade);
    put(px, 7, 15, shade);
    put(px, 8, 15, shade);
    put(px, 7, 16, shade);

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
