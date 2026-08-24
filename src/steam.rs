//! Detect whether Steam is running (Windows). Used to choose lightbar claim strategy.

use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const CACHE_TTL: Duration = Duration::from_secs(3);

struct SteamCache {
    last_check: Instant,
    running: bool,
}

impl SteamCache {
    fn new() -> Self {
        Self {
            last_check: Instant::now() - CACHE_TTL,
            running: false,
        }
    }
}

static CACHE: LazyLock<Mutex<SteamCache>> = LazyLock::new(|| Mutex::new(SteamCache::new()));

#[cfg(test)]
static TEST_OVERRIDE: LazyLock<Mutex<Option<bool>>> = LazyLock::new(|| Mutex::new(None));

/// Whether `steam.exe` is currently running. Result is cached for a few seconds.
pub fn is_running() -> bool {
    #[cfg(test)]
    if let Ok(guard) = TEST_OVERRIDE.lock() {
        if let Some(value) = *guard {
            return value;
        }
    }

    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if cache.last_check.elapsed() < CACHE_TTL {
        return cache.running;
    }

    cache.running = detect_steam_running();
    cache.last_check = Instant::now();
    cache.running
}

#[cfg(test)]
pub fn set_running_for_test(running: Option<bool>) {
    if let Ok(mut guard) = TEST_OVERRIDE.lock() {
        *guard = running;
    }
    if let Ok(mut cache) = CACHE.lock() {
        cache.last_check = Instant::now() - CACHE_TTL;
    }
}

#[cfg(windows)]
fn detect_steam_running() -> bool {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(dw_flags: u32, th32_process_id: u32) -> isize;
        fn Process32FirstW(h_snapshot: isize, lppe: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(h_snapshot: isize, lppe: *mut ProcessEntry32W) -> i32;
        fn CloseHandle(h_object: isize) -> i32;
    }

    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const INVALID_HANDLE_VALUE: isize = -1;

    #[repr(C)]
    struct ProcessEntry32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }

        let mut entry = ProcessEntry32W {
            dw_size: std::mem::size_of::<ProcessEntry32W>() as u32,
            cnt_usage: 0,
            th32_process_id: 0,
            th32_default_heap_id: 0,
            th32_module_id: 0,
            cnt_threads: 0,
            th32_parent_process_id: 0,
            pc_pri_class_base: 0,
            dw_flags: 0,
            sz_exe_file: [0; 260],
        };

        let mut found = false;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let len = entry
                    .sz_exe_file
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.sz_exe_file.len());
                let name = OsString::from_wide(&entry.sz_exe_file[..len]);
                if name.to_string_lossy().eq_ignore_ascii_case("steam.exe") {
                    found = true;
                    break;
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }

        CloseHandle(snapshot);
        found
    }
}

#[cfg(not(windows))]
fn detect_steam_running() -> bool {
    false
}
