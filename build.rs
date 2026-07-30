//! Build script: embed Windows .exe icon from the same DualSense silhouette as the tray.

#[path = "src/icon_draw.rs"]
mod icon_draw;

use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let connected = icon_draw::render_connected_rgba();
    let dim = icon_draw::render_dim_rgba();

    write_embedded_bytes(&out_dir, &connected, &dim);

    #[cfg(windows)]
    {
        write_app_ico(&out_dir, &connected);
        let ico_path = out_dir.join("app.ico");
        let mut res = winres::WindowsResource::new();
        res.set_icon(ico_path.to_str().expect("utf-8 icon path"));
        res.set("ProductName", "PS5 Battery Display");
        res.set("FileDescription", "DualSense battery tray app");
        if let Err(err) = res.compile() {
            // Missing Windows SDK / RC tooling should not block non-icon builds in CI-like envs.
            println!("cargo:warning=winres failed to embed icon: {err}");
        }
    }

    println!("cargo:rerun-if-changed=src/icon_draw.rs");
    println!("cargo:rerun-if-changed=build.rs");
}

fn write_embedded_bytes(out_dir: &PathBuf, connected: &[u8], dim: &[u8]) {
    let path = out_dir.join("icon_embedded.rs");
    let mut file = fs::File::create(&path).expect("create icon_embedded.rs");
    writeln!(
        file,
        "pub const CONNECTED_RGBA: &[u8] = &{:?};",
        connected
    )
    .unwrap();
    writeln!(file, "pub const DIM_RGBA: &[u8] = &{:?};", dim).unwrap();
}

#[cfg(windows)]
fn write_app_ico(out_dir: &PathBuf, connected_32: &[u8]) {
    use ico::{IconDir, IconDirEntry, IconImage, ResourceType};

    let mut icon_dir = IconDir::new(ResourceType::Icon);
    for size in [16u32, 32, 48, 256] {
        let rgba = if size == icon_draw::SIZE {
            connected_32.to_vec()
        } else {
            icon_draw::scale_rgba_nn(connected_32, size)
        };
        let image = IconImage::from_rgba_data(size, size, rgba);
        icon_dir.add_entry(IconDirEntry::encode(&image).expect("encode ico entry"));
    }

    let ico_path = out_dir.join("app.ico");
    let file = fs::File::create(&ico_path).expect("create app.ico");
    icon_dir.write(file).expect("write app.ico");
}
