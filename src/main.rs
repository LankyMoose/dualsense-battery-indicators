#![cfg_attr(windows, windows_subsystem = "windows")]

mod app_log;
mod autostart;
mod battery;
mod color;
mod icon;
mod lightbar;
mod tray;

use single_instance::SingleInstance;
use std::env;
use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    app_log::init();

    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        attach_console_for_cli();
        println!(
            "{} {}",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION")
        );
        return ExitCode::SUCCESS;
    }

    if args.iter().any(|a| a == "--install-autostart") {
        attach_console_for_cli();
        return match autostart::install() {
            Ok(()) => {
                println!("Autostart installed.");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("error: {err}");
                app_log::error(format!("autostart install failed: {err}"));
                ExitCode::FAILURE
            }
        };
    }

    if args.iter().any(|a| a == "--uninstall-autostart") {
        attach_console_for_cli();
        return match autostart::uninstall() {
            Ok(()) => {
                println!("Autostart removed.");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("error: {err}");
                app_log::error(format!("autostart uninstall failed: {err}"));
                ExitCode::FAILURE
            }
        };
    }

    let instance = match SingleInstance::new("ps5-battery-display") {
        Ok(instance) => instance,
        Err(err) => {
            app_log::error(format!("single-instance init failed: {err}"));
            return ExitCode::FAILURE;
        }
    };
    if !instance.is_single() {
        app_log::info("another instance is already running; exiting");
        return ExitCode::SUCCESS;
    }

    if let Err(err) = tray::run() {
        app_log::error(format!("tray exited with error: {err}"));
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn attach_console_for_cli() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn AttachConsole(dw_process_id: u32) -> i32;
            fn AllocConsole() -> i32;
        }

        const ATTACH_PARENT_PROCESS: u32 = 0xFFFFFFFF;
        unsafe {
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                AllocConsole();
            }
        }
        let _ = io::stdout().flush();
    }
}
