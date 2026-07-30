#![windows_subsystem = "windows"]

mod battery;
mod icon;
mod tray;

fn main() {
    if let Err(err) = tray::run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
