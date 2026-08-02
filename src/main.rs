#![cfg_attr(windows, windows_subsystem = "windows")]

mod app_log;
mod app_meta;
mod autostart;
mod battery;
mod color;
mod icon;
mod lightbar;
mod tray;

use app_meta::{DISPLAY_NAME, PKG_NAME, PKG_VERSION};
use single_instance::SingleInstance;
use std::env;
use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    app_log::init();

    let args: Vec<String> = env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        attach_console_for_cli();
        print_help();
        return ExitCode::SUCCESS;
    }

    if args.iter().any(|a| a == "--version" || a == "-V") {
        attach_console_for_cli();
        println!("{PKG_NAME} {PKG_VERSION}");
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

    if args.iter().any(|a| a == "--list-controllers") {
        attach_console_for_cli();
        return list_controllers_cli();
    }

    if let Some(unknown) = args.first() {
        attach_console_for_cli();
        eprintln!("error: unknown argument '{unknown}'");
        eprintln!();
        print_help();
        return ExitCode::FAILURE;
    }

    let instance = match SingleInstance::new(PKG_NAME) {
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

fn print_help() {
    println!("{DISPLAY_NAME} ({PKG_NAME}) {PKG_VERSION}");
    println!();
    println!("Usage: {PKG_NAME} [OPTIONS]");
    println!();
    println!("Options:");
    println!("  -h, --help                 Show this help and exit");
    println!("  -V, --version              Print version and exit");
    println!("      --install-autostart    Windows: add a Startup entry for this exe");
    println!("      --uninstall-autostart  Windows: remove that Startup entry");
    println!("      --list-controllers     Print connected DualSense pads and exit");
    println!();
    println!("With no options, starts the system tray app.");
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

fn list_controllers_cli() -> ExitCode {
    match battery::poll_controllers() {
        Ok(statuses) => {
            if statuses.is_empty() {
                println!("No DualSense controllers found.");
            } else {
                println!("{} controller(s):", statuses.len());
                for s in &statuses {
                    println!(
                        "  #{} {} ({}) id={} {}% {}",
                        s.index,
                        s.product,
                        s.connection,
                        s.serial,
                        s.percent,
                        s.state.as_str()
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
