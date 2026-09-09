//! Reusable, no-activate overlay window for controller notifications.

use crate::color::{BatterySpectrum, Rgb};
use crate::configure_ui::load_system_ui_font;
use crate::icon_draw;
use crate::notify::NotifyEvent;
use crate::prefs::ToastPosition;
use fontdue::Font;
use softbuffer::{Context, Surface};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Duration;
#[cfg(not(windows))]
use winit::dpi::PhysicalPosition;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, OwnedDisplayHandle};
use winit::keyboard::{Key, NamedKey};
#[cfg(windows)]
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowAttributes, WindowId, WindowLevel};

#[cfg(windows)]
use winit::platform::windows::{CornerPreference, WindowAttributesExtWindows};

const WIDTH: f64 = 360.0;
const HEIGHT: f64 = 88.0;
const MARGIN: i32 = 20;
pub const SLIDE_DURATION: Duration = Duration::from_millis(250);
const BG: u32 = 0x171A21;
const INK: u32 = 0xF1F3F5;
const MUTED: u32 = 0xAAB2BD;

#[derive(Debug, Clone)]
pub struct ToastMessage {
    pub heading: String,
    pub body: String,
    pub accent: Rgb,
}

impl ToastMessage {
    pub fn from_notification(event: NotifyEvent, spectrum: BatterySpectrum) -> Self {
        Self {
            heading: event.heading,
            body: event.body,
            accent: spectrum.color_at_percent(event.percent.unwrap_or(100)),
        }
    }

    pub fn preview(accent: Rgb) -> Self {
        Self {
            heading: "DualSense Battery Indicators".to_string(),
            body: "Toasts will appear here".to_string(),
            accent,
        }
    }
}

pub struct ToastWindow {
    window: Rc<Window>,
    surface: Surface<OwnedDisplayHandle, Rc<Window>>,
    font: Font,
    message: Option<ToastMessage>,
    placement: Option<ToastPlacement>,
}

impl ToastWindow {
    pub fn open(event_loop: &ActiveEventLoop, display: OwnedDisplayHandle) -> Result<Self, String> {
        let attrs = WindowAttributes::default()
            .with_title("DualSense notification")
            .with_inner_size(PhysicalSize::new(WIDTH as u32, HEIGHT as u32))
            .with_resizable(false)
            .with_decorations(false)
            .with_visible(false)
            .with_active(false)
            .with_window_level(WindowLevel::AlwaysOnTop);

        #[cfg(windows)]
        let attrs = attrs
            .with_skip_taskbar(true)
            .with_undecorated_shadow(true)
            .with_corner_preference(CornerPreference::Round);

        let window = Rc::new(
            event_loop
                .create_window(attrs)
                .map_err(|e| format!("create toast window: {e}"))?,
        );
        configure_no_activate(&window);

        let context = Context::new(display).map_err(|e| format!("toast context: {e}"))?;
        let surface =
            Surface::new(&context, window.clone()).map_err(|e| format!("toast surface: {e}"))?;

        Ok(Self {
            window,
            surface,
            font: load_system_ui_font()?,
            message: None,
            placement: None,
        })
    }

    pub fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub fn show(&mut self, message: ToastMessage, position: ToastPosition) {
        self.message = Some(message);
        let target = target_area(&self.window);
        let scale = target.scale.max(1.0);
        let width = (WIDTH * scale).round() as u32;
        let height = (HEIGHT * scale).round() as u32;
        let (x, y) = corner_position(target.rect, width, height, position, MARGIN);
        let outside_y = match position {
            ToastPosition::TopLeft | ToastPosition::TopRight => target.rect.y - height as i32,
            ToastPosition::BottomLeft | ToastPosition::BottomRight => {
                target.rect.y + target.rect.height as i32
            }
        };
        self.placement = Some(ToastPlacement {
            x,
            outside_y,
            target_y: y,
        });

        #[cfg(windows)]
        show_topmost_no_activate(&self.window, x, outside_y, width, height);

        #[cfg(not(windows))]
        {
            let _ = self
                .window
                .request_inner_size(PhysicalSize::new(width, height));
            self.window
                .set_outer_position(PhysicalPosition::new(x, outside_y));
            self.window.set_visible(true);
        }

        self.window.request_redraw();
    }

    pub fn set_slide_progress(&self, progress: f32, sliding_out: bool) {
        let Some(placement) = self.placement else {
            return;
        };
        let eased = ease_out_cubic(progress.clamp(0.0, 1.0));
        let t = if sliding_out { 1.0 - eased } else { eased };
        let y = lerp_i32(placement.outside_y, placement.target_y, t);

        #[cfg(windows)]
        position_topmost_no_activate(&self.window, placement.x, y);

        #[cfg(not(windows))]
        self.window
            .set_outer_position(PhysicalPosition::new(placement.x, y));
    }

    pub fn hide(&mut self) {
        #[cfg(windows)]
        hide_window(&self.window);

        #[cfg(not(windows))]
        self.window.set_visible(false);

        self.message = None;
        self.placement = None;
    }

    /// Returns true when the visible toast should be dismissed.
    pub fn handle(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::CloseRequested => return true,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => return true,
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.logical_key == Key::Named(NamedKey::Escape) =>
            {
                return true;
            }
            WindowEvent::RedrawRequested => {
                if let Err(err) = self.paint() {
                    crate::app_log::warn(format!("toast paint failed: {err}"));
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => self.window.request_redraw(),
            _ => {}
        }
        false
    }

    fn paint(&mut self) -> Result<(), String> {
        let Some(message) = self.message.as_ref() else {
            return Ok(());
        };
        let size = self.window.inner_size();
        let width = NonZeroU32::new(size.width.max(1)).unwrap();
        let height = NonZeroU32::new(size.height.max(1)).unwrap();
        self.surface
            .resize(width, height)
            .map_err(|e| format!("resize: {e}"))?;
        let mut buffer = self
            .surface
            .buffer_mut()
            .map_err(|e| format!("buffer: {e}"))?;
        let scale = self.window.scale_factor();
        let mut fb = Framebuffer {
            buf: &mut buffer,
            w: width.get() as usize,
            h: height.get() as usize,
            scale,
            font: &self.font,
        };

        fb.clear(BG);
        fb.fill_rect(0.0, 0.0, 5.0, HEIGHT, rgb(message.accent));
        fb.icon(18.0, 18.0, 52.0, message.accent);
        fb.text(84.0, 20.0, &message.heading, INK, 15.0);
        fb.text(84.0, 47.0, &message.body, MUTED, 13.0);
        buffer.present().map_err(|e| format!("present: {e}"))
    }
}

#[derive(Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

struct TargetArea {
    rect: Rect,
    scale: f64,
}

#[derive(Clone, Copy)]
struct ToastPlacement {
    x: i32,
    outside_y: i32,
    target_y: i32,
}

fn ease_out_cubic(progress: f32) -> f32 {
    1.0 - (1.0 - progress).powi(3)
}

fn lerp_i32(from: i32, to: i32, progress: f32) -> i32 {
    (from as f32 + (to - from) as f32 * progress).round() as i32
}

fn corner_position(
    area: Rect,
    width: u32,
    height: u32,
    position: ToastPosition,
    margin: i32,
) -> (i32, i32) {
    let left = area.x + margin;
    let right = area.x + area.width as i32 - width as i32 - margin;
    let top = area.y + margin;
    let bottom = area.y + area.height as i32 - height as i32 - margin;
    match position {
        ToastPosition::TopLeft => (left, top),
        ToastPosition::TopRight => (right, top),
        ToastPosition::BottomLeft => (left, bottom),
        ToastPosition::BottomRight => (right, bottom),
    }
}

#[cfg(not(windows))]
fn target_area(window: &Window) -> TargetArea {
    let monitor = window.current_monitor();
    match monitor {
        Some(monitor) => {
            let position = monitor.position();
            let size = monitor.size();
            TargetArea {
                rect: Rect {
                    x: position.x,
                    y: position.y,
                    width: size.width,
                    height: size.height,
                },
                scale: monitor.scale_factor(),
            }
        }
        None => TargetArea {
            rect: Rect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            scale: window.scale_factor(),
        },
    }
}

#[cfg(windows)]
fn target_area(_window: &Window) -> TargetArea {
    let foreground = unsafe { GetForegroundWindow() };
    let monitor = unsafe { MonitorFromWindow(foreground, MONITOR_DEFAULTTOPRIMARY) };
    let mut info = MonitorInfo {
        size: std::mem::size_of::<MonitorInfo>() as u32,
        monitor: WinRect::default(),
        work: WinRect::default(),
        flags: 0,
    };
    let got_info = unsafe { GetMonitorInfoW(monitor, &mut info) } != 0;
    if !got_info {
        return TargetArea {
            rect: Rect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            scale: 1.0,
        };
    }

    let mut foreground_rect = WinRect::default();
    let got_foreground =
        foreground != 0 && unsafe { GetWindowRect(foreground, &mut foreground_rect) } != 0;
    let fullscreen = got_foreground
        && foreground_rect.left <= info.monitor.left + 2
        && foreground_rect.top <= info.monitor.top + 2
        && foreground_rect.right >= info.monitor.right - 2
        && foreground_rect.bottom >= info.monitor.bottom - 2;
    let area = if fullscreen { info.monitor } else { info.work };
    let dpi = if foreground != 0 {
        unsafe { GetDpiForWindow(foreground) }
    } else {
        96
    };
    TargetArea {
        rect: Rect {
            x: area.left,
            y: area.top,
            width: (area.right - area.left).max(1) as u32,
            height: (area.bottom - area.top).max(1) as u32,
        },
        scale: dpi.max(96) as f64 / 96.0,
    }
}

struct Framebuffer<'a> {
    buf: &'a mut [u32],
    w: usize,
    h: usize,
    scale: f64,
    font: &'a Font,
}

impl Framebuffer<'_> {
    fn clear(&mut self, color: u32) {
        self.buf.fill(color);
    }

    fn to_phys(&self, value: f64) -> i32 {
        (value * self.scale).round() as i32
    }

    fn put(&mut self, x: i32, y: i32, color: u32) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.buf[y as usize * self.w + x as usize] = color;
        }
    }

    fn fill_rect(&mut self, x: f64, y: f64, width: f64, height: f64, color: u32) {
        for py in self.to_phys(y)..self.to_phys(y + height) {
            for px in self.to_phys(x)..self.to_phys(x + width) {
                self.put(px, py, color);
            }
        }
    }

    fn icon(&mut self, x: f64, y: f64, size: f64, accent: Rgb) {
        let pixels = icon_draw::render(
            icon_draw::BODY,
            icon_draw::SHADE,
            [accent.r, accent.g, accent.b, 255],
        );
        let x0 = self.to_phys(x);
        let y0 = self.to_phys(y);
        let out = self.to_phys(size).max(1);
        for dy in 0..out {
            for dx in 0..out {
                let sx = dx * icon_draw::SIZE as i32 / out;
                let sy = dy * icon_draw::SIZE as i32 / out;
                let source = pixels[(sy as u32 * icon_draw::SIZE + sx as u32) as usize];
                if source[3] != 0 {
                    self.put(
                        x0 + dx,
                        y0 + dy,
                        ((source[0] as u32) << 16) | ((source[1] as u32) << 8) | source[2] as u32,
                    );
                }
            }
        }
    }

    fn text(&mut self, x: f64, y: f64, text: &str, color: u32, logical_size: f32) {
        let px = (logical_size as f64 * self.scale) as f32;
        let ascent = self
            .font
            .horizontal_line_metrics(px)
            .map(|metrics| metrics.ascent)
            .unwrap_or(px * 0.8);
        let baseline = self.to_phys(y) as f32 + ascent;
        let mut pen_x = self.to_phys(x) as f32;

        for ch in text.chars() {
            let (metrics, bitmap) = self.font.rasterize(ch, px);
            if metrics.width > 0 && metrics.height > 0 {
                let glyph_x = (pen_x + metrics.xmin as f32).round() as i32;
                let glyph_y =
                    (baseline - metrics.ymin as f32 - metrics.height as f32).round() as i32;
                for row in 0..metrics.height {
                    for col in 0..metrics.width {
                        let cover = bitmap[row * metrics.width + col];
                        if cover != 0 {
                            self.blend(glyph_x + col as i32, glyph_y + row as i32, color, cover);
                        }
                    }
                }
            }
            pen_x += metrics.advance_width;
        }
    }

    fn blend(&mut self, x: i32, y: i32, color: u32, alpha: u8) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return;
        }
        let index = y as usize * self.w + x as usize;
        let dst = self.buf[index];
        let alpha = alpha as u32;
        let inverse = 255 - alpha;
        let r = (((color >> 16) & 0xff) * alpha + ((dst >> 16) & 0xff) * inverse) / 255;
        let g = (((color >> 8) & 0xff) * alpha + ((dst >> 8) & 0xff) * inverse) / 255;
        let b = ((color & 0xff) * alpha + (dst & 0xff) * inverse) / 255;
        self.buf[index] = (r << 16) | (g << 8) | b;
    }
}

const fn rgb(color: Rgb) -> u32 {
    ((color.r as u32) << 16) | ((color.g as u32) << 8) | color.b as u32
}

#[cfg(windows)]
fn window_handle(window: &Window) -> Option<isize> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

#[cfg(not(windows))]
fn configure_no_activate(_window: &Window) {}

#[cfg(windows)]
fn configure_no_activate(window: &Window) {
    let Some(hwnd) = window_handle(window) else {
        return;
    };
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            style | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
        );
    }
}

#[cfg(windows)]
fn show_topmost_no_activate(window: &Window, x: i32, y: i32, width: u32, height: u32) {
    let Some(hwnd) = window_handle(window) else {
        return;
    };
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            x,
            y,
            width as i32,
            height as i32,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
}

#[cfg(windows)]
fn position_topmost_no_activate(window: &Window, x: i32, y: i32) {
    let Some(hwnd) = window_handle(window) else {
        return;
    };
    unsafe {
        SetWindowPos(hwnd, HWND_TOPMOST, x, y, 0, 0, SWP_NOACTIVATE | SWP_NOSIZE);
    }
}

#[cfg(windows)]
fn hide_window(window: &Window) {
    let Some(hwnd) = window_handle(window) else {
        return;
    };
    unsafe {
        ShowWindow(hwnd, SW_HIDE);
    }
}

#[cfg(windows)]
pub struct EscapeHotkey {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(windows)]
impl EscapeHotkey {
    pub fn register(
        proxy: winit::event_loop::EventLoopProxy<crate::tray::UserEvent>,
        generation: u64,
    ) -> Option<Self> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_stop = std::sync::Arc::clone(&stop);
        let thread = std::thread::spawn(move || unsafe {
            let registered = RegisterHotKey(0, HOTKEY_ID, MOD_NOREPEAT, VK_ESCAPE);
            let _ = ready_tx.send(registered != 0);
            if registered == 0 {
                return;
            }
            let mut message = WinMessage::default();
            while !worker_stop.load(std::sync::atomic::Ordering::Acquire) {
                while PeekMessageW(&mut message, 0, 0, 0, PM_REMOVE) != 0 {
                    if message.message == WM_HOTKEY && message.w_param == HOTKEY_ID as usize {
                        let _ = proxy.send_event(crate::tray::UserEvent::ToastDismiss(generation));
                        worker_stop.store(true, std::sync::atomic::Ordering::Release);
                        break;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            UnregisterHotKey(0, HOTKEY_ID);
        });
        let registered = ready_rx.recv().ok()?;
        if registered {
            Some(Self {
                stop,
                thread: Some(thread),
            })
        } else {
            let _ = thread.join();
            None
        }
    }
}

#[cfg(windows)]
impl Drop for EscapeHotkey {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(not(windows))]
pub struct EscapeHotkey;

#[cfg(not(windows))]
impl EscapeHotkey {
    pub fn register(
        _proxy: winit::event_loop::EventLoopProxy<crate::tray::UserEvent>,
        _generation: u64,
    ) -> Option<Self> {
        None
    }
}

#[cfg(windows)]
#[derive(Clone, Copy, Default)]
#[repr(C)]
struct WinRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[cfg(windows)]
#[repr(C)]
struct MonitorInfo {
    size: u32,
    monitor: WinRect,
    work: WinRect,
    flags: u32,
}

#[cfg(windows)]
#[derive(Default)]
#[repr(C)]
struct WinPoint {
    x: i32,
    y: i32,
}

#[cfg(windows)]
#[derive(Default)]
#[repr(C)]
struct WinMessage {
    hwnd: isize,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    point: WinPoint,
    private: u32,
}

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn GetForegroundWindow() -> isize;
    fn MonitorFromWindow(hwnd: isize, flags: u32) -> isize;
    fn GetMonitorInfoW(monitor: isize, info: *mut MonitorInfo) -> i32;
    fn GetWindowRect(hwnd: isize, rect: *mut WinRect) -> i32;
    fn GetDpiForWindow(hwnd: isize) -> u32;
    fn GetWindowLongPtrW(hwnd: isize, index: i32) -> isize;
    fn SetWindowLongPtrW(hwnd: isize, index: i32, value: isize) -> isize;
    fn SetWindowPos(
        hwnd: isize,
        insert_after: isize,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> i32;
    fn ShowWindow(hwnd: isize, command: i32) -> i32;
    fn RegisterHotKey(hwnd: isize, id: i32, modifiers: u32, key: u32) -> i32;
    fn UnregisterHotKey(hwnd: isize, id: i32) -> i32;
    fn PeekMessageW(message: *mut WinMessage, hwnd: isize, min: u32, max: u32, remove: u32) -> i32;
}

#[cfg(windows)]
const MONITOR_DEFAULTTOPRIMARY: u32 = 1;
#[cfg(windows)]
const GWL_EXSTYLE: i32 = -20;
#[cfg(windows)]
const WS_EX_TOOLWINDOW: isize = 0x0000_0080;
#[cfg(windows)]
const WS_EX_NOACTIVATE: isize = 0x0800_0000;
#[cfg(windows)]
const HWND_TOPMOST: isize = -1;
#[cfg(windows)]
const SWP_NOACTIVATE: u32 = 0x0010;
#[cfg(windows)]
const SWP_NOSIZE: u32 = 0x0001;
#[cfg(windows)]
const SWP_SHOWWINDOW: u32 = 0x0040;
#[cfg(windows)]
const SW_SHOWNOACTIVATE: i32 = 4;
#[cfg(windows)]
const SW_HIDE: i32 = 0;
#[cfg(windows)]
const MOD_NOREPEAT: u32 = 0x4000;
#[cfg(windows)]
const VK_ESCAPE: u32 = 0x1B;
#[cfg(windows)]
const HOTKEY_ID: i32 = 0x4453;
#[cfg(windows)]
const WM_HOTKEY: u32 = 0x0312;
#[cfg(windows)]
const PM_REMOVE: u32 = 0x0001;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_each_corner_inside_area() {
        let area = Rect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            corner_position(area, 360, 88, ToastPosition::TopLeft, 20),
            (-1900, 20)
        );
        assert_eq!(
            corner_position(area, 360, 88, ToastPosition::TopRight, 20),
            (-380, 20)
        );
        assert_eq!(
            corner_position(area, 360, 88, ToastPosition::BottomLeft, 20),
            (-1900, 972)
        );
        assert_eq!(
            corner_position(area, 360, 88, ToastPosition::BottomRight, 20),
            (-380, 972)
        );
    }
}
