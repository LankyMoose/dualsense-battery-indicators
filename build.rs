//! Build script: embed Windows .exe icon from the same DualSense silhouette as the tray.

#[path = "src/icon_draw.rs"]
mod icon_draw;

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const DISPLAY_NAME: &str = "DualSense Battery Indicators";

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let connected = icon_draw::render_connected_rgba();
    let dim = icon_draw::render_dim_rgba();

    write_embedded_bytes(&out_dir, &connected, &dim);

    #[cfg(windows)]
    {
        write_app_ico(&out_dir, &connected);
        let ico_path = out_dir.join("app.ico");
        let (version_display, version_packed) = cargo_version_quad();
        let mut res = winres::WindowsResource::new();
        res.set_icon(ico_path.to_str().expect("utf-8 icon path"));
        res.set("ProductName", DISPLAY_NAME);
        res.set(
            "FileDescription",
            "Show DualSense controller battery levels in the system tray",
        );
        res.set("CompanyName", "LankyMoose");
        res.set("LegalCopyright", "Copyright (c) 2026 LankyMoose");
        res.set("OriginalFilename", "dualsense-battery-indicators.exe");
        res.set("InternalName", "dualsense-battery-indicators");
        res.set("FileVersion", &version_display);
        res.set("ProductVersion", &version_display);
        res.set_version_info(winres::VersionInfo::FILEVERSION, version_packed);
        res.set_version_info(winres::VersionInfo::PRODUCTVERSION, version_packed);
        if let Err(err) = res.compile() {
            // Missing Windows SDK / RC tooling should not block non-icon builds in CI-like envs.
            println!("cargo:warning=winres failed to embed icon: {err}");
        }
    }

    println!("cargo:rerun-if-changed=src/icon_draw.rs");
    println!("cargo:rerun-if-changed=build.rs");
}

/// `0.1.10` → display `0.1.10.0` and the packed 64-bit VERSIONINFO quad.
fn cargo_version_quad() -> (String, u64) {
    let ver = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut parts = [0u64; 4];
    for (i, p) in ver.split('.').take(4).enumerate() {
        parts[i] = p.parse().unwrap_or(0);
    }
    let packed = (parts[0] << 48) | (parts[1] << 32) | (parts[2] << 16) | parts[3];
    let display = format!("{}.{}.{}.{}", parts[0], parts[1], parts[2], parts[3]);
    (display, packed)
}

fn write_embedded_bytes(out_dir: &Path, connected: &[u8], dim: &[u8]) {
    let path = out_dir.join("icon_embedded.rs");
    let mut file = fs::File::create(&path).expect("create icon_embedded.rs");
    writeln!(file, "pub const CONNECTED_RGBA: &[u8] = &{:?};", connected).unwrap();
    writeln!(file, "pub const DIM_RGBA: &[u8] = &{:?};", dim).unwrap();
}

#[cfg(windows)]
fn write_app_ico(out_dir: &Path, connected_32: &[u8]) {
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
