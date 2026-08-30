//! Windows autostart helpers.
//!
//! Unpackaged (GitHub `.exe`): a Startup-folder `.lnk` so login does not flash a console.
//! Packaged (Microsoft Store / sideload MSIX): the `windows.startupTask` declared in the
//! Appx manifest (`DualSenseBatteryIndicatorsStartup`).

use crate::app_log;
use crate::app_meta::PKG_NAME;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Must match `desktop:StartupTask TaskId` in `msix/AppxManifest.xml`.
#[cfg(windows)]
pub const STARTUP_TASK_ID: &str = "DualSenseBatteryIndicatorsStartup";

pub fn is_enabled() -> bool {
    #[cfg(windows)]
    {
        if is_packaged() {
            return packaged_is_enabled();
        }
        migrate_legacy_cmd();
        startup_lnk_path()
            .map(|path| path.exists())
            .unwrap_or(false)
    }

    #[cfg(not(windows))]
    {
        false
    }
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    if enabled { install() } else { uninstall() }
}

/// If an older Startup `.cmd` entry exists, replace it with a silent `.lnk`.
/// Safe to call on every launch. No-op when running from an MSIX package.
pub fn ensure_quiet_entry() {
    #[cfg(windows)]
    {
        if is_packaged() {
            // Avoid double-start if the user previously used the GitHub exe.
            if let Ok(path) = startup_lnk_path() {
                let _ = remove_if_exists(&path);
            }
            if let Ok(path) = startup_cmd_path() {
                let _ = remove_if_exists(&path);
            }
            return;
        }
        migrate_legacy_cmd();
    }
}

pub fn install() -> Result<(), String> {
    #[cfg(not(windows))]
    {
        return Err("autostart install is only supported on Windows".into());
    }

    #[cfg(windows)]
    {
        if is_packaged() {
            return packaged_set_enabled(true);
        }
        let exe = env::current_exe().map_err(|e| e.to_string())?;
        let lnk_path = startup_lnk_path()?;
        if let Some(parent) = lnk_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let work_dir = exe
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        create_shortcut(&lnk_path, &exe, &work_dir)?;
        remove_if_exists(&startup_cmd_path()?)?;

        app_log::info(format!("installed autostart at {}", lnk_path.display()));
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
        if is_packaged() {
            return packaged_set_enabled(false);
        }
        let lnk_path = startup_lnk_path()?;
        let cmd_path = startup_cmd_path()?;
        let mut removed_any = false;

        if remove_if_exists(&lnk_path)? {
            app_log::info(format!("removed autostart at {}", lnk_path.display()));
            removed_any = true;
        }
        if remove_if_exists(&cmd_path)? {
            app_log::info(format!(
                "removed legacy autostart at {}",
                cmd_path.display()
            ));
            removed_any = true;
        }
        if !removed_any {
            app_log::info("autostart entry was not present");
        }
        Ok(())
    }
}

/// True when this process is running inside an MSIX/Store package.
#[cfg(windows)]
pub fn is_packaged() -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentPackageFullName(
            package_full_name_length: *mut u32,
            package_full_name: *mut u16,
        ) -> i32;
    }
    const APPMODEL_ERROR_NO_PACKAGE: i32 = 15700;
    let mut len = 0u32;
    let rc = unsafe { GetCurrentPackageFullName(&mut len, std::ptr::null_mut()) };
    rc != APPMODEL_ERROR_NO_PACKAGE
}

#[cfg(windows)]
fn packaged_is_enabled() -> bool {
    match packaged_state() {
        Ok(state) => state.is_on(),
        Err(err) => {
            app_log::warn(format!("startup task state: {err}"));
            false
        }
    }
}

#[cfg(windows)]
fn packaged_set_enabled(enabled: bool) -> Result<(), String> {
    use windows::ApplicationModel::{StartupTask, StartupTaskState};
    use windows::core::HSTRING;

    let task = StartupTask::GetAsync(&HSTRING::from(STARTUP_TASK_ID))
        .and_then(|op| op.get())
        .map_err(|e| format!("startup task: {e}"))?;

    if enabled {
        match task.State().map_err(|e| e.to_string())? {
            StartupTaskState::Enabled | StartupTaskState::EnabledByPolicy => {}
            StartupTaskState::DisabledByUser => {
                return Err(
                    "Start with Windows was turned off in Task Manager or Settings → Apps → Startup. Enable it there, then try again."
                        .into(),
                );
            }
            StartupTaskState::DisabledByPolicy => {
                return Err("Start with Windows is disabled by policy on this PC.".into());
            }
            _ => {
                let new_state = task
                    .RequestEnableAsync()
                    .and_then(|op| op.get())
                    .map_err(|e| format!("enable startup task: {e}"))?;
                if !PackagedState(new_state).is_on() {
                    return Err("Windows did not enable the startup task".into());
                }
            }
        }
        app_log::info("enabled MSIX startup task");
    } else {
        task.Disable()
            .map_err(|e| format!("disable startup task: {e}"))?;
        app_log::info("disabled MSIX startup task");
    }
    Ok(())
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct PackagedState(windows::ApplicationModel::StartupTaskState);

#[cfg(windows)]
impl PackagedState {
    fn is_on(self) -> bool {
        use windows::ApplicationModel::StartupTaskState;
        matches!(
            self.0,
            StartupTaskState::Enabled | StartupTaskState::EnabledByPolicy
        )
    }
}

#[cfg(windows)]
fn packaged_state() -> Result<PackagedState, String> {
    use windows::ApplicationModel::StartupTask;
    use windows::core::HSTRING;

    let task = StartupTask::GetAsync(&HSTRING::from(STARTUP_TASK_ID))
        .and_then(|op| op.get())
        .map_err(|e| e.to_string())?;
    Ok(PackagedState(task.State().map_err(|e| e.to_string())?))
}

/// Replace a leftover Startup `.cmd` with a silent `.lnk` (fixes console flash on login).
#[cfg(windows)]
fn migrate_legacy_cmd() {
    let Ok(cmd_path) = startup_cmd_path() else {
        return;
    };
    let Ok(lnk_path) = startup_lnk_path() else {
        return;
    };
    if !cmd_path.exists() {
        return;
    }
    // Prefer migrating in place so the next login is quiet even if the user never
    // toggles the setting. If shortcut creation fails, leave the .cmd alone.
    if let Err(err) = install() {
        app_log::warn(format!(
            "failed to migrate legacy autostart {} → {}: {err}",
            cmd_path.display(),
            lnk_path.display()
        ));
    }
}

#[cfg(windows)]
fn create_shortcut(lnk: &Path, target: &Path, work_dir: &Path) -> Result<(), String> {
    let script = format!(
        "$s = (New-Object -ComObject WScript.Shell).CreateShortcut('{}'); \
         $s.TargetPath = '{}'; \
         $s.WorkingDirectory = '{}'; \
         $s.Save()",
        ps_single_quoted(&lnk.to_string_lossy()),
        ps_single_quoted(&target.to_string_lossy()),
        ps_single_quoted(&work_dir.to_string_lossy()),
    );

    let status = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("failed to run powershell for shortcut: {e}"))?;

    if !status.success() {
        return Err(format!(
            "powershell failed creating shortcut (exit {status})"
        ));
    }
    if !lnk.exists() {
        return Err("shortcut was not created".into());
    }
    Ok(())
}

#[cfg(windows)]
fn ps_single_quoted(s: &str) -> String {
    // PowerShell single-quoted strings escape ' by doubling it.
    s.replace('\'', "''")
}

#[cfg(windows)]
fn remove_if_exists(path: &Path) -> Result<bool, String> {
    if path.as_os_str().is_empty() {
        return Ok(false);
    }
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(windows)]
fn startup_lnk_path() -> Result<PathBuf, String> {
    Ok(startup_dir()?.join(format!("{PKG_NAME}.lnk")))
}

#[cfg(windows)]
fn startup_cmd_path() -> Result<PathBuf, String> {
    Ok(startup_dir()?.join(format!("{PKG_NAME}.cmd")))
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
