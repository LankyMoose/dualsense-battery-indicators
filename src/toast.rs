//! Reusable, no-activate overlay window for controller notifications.

use crate::color::{BatterySpectrum, Rgb};
use crate::notify::NotifyEvent;
use crate::prefs::ToastPosition;
use crate::ui::layout::{self, LayoutTree, Rect};
use crate::ui::{self, Framebuffer};
use fontdue::Font;
use softbuffer::{Context, Surface};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Duration;
use taffy::prelude::{
    AlignItems, Dimension, Display, FlexDirection, JustifyContent, LengthPercentage, Size, Style,
};
use winit::dpi::LogicalSize;
#[cfg(not(windows))]
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, OwnedDisplayHandle};
use winit::keyboard::{Key, NamedKey};
#[cfg(windows)]
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowAttributes, WindowId, WindowLevel};

#[cfg(windows)]
use winit::platform::windows::{CornerPreference, WindowAttributesExtWindows};

const MAX_WIDTH: f64 = 360.0;
const PAD: f64 = layout::SPACE_3;
const RAIL_W: f64 = 3.0;
const ICON_SIZE: f64 = 36.0;
const HEADING_SIZE: f32 = 13.0;
const BODY_SIZE: f32 = 12.0;
const TEXT_GAP: f64 = layout::SPACE_1;
pub const SLIDE_DURATION: Duration = Duration::from_millis(250);
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
    position: Option<ToastPosition>,
    layout: Option<ToastLayout>,
}

#[derive(Clone)]
struct ToastLayout {
    root: Rect,
    rail: Rect,
    icon: Rect,
    heading: Rect,
    body: Rect,
}

impl ToastLayout {
    fn compute(font: &Font, message: &ToastMessage) -> Result<Self, String> {
        let heading_w = text_advance(font, &message.heading, HEADING_SIZE);
        let body_w = text_advance(font, &message.body, BODY_SIZE);
        let text_w = heading_w.max(body_w);
        let width = (RAIL_W + PAD + ICON_SIZE + layout::SPACE_2 + text_w + PAD)
            .clamp(RAIL_W + PAD + ICON_SIZE + layout::SPACE_2 + PAD, MAX_WIDTH);

        let mut tree = LayoutTree::new();
        let rail = tree.leaf(toast_fixed(Some(RAIL_W), None, 0.0))?;
        let icon = tree.leaf(toast_fixed(Some(ICON_SIZE), Some(ICON_SIZE), 0.0))?;
        let heading = tree.text(&message.heading, HEADING_SIZE, toast_text_fill())?;
        let body = tree.text(&message.body, BODY_SIZE, toast_text_fill())?;
        let text = tree.container(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                justify_content: Some(JustifyContent::CENTER),
                gap: Size {
                    width: LengthPercentage::length(0.0),
                    height: LengthPercentage::length(TEXT_GAP as f32),
                },
                flex_grow: 1.0,
                flex_basis: Dimension::length(0.0),
                min_size: Size {
                    width: Dimension::length(0.0),
                    height: Dimension::auto(),
                },
                ..Default::default()
            },
            &[heading, body],
        )?;
        let content = tree.container(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: Some(AlignItems::CENTER),
                flex_grow: 1.0,
                flex_basis: Dimension::length(0.0),
                min_size: Size {
                    width: Dimension::length(0.0),
                    height: Dimension::auto(),
                },
                padding: layout::points(PAD),
                gap: Size {
                    width: LengthPercentage::length(layout::SPACE_2 as f32),
                    height: LengthPercentage::length(0.0),
                },
                ..Default::default()
            },
            &[icon, text],
        )?;
        let root = tree.container(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: Some(AlignItems::STRETCH),
                size: Size {
                    width: Dimension::length(width as f32),
                    height: Dimension::auto(),
                },
                ..Default::default()
            },
            &[rail, content],
        )?;
        tree.compute(root, width, None, font)?;
        Ok(Self {
            root: tree.rect(root)?,
            rail: tree.rect(rail)?,
            icon: tree.rect(icon)?,
            heading: tree.rect(heading)?,
            body: tree.rect(body)?,
        })
    }
}

fn text_advance(font: &Font, text: &str, size: f32) -> f64 {
    text.chars()
        .map(|character| font.metrics(character, size).advance_width as f64)
        .sum()
}

fn toast_fixed(width: Option<f64>, height: Option<f64>, grow: f32) -> Style {
    Style {
        size: Size {
            width: width.map_or(Dimension::auto(), |value| Dimension::length(value as f32)),
            height: height.map_or(Dimension::auto(), |value| Dimension::length(value as f32)),
        },
        flex_grow: grow,
        flex_shrink: 0.0,
        ..Default::default()
    }
}

fn toast_text_fill() -> Style {
    Style {
        size: Size {
            width: Dimension::percent(1.0),
            height: Dimension::auto(),
        },
        min_size: Size {
            width: Dimension::length(0.0),
            height: Dimension::auto(),
        },
        flex_shrink: 1.0,
        ..Default::default()
    }
}

impl ToastWindow {
    pub fn open(event_loop: &ActiveEventLoop, display: OwnedDisplayHandle) -> Result<Self, String> {
        let attrs = WindowAttributes::default()
            .with_title("DualSense notification")
            .with_inner_size(LogicalSize::new(MAX_WIDTH, ICON_SIZE + PAD * 2.0))
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
            font: ui::load_system_ui_font()?,
            message: None,
            placement: None,
            position: None,
            layout: None,
        })
    }

    pub fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub fn show(&mut self, message: ToastMessage, position: ToastPosition) {
        let layout = match ToastLayout::compute(&self.font, &message) {
            Ok(layout) => layout,
            Err(err) => {
                crate::app_log::warn(format!("toast layout failed: {err}"));
                return;
            }
        };
        self.message = Some(message);
        self.layout = Some(layout);
        self.position = Some(position);
        self.reposition(position, false);
        self.window.request_redraw();
    }

    fn reposition(&mut self, position: ToastPosition, at_target: bool) {
        let target = target_area(&self.window);
        let logical_size = self
            .layout
            .as_ref()
            .map(|layout| (layout.root.w, layout.root.h))
            .unwrap_or((MAX_WIDTH, ICON_SIZE + PAD * 2.0));
        let (placement, width, height) = scaled_placement(target, logical_size, position);
        self.placement = Some(placement);
        let x = placement.x;
        let y = placement.target_y;
        let outside_y = placement.outside_y;
        let initial_y = if at_target { y } else { outside_y };

        #[cfg(windows)]
        show_topmost_no_activate(&self.window, x, initial_y, width, height);

        #[cfg(not(windows))]
        {
            let _ = self
                .window
                .request_inner_size(PhysicalSize::new(width, height));
            self.window
                .set_outer_position(PhysicalPosition::new(x, initial_y));
            self.window.set_visible(true);
        }
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
        self.position = None;
        self.layout = None;
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
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(position) = self.position {
                    self.reposition(position, true);
                }
                self.window.request_redraw();
            }
            _ => {}
        }
        false
    }

    fn paint(&mut self) -> Result<(), String> {
        let Some(message) = self.message.as_ref() else {
            return Ok(());
        };
        let Some(layout) = self.layout.clone() else {
            return Ok(());
        };
        let heading = message.heading.clone();
        let body = message.body.clone();
        let accent = message.accent;
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
        let mut fb = Framebuffer::new(
            &mut buffer,
            width.get() as usize,
            height.get() as usize,
            scale,
            &self.font,
        );

        fb.clear(ui::BG);
        let rail_inset = layout::SPACE_1;
        let rail = Rect::new(
            layout.rail.x,
            layout.rail.y + rail_inset,
            layout.rail.w,
            (layout.rail.h - rail_inset * 2.0).max(0.0),
        );
        fb.round_rect(
            (rail.x, rail.y, rail.w, rail.h),
            (rail.w / 2.0).max(1.0),
            ui::rgb_of(accent),
            None,
        );
        fb.icon(
            layout.icon.x,
            layout.icon.y,
            layout.icon.w.min(layout.icon.h),
            accent,
        );
        fb.text_in_rect(
            layout.heading,
            &heading,
            ui::INK,
            HEADING_SIZE,
            ui::HorizontalAlign::Left,
            ui::VerticalAlign::Center,
        );
        fb.text_in_rect(
            layout.body,
            &body,
            ui::MUTED,
            BODY_SIZE,
            ui::HorizontalAlign::Left,
            ui::VerticalAlign::Center,
        );
        buffer.present().map_err(|e| format!("present: {e}"))
    }
}

#[derive(Clone, Copy)]
struct ScreenRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy)]
struct TargetArea {
    rect: ScreenRect,
    scale: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ToastPlacement {
    x: i32,
    outside_y: i32,
    target_y: i32,
}

fn scaled_placement(
    target: TargetArea,
    logical_size: (f64, f64),
    position: ToastPosition,
) -> (ToastPlacement, u32, u32) {
    let scale = target.scale.max(1.0);
    let width = (logical_size.0 * scale).round() as u32;
    let height = (logical_size.1 * scale).round() as u32;
    let margin = (layout::SPACE_5 * scale).round() as i32;
    let (x, target_y) = corner_position(target.rect, width, height, position, margin);
    let outside_y = match position {
        ToastPosition::TopLeft | ToastPosition::TopCenter | ToastPosition::TopRight => {
            target.rect.y - height as i32
        }
        ToastPosition::BottomLeft | ToastPosition::BottomCenter | ToastPosition::BottomRight => {
            target.rect.y + target.rect.height as i32
        }
    };
    (
        ToastPlacement {
            x,
            outside_y,
            target_y,
        },
        width,
        height,
    )
}

fn ease_out_cubic(progress: f32) -> f32 {
    1.0 - (1.0 - progress).powi(3)
}

fn lerp_i32(from: i32, to: i32, progress: f32) -> i32 {
    (from as f32 + (to - from) as f32 * progress).round() as i32
}

fn corner_position(
    area: ScreenRect,
    width: u32,
    height: u32,
    position: ToastPosition,
    margin: i32,
) -> (i32, i32) {
    let left = area.x + margin;
    let right = area.x + area.width as i32 - width as i32 - margin;
    let center = area.x + (area.width as i32 - width as i32) / 2;
    let top = area.y + margin;
    let bottom = area.y + area.height as i32 - height as i32 - margin;
    match position {
        ToastPosition::TopLeft => (left, top),
        ToastPosition::TopCenter => (center, top),
        ToastPosition::TopRight => (right, top),
        ToastPosition::BottomLeft => (left, bottom),
        ToastPosition::BottomCenter => (center, bottom),
        ToastPosition::BottomRight => (right, bottom),
    }
}

#[cfg(not(windows))]
fn target_area(window: &Window) -> TargetArea {
    let monitor = window.primary_monitor();
    match monitor {
        Some(monitor) => {
            let position = monitor.position();
            let size = monitor.size();
            TargetArea {
                rect: ScreenRect {
                    x: position.x,
                    y: position.y,
                    width: size.width,
                    height: size.height,
                },
                scale: monitor.scale_factor(),
            }
        }
        None => TargetArea {
            rect: ScreenRect {
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
    let monitor = unsafe {
        MonitorFromPoint(
            WinPoint { x: 0, y: 0 },
            MONITOR_DEFAULTTOPRIMARY,
        )
    };
    let mut info = MonitorInfo {
        size: std::mem::size_of::<MonitorInfo>() as u32,
        monitor: WinRect::default(),
        work: WinRect::default(),
        flags: 0,
    };
    let got_info = unsafe { GetMonitorInfoW(monitor, &mut info) } != 0;
    if !got_info {
        return TargetArea {
            rect: ScreenRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            scale: 1.0,
        };
    }

    let foreground = unsafe { GetForegroundWindow() };
    let mut foreground_rect = WinRect::default();
    let got_foreground =
        foreground != 0 && unsafe { GetWindowRect(foreground, &mut foreground_rect) } != 0;
    let fullscreen = got_foreground
        && foreground_rect.left <= info.monitor.left + 2
        && foreground_rect.top <= info.monitor.top + 2
        && foreground_rect.right >= info.monitor.right - 2
        && foreground_rect.bottom >= info.monitor.bottom - 2;
    let area = if fullscreen { info.monitor } else { info.work };
    let dpi = primary_monitor_dpi(monitor);
    TargetArea {
        rect: ScreenRect {
            x: area.left,
            y: area.top,
            width: (area.right - area.left).max(1) as u32,
            height: (area.bottom - area.top).max(1) as u32,
        },
        scale: dpi.max(96) as f64 / 96.0,
    }
}

#[cfg(windows)]
fn primary_monitor_dpi(monitor: isize) -> u32 {
    let mut dpi_x = 0u32;
    let mut dpi_y = 0u32;
    let result = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    if result == 0 && dpi_x > 0 {
        dpi_x
    } else {
        96
    }
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
    fn MonitorFromPoint(point: WinPoint, flags: u32) -> isize;
    fn GetMonitorInfoW(monitor: isize, info: *mut MonitorInfo) -> i32;
    fn GetWindowRect(hwnd: isize, rect: *mut WinRect) -> i32;
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
#[link(name = "shcore")]
unsafe extern "system" {
    fn GetDpiForMonitor(monitor: isize, dpi_type: u32, dpi_x: *mut u32, dpi_y: *mut u32) -> i32;
}

#[cfg(windows)]
const MONITOR_DEFAULTTOPRIMARY: u32 = 1;
#[cfg(windows)]
const MDT_EFFECTIVE_DPI: u32 = 0;
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
        let area = ScreenRect {
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
            corner_position(area, 360, 88, ToastPosition::TopCenter, 20),
            (-1140, 20)
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
            corner_position(area, 360, 88, ToastPosition::BottomCenter, 20),
            (-1140, 972)
        );
        assert_eq!(
            corner_position(area, 360, 88, ToastPosition::BottomRight, 20),
            (-380, 972)
        );
    }

    #[test]
    fn toast_layout_hugs_content_size() {
        let font = ui::load_system_ui_font().unwrap();
        let short = ToastLayout::compute(
            &font,
            &ToastMessage {
                heading: "Pad".into(),
                body: "low".into(),
                accent: Rgb::new(65, 65, 251),
            },
        )
        .unwrap();
        let long = ToastLayout::compute(
            &font,
            &ToastMessage {
                heading: "DualSense Wireless Controller (Bluetooth)".into(),
                body: "connected — 100%".into(),
                accent: Rgb::new(65, 65, 251),
            },
        )
        .unwrap();
        assert!(short.root.w < long.root.w);
        assert!(short.root.w < MAX_WIDTH);
        assert!(long.root.w <= MAX_WIDTH + 0.5);
        // Height is content-driven (icon + uniform padding), same for both.
        let expected_h = ICON_SIZE + PAD * 2.0;
        assert!((short.root.h - expected_h).abs() < 1.0, "short {:?}", short.root);
        assert!((long.root.h - expected_h).abs() < 1.0, "long {:?}", long.root);
    }

    #[test]
    fn toast_layout_keeps_content_inside_fixed_viewport() {
        let font = ui::load_system_ui_font().unwrap();
        let message = ToastMessage {
            heading: "A very long controller notification heading".into(),
            body: "A very long controller notification body that must be constrained".into(),
            accent: Rgb::new(65, 65, 251),
        };
        let layout = ToastLayout::compute(&font, &message).unwrap();
        assert!(layout.root.w <= MAX_WIDTH + 0.5);
        for rect in [layout.rail, layout.icon, layout.heading, layout.body] {
            assert!(rect.x >= layout.root.x);
            assert!(rect.y >= layout.root.y);
            assert!(
                rect.right() <= layout.root.right() + 0.5,
                "{rect:?} exceeds {:?}",
                layout.root
            );
            assert!(rect.bottom() <= layout.root.bottom() + 0.5);
        }
    }

    #[test]
    fn dpi_scales_toast_size_margin_and_placement() {
        let target = TargetArea {
            rect: ScreenRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            scale: 1.5,
        };
        let (placement, width, height) =
            scaled_placement(target, (240.0, ICON_SIZE + PAD * 2.0), ToastPosition::TopRight);
        assert_eq!((width, height), (360, 90));
        assert_eq!(
            placement,
            ToastPlacement {
                x: 1530,
                outside_y: -90,
                target_y: 30,
            }
        );
    }
}
