//! Windows Startup-folder autostart helpers.

use crate::app_log;
use std::env;
use std::fs;
use std::path::PathBuf;

const STARTUP_NAME: &str = "ps5-battery-display.cmd";

pub fn is_supported() -> bool {
    cfg!(windows)
}

pub fn is_enabled() -> bool {
    #[cfg(windows)]
    {
        startup_path()
            .map(|path| path.exists())
            .unwrap_or(false)
    }

    #[cfg(not(windows))]
    {
        false
    }
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    if enabled {
        install()
    } else {
        uninstall()
    }
}

pub fn install() -> Result<(), String> {
    #[cfg(not(windows))]
    {
        return Err("autostart install is only supported on Windows".into());
    }

    #[cfg(windows)]
    {
        let exe = env::current_exe().map_err(|e| e.to_string())?;
        let cmd_path = startup_path()?;
        if let Some(parent) = cmd_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let contents = format!("@echo off\r\nstart \"\" \"{}\"\r\n", exe.display());
        fs::write(&cmd_path, contents).map_err(|e| e.to_string())?;
        app_log::info(format!("installed autostart at {}", cmd_path.display()));
        Ok(())
    }
}

pub fn uninstall() -> Result<(), String> {
    #[cfg(not(windows))]
    {
        return Err("autostart uninstall is only supported on Windows".into());
    }

    #[cfg(windows)]
    {
        let cmd_path = startup_path()?;
        if cmd_path.exists() {
            fs::remove_file(&cmd_path).map_err(|e| e.to_string())?;
            app_log::info(format!("removed autostart at {}", cmd_path.display()));
        } else {
            app_log::info("autostart entry was not present");
        }
        Ok(())
    }
}

#[cfg(windows)]
fn startup_path() -> Result<PathBuf, String> {
    Ok(startup_dir()?.join(STARTUP_NAME))
}

#[cfg(windows)]
fn startup_dir() -> Result<PathBuf, String> {
    let appdata = env::var_os("APPDATA").ok_or("APPDATA not set")?;
    Ok(PathBuf::from(appdata)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup"))
}
