//! Generate Microsoft Store / MSIX PNG assets from the DualSense silhouette.

#[allow(dead_code)]
#[path = "../src/icon_draw.rs"]
mod icon_draw;

use std::env;
use std::fs;
use std::io::BufWriter;
use std::path::Path;

const BG: [u8; 4] = [20, 24, 32, 255];

fn main() {
    let out = env::args()
        .nth(1)
        .unwrap_or_else(|| "msix/Assets".to_string());
    let out = Path::new(&out);
    fs::create_dir_all(out).expect("create assets dir");

    let sprite = icon_draw::render_connected_rgba();
    write_png(&out.join("StoreLogo.png"), &square_on_bg(&sprite, 50, 32));
    write_png(
        &out.join("Square44x44Logo.png"),
        &square_on_bg(&sprite, 44, 32),
    );
    write_png(
        &out.join("Square150x150Logo.png"),
        &square_on_bg(&sprite, 150, 96),
    );
    write_png(
        &out.join("Wide310x150Logo.png"),
        &wide_on_bg(&sprite, 310, 150, 96),
    );
    write_png(
        &out.join("SplashScreen.png"),
        &wide_on_bg(&sprite, 620, 300, 128),
    );
}

fn square_on_bg(sprite32: &[u8], canvas: u32, sprite_size: u32) -> (u32, u32, Vec<u8>) {
    let scaled = icon_draw::scale_rgba_nn(sprite32, sprite_size);
    let mut out = vec![0u8; (canvas * canvas * 4) as usize];
    fill(&mut out, canvas, canvas, BG);
    blit(
        &mut out,
        canvas,
        &scaled,
        sprite_size,
        sprite_size,
        ((canvas - sprite_size) / 2) as i32,
        ((canvas - sprite_size) / 2) as i32,
    );
    (canvas, canvas, out)
}

fn wide_on_bg(sprite32: &[u8], w: u32, h: u32, sprite_size: u32) -> (u32, u32, Vec<u8>) {
    let scaled = icon_draw::scale_rgba_nn(sprite32, sprite_size);
    let mut out = vec![0u8; (w * h * 4) as usize];
    fill(&mut out, w, h, BG);
    blit(
        &mut out,
        w,
        &scaled,
        sprite_size,
        sprite_size,
        ((w - sprite_size) / 2) as i32,
        ((h - sprite_size) / 2) as i32,
    );
    (w, h, out)
}

fn fill(buf: &mut [u8], w: u32, h: u32, color: [u8; 4]) {
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            buf[i..i + 4].copy_from_slice(&color);
        }
    }
}

fn blit(dst: &mut [u8], dst_w: u32, src: &[u8], src_w: u32, src_h: u32, ox: i32, oy: i32) {
    for y in 0..src_h as i32 {
        for x in 0..src_w as i32 {
            let dx = ox + x;
            let dy = oy + y;
            if dx < 0 || dy < 0 {
                continue;
            }
            let si = ((y as u32 * src_w + x as u32) * 4) as usize;
            if src[si + 3] == 0 {
                continue;
            }
            let di = ((dy as u32 * dst_w + dx as u32) * 4) as usize;
            dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
}

fn write_png(path: &Path, (w, h, rgba): &(u32, u32, Vec<u8>)) {
    let file = fs::File::create(path).unwrap_or_else(|e| panic!("create {}: {e}", path.display()));
    let mut encoder = png::Encoder::new(BufWriter::new(file), *w, *h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(rgba)
        .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}
