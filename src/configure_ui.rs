//! Softbuffer Configure window: lightbar spectrum editor + native menu bar.

use crate::app_meta::DISPLAY_NAME;
use crate::color::{BatterySpectrum, Rgb, hsv_to_rgb};
#[cfg(feature = "dev-emulate")]
use crate::emulate::Preset;
use fontdue::Font;
use softbuffer::{Context, Surface};
use std::num::NonZeroU32;
use std::rc::Rc;
#[cfg(feature = "dev-emulate")]
use tray_icon::menu::MenuItem;
use tray_icon::menu::{CheckMenuItem, Menu, Submenu};
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, OwnedDisplayHandle};
#[cfg(windows)]
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowAttributes, WindowId};

/// ~⅔ of the previous 460px width.
const WIN_W: f64 = 308.0;
const PAD: f64 = 12.0;
const CONTENT_W: f64 = WIN_W - PAD * 2.0;
const GAP: f64 = 8.0;
const HUE_W: f64 = 14.0;
/// Saturation/value square is 16:9 inside the picker panel.
const SV_W: f64 = CONTENT_W - 24.0 - HUE_W - GAP;
const SV_H: f64 = SV_W * 9.0 / 16.0;
const PREVIEW_H: f64 = 68.0;
const STOP_H: f64 = 102.0;
const PICKER_HEADER: f64 = 28.0;
const PICKER_H: f64 = 12.0 + PICKER_HEADER + GAP + SV_H + 12.0;
const BTN_H: f64 = 32.0;
const WIN_H: f64 = 18.0 // top
    + 44.0 // title + subtitle block
    + GAP
    + PREVIEW_H
    + GAP
    + STOP_H
    + GAP
    + PICKER_H
    + GAP
    + BTN_H
    + 16.0; // bottom padding
/// Logical pixel sizes for UI text (scaled by the window DPI factor).
const FONT_TITLE: f32 = 18.0;
const FONT_BODY: f32 = 12.0;
const FONT_SMALL: f32 = 11.0;

const BG: u32 = rgb_u32(245, 246, 248);
const PANEL: u32 = rgb_u32(255, 255, 255);
const INK: u32 = rgb_u32(28, 32, 40);
const MUTED: u32 = rgb_u32(100, 108, 120);
const LINE: u32 = rgb_u32(210, 216, 224);
const ACCENT: u32 = rgb_u32(0, 90, 255);
const BTN_BG: u32 = rgb_u32(236, 239, 244);
const BTN_BG_HOT: u32 = rgb_u32(0, 90, 255);
const BTN_INK_HOT: u32 = rgb_u32(255, 255, 255);

pub const NOTIFY_LOW_ID: &str = "cfg:notify_low";
pub const NOTIFY_CHARGED_ID: &str = "cfg:notify_charged";
pub const NOTIFY_CONNECT_ID: &str = "cfg:notify_connect";
#[cfg(windows)]
pub const AUTOSTART_ID: &str = "cfg:autostart";

#[derive(Debug, Clone, Copy)]
pub struct ConfigureSettings {
    pub notify_low: bool,
    pub notify_charged: bool,
    pub notify_connect: bool,
    #[cfg(windows)]
    pub autostart: bool,
    pub show_developer: bool,
}

/// Native window menu bar + check items we keep alive for state sync.
pub struct ConfigureMenuBar {
    pub menu: Menu,
    pub notify_low: CheckMenuItem,
    pub notify_charged: CheckMenuItem,
    pub notify_connect: CheckMenuItem,
    #[cfg(windows)]
    pub autostart: CheckMenuItem,
}

impl ConfigureMenuBar {
    pub fn build(settings: ConfigureSettings) -> Result<Self, String> {
        let menu = Menu::new();

        let settings_menu = Submenu::new("Settings", true);
        let notifications = Submenu::new("Notifications", true);
        let notify_connect = CheckMenuItem::with_id(
            NOTIFY_CONNECT_ID,
            "On connect",
            true,
            settings.notify_connect,
            None,
        );
        let notify_low = CheckMenuItem::with_id(
            NOTIFY_LOW_ID,
            "When low",
            true,
            settings.notify_low,
            None,
        );
        let notify_charged = CheckMenuItem::with_id(
            NOTIFY_CHARGED_ID,
            "When charged",
            true,
            settings.notify_charged,
            None,
        );
        notifications
            .append(&notify_connect)
            .map_err(|e| format!("menu append: {e}"))?;
        notifications
            .append(&notify_low)
            .map_err(|e| format!("menu append: {e}"))?;
        notifications
            .append(&notify_charged)
            .map_err(|e| format!("menu append: {e}"))?;
        settings_menu
            .append(&notifications)
            .map_err(|e| format!("menu append: {e}"))?;

        #[cfg(windows)]
        let autostart = {
            let item = CheckMenuItem::with_id(
                AUTOSTART_ID,
                "Start with Windows",
                true,
                settings.autostart,
                None,
            );
            settings_menu
                .append(&item)
                .map_err(|e| format!("menu append: {e}"))?;
            item
        };

        menu.append(&settings_menu)
            .map_err(|e| format!("menu append: {e}"))?;

        #[cfg(feature = "dev-emulate")]
        if settings.show_developer {
            let developer = Submenu::new("Developer", true);
            for preset in Preset::ALL {
                let item = MenuItem::with_id(preset.menu_id(), preset.menu_label(), true, None);
                developer
                    .append(&item)
                    .map_err(|e| format!("menu append: {e}"))?;
            }
            menu.append(&developer)
                .map_err(|e| format!("menu append: {e}"))?;
        }
        #[cfg(not(feature = "dev-emulate"))]
        let _ = settings.show_developer;

        Ok(Self {
            menu,
            notify_low,
            notify_charged,
            notify_connect,
            #[cfg(windows)]
            autostart,
        })
    }

    pub fn sync_checks(&self, settings: ConfigureSettings) {
        self.notify_low.set_checked(settings.notify_low);
        self.notify_charged.set_checked(settings.notify_charged);
        self.notify_connect.set_checked(settings.notify_connect);
        #[cfg(windows)]
        self.autostart.set_checked(settings.autostart);
    }

    pub fn attach(&self, window: &Window) -> Result<(), String> {
        #[cfg(windows)]
        {
            let hwnd = win32_hwnd(window).ok_or_else(|| "missing Win32 hwnd".to_string())?;
            // SAFETY: hwnd comes from a live winit Window.
            unsafe {
                self.menu
                    .init_for_hwnd(hwnd)
                    .map_err(|e| format!("init menu for hwnd: {e}"))?;
            }
            Ok(())
        }

        #[cfg(target_os = "macos")]
        {
            let _ = window;
            self.menu.init_for_nsapp();
            Ok(())
        }

        #[cfg(all(unix, not(target_os = "macos")))]
        {
            // winit windows are not GTK windows, so muda cannot attach a menubar here.
            let _ = window;
            Ok(())
        }
    }

    pub fn detach(&self, window: &Window) {
        #[cfg(windows)]
        {
            if let Some(hwnd) = win32_hwnd(window) {
                // SAFETY: hwnd came from the same Window we attached to.
                let _ = unsafe { self.menu.remove_for_hwnd(hwnd) };
            }
        }

        #[cfg(target_os = "macos")]
        {
            let _ = window;
            self.menu.remove_for_nsapp();
        }

        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let _ = window;
        }
    }
}

#[cfg(windows)]
fn win32_hwnd(window: &Window) -> Option<isize> {
    let handle = window.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(win32) => Some(win32.hwnd.get()),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Full,
    Mid,
    Empty,
}

impl Stop {
    fn label(self) -> &'static str {
        match self {
            Self::Full => "Full",
            Self::Mid => "Mid",
            Self::Empty => "Empty",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Full => "100%",
            Self::Mid => "50%",
            Self::Empty => "0%",
        }
    }

    fn all() -> [Self; 3] {
        [Self::Full, Self::Mid, Self::Empty]
    }
}

#[derive(Debug, Clone, Copy)]
enum DragKind {
    Sv,
    Hue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonId {
    Reset,
    Apply,
}

pub enum ConfigureAction {
    None,
    ApplySpectrum(BatterySpectrum),
    Closed,
}

pub struct ConfigureWindow {
    window: Rc<Window>,
    surface: Surface<OwnedDisplayHandle, Rc<Window>>,
    menu_bar: ConfigureMenuBar,
    font: Font,
    draft: BatterySpectrum,
    selected: Stop,
    hue: f32,
    sat: f32,
    val: f32,
    drag: Option<DragKind>,
    cursor: Option<(f64, f64)>,
    dirty: bool,
}

fn load_system_ui_font() -> Result<Font, String> {
    use font_kit::family_name::FamilyName;
    use font_kit::properties::Properties;
    use font_kit::source::SystemSource;

    let handle = SystemSource::new()
        .select_best_match(&[FamilyName::SansSerif], &Properties::new())
        .map_err(|e| format!("select system UI font: {e}"))?;
    let font = handle
        .load()
        .map_err(|e| format!("load system UI font: {e}"))?;
    let data = font
        .copy_font_data()
        .ok_or_else(|| "system UI font has no accessible font data".to_string())?;
    Font::from_bytes(data.as_slice(), fontdue::FontSettings::default())
        .map_err(|e| format!("parse system UI font: {e}"))
}

impl ConfigureWindow {
    pub fn open(
        event_loop: &ActiveEventLoop,
        display: OwnedDisplayHandle,
        initial: BatterySpectrum,
        settings: ConfigureSettings,
    ) -> Result<Self, String> {
        let menu_bar = ConfigureMenuBar::build(settings)?;
        let font = load_system_ui_font()?;

        let attrs = WindowAttributes::default()
            .with_title(format!("{DISPLAY_NAME} — Configure"))
            .with_inner_size(LogicalSize::new(WIN_W, WIN_H))
            .with_resizable(false);

        let window = Rc::new(
            event_loop
                .create_window(attrs)
                .map_err(|e| format!("create configure window: {e}"))?,
        );

        menu_bar.attach(&window)?;

        let context = Context::new(display).map_err(|e| format!("softbuffer context: {e}"))?;
        let surface = Surface::new(&context, window.clone())
            .map_err(|e| format!("softbuffer surface: {e}"))?;

        let (hue, sat, val) = initial.full.to_hsv();
        let editor = Self {
            window,
            surface,
            menu_bar,
            font,
            draft: initial,
            selected: Stop::Full,
            hue,
            sat,
            val,
            drag: None,
            cursor: None,
            dirty: false,
        };
        editor.window.request_redraw();
        Ok(editor)
    }

    pub fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub fn menu_bar(&self) -> &ConfigureMenuBar {
        &self.menu_bar
    }

    pub fn focus(&self) {
        self.window.set_visible(true);
        self.window.focus_window();
        self.window.request_redraw();
    }

    pub fn sync_settings(&mut self, settings: ConfigureSettings) {
        self.menu_bar.sync_checks(settings);
    }

    pub fn handle(&mut self, event: &WindowEvent) -> ConfigureAction {
        match event {
            WindowEvent::CloseRequested => {
                self.menu_bar.detach(&self.window);
                return ConfigureAction::Closed;
            }
            WindowEvent::RedrawRequested => {
                if let Err(err) = self.paint() {
                    crate::app_log::warn(format!("configure UI paint failed: {err}"));
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Some(self.to_logical(*position));
                if self.drag.is_some() {
                    self.apply_drag();
                }
                self.window.request_redraw();
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor = None;
                self.window.request_redraw();
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => match state {
                ElementState::Pressed => {
                    if let Some(action) = self.on_press() {
                        return action;
                    }
                    self.window.request_redraw();
                }
                ElementState::Released => {
                    self.drag = None;
                }
            },
            WindowEvent::ScaleFactorChanged { .. } => {
                self.window.request_redraw();
            }
            _ => {}
        }
        ConfigureAction::None
    }

    fn on_press(&mut self) -> Option<ConfigureAction> {
        let (x, y) = self.cursor?;

        for stop in Stop::all() {
            if hit(stop_swatch_rect(stop), x, y) || hit(stop_label_rect(stop), x, y) {
                self.select_stop(stop);
                return None;
            }
        }

        if hit(sv_rect(), x, y) {
            self.drag = Some(DragKind::Sv);
            self.apply_drag();
            return None;
        }
        if hit(hue_rect(), x, y) {
            self.drag = Some(DragKind::Hue);
            self.apply_drag();
            return None;
        }

        if hit(button_rect(ButtonId::Reset), x, y) {
            self.draft = BatterySpectrum::DEFAULT;
            self.select_stop(self.selected);
            self.dirty = true;
            return None;
        }
        if hit(button_rect(ButtonId::Apply), x, y) {
            if !self.dirty {
                return None;
            }
            self.dirty = false;
            return Some(ConfigureAction::ApplySpectrum(self.draft));
        }

        None
    }

    fn select_stop(&mut self, stop: Stop) {
        self.selected = stop;
        let color = self.color_of(stop);
        let (h, s, v) = color.to_hsv();
        self.hue = h;
        self.sat = s;
        self.val = v;
    }

    fn color_of(&self, stop: Stop) -> Rgb {
        match stop {
            Stop::Full => self.draft.full,
            Stop::Mid => self.draft.mid,
            Stop::Empty => self.draft.empty,
        }
    }

    fn set_color_of(&mut self, stop: Stop, color: Rgb) {
        match stop {
            Stop::Full => self.draft.full = color,
            Stop::Mid => self.draft.mid = color,
            Stop::Empty => self.draft.empty = color,
        }
        self.dirty = true;
    }

    fn apply_drag(&mut self) {
        let Some((x, y)) = self.cursor else {
            return;
        };
        match self.drag {
            Some(DragKind::Sv) => {
                let r = sv_rect();
                self.sat = ((x - r.x) / r.w).clamp(0.0, 1.0) as f32;
                self.val = (1.0 - (y - r.y) / r.h).clamp(0.0, 1.0) as f32;
                let color = hsv_to_rgb(self.hue, self.sat, self.val);
                self.set_color_of(self.selected, color);
            }
            Some(DragKind::Hue) => {
                let r = hue_rect();
                self.hue = ((y - r.y) / r.h).clamp(0.0, 1.0) as f32 * 360.0;
                let color = hsv_to_rgb(self.hue, self.sat, self.val);
                self.set_color_of(self.selected, color);
            }
            None => {}
        }
    }

    fn to_logical(&self, pos: PhysicalPosition<f64>) -> (f64, f64) {
        let scale = self.window.scale_factor();
        (pos.x / scale, pos.y / scale)
    }

    fn paint(&mut self) -> Result<(), String> {
        let size = self.window.inner_size();
        let width = NonZeroU32::new(size.width.max(1)).unwrap();
        let height = NonZeroU32::new(size.height.max(1)).unwrap();
        self.surface
            .resize(width, height)
            .map_err(|e| format!("resize: {e}"))?;

        let draft = self.draft;
        let selected = self.selected;
        let hue = self.hue;
        let sat = self.sat;
        let val = self.val;
        let cursor = self.cursor;
        let dirty = self.dirty;
        let scale = self.window.scale_factor();
        let stop_colors = [
            (Stop::Full, draft.full),
            (Stop::Mid, draft.mid),
            (Stop::Empty, draft.empty),
        ];

        let mut buffer = self
            .surface
            .buffer_mut()
            .map_err(|e| format!("buffer: {e}"))?;

        {
            let w = width.get() as usize;
            let h = height.get() as usize;
            let mut fb = Framebuffer {
                buf: &mut buffer,
                w,
                h,
                scale,
                font: &self.font,
            };
            fb.clear(BG);

            let preview = preview_rect();
            let picker = picker_rect();

            fb.text(PAD, 14.0, "Lightbar colors", INK, FONT_TITLE);
            fb.text(
                PAD,
                36.0,
                "Blend three colors across battery level",
                MUTED,
                FONT_BODY,
            );

            fb.round_rect(
                preview.x,
                preview.y,
                preview.w,
                preview.h,
                PANEL,
                Some(LINE),
            );
            fb.text(
                preview.x + 12.0,
                preview.y + 10.0,
                "Preview",
                MUTED,
                FONT_SMALL,
            );
            draw_spectrum_bar(
                &mut fb,
                preview.x + 12.0,
                preview.y + 30.0,
                preview.w - 24.0,
                26.0,
                draft,
            );

            for (stop, color) in stop_colors {
                let is_selected = stop == selected;
                let rect = stop_card_rect(stop);
                fb.round_rect(
                    rect.x,
                    rect.y,
                    rect.w,
                    rect.h,
                    PANEL,
                    Some(if is_selected { ACCENT } else { LINE }),
                );
                let sw = stop_swatch_rect(stop);
                fb.round_rect(sw.x, sw.y, sw.w, sw.h, rgb_of(color), Some(LINE));
                fb.text(rect.x + 8.0, rect.y + 52.0, stop.label(), INK, FONT_BODY);
                fb.text(rect.x + 8.0, rect.y + 68.0, stop.hint(), MUTED, FONT_SMALL);
                fb.text(
                    rect.x + 8.0,
                    rect.y + 84.0,
                    &color.to_hex(),
                    MUTED,
                    FONT_SMALL,
                );
            }

            fb.round_rect(picker.x, picker.y, picker.w, picker.h, PANEL, Some(LINE));
            fb.text(
                picker.x + 12.0,
                picker.y + 10.0,
                &format!("Editing {} ({})", selected.label(), selected.hint()),
                INK,
                FONT_BODY,
            );

            draw_sv_square(&mut fb, sv_rect(), hue, sat, val);
            draw_hue_strip(&mut fb, hue_rect(), hue);

            let reset_hot = cursor.is_some_and(|(x, y)| hit(button_rect(ButtonId::Reset), x, y));
            let apply_hot = cursor.is_some_and(|(x, y)| hit(button_rect(ButtonId::Apply), x, y));
            draw_button(
                &mut fb,
                button_rect(ButtonId::Reset),
                "Reset defaults",
                reset_hot,
                false,
                true,
            );
            draw_button(
                &mut fb,
                button_rect(ButtonId::Apply),
                "Apply",
                apply_hot,
                true,
                dirty,
            );
        }

        buffer.present().map_err(|e| format!("present: {e}"))?;
        Ok(())
    }
}

impl Drop for ConfigureWindow {
    fn drop(&mut self) {
        self.menu_bar.detach(&self.window);
    }
}

#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

fn hit(r: Rect, x: f64, y: f64) -> bool {
    x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

fn preview_rect() -> Rect {
    Rect {
        x: PAD,
        y: 58.0,
        w: CONTENT_W,
        h: PREVIEW_H,
    }
}

fn stop_card_rect(stop: Stop) -> Rect {
    let i = match stop {
        Stop::Full => 0,
        Stop::Mid => 1,
        Stop::Empty => 2,
    };
    let card_w = (CONTENT_W - GAP * 2.0) / 3.0;
    Rect {
        x: PAD + i as f64 * (card_w + GAP),
        y: preview_rect().y + preview_rect().h + GAP,
        w: card_w,
        h: STOP_H,
    }
}

fn stop_swatch_rect(stop: Stop) -> Rect {
    let c = stop_card_rect(stop);
    Rect {
        x: c.x + 8.0,
        y: c.y + 10.0,
        w: 36.0,
        h: 36.0,
    }
}

fn stop_label_rect(stop: Stop) -> Rect {
    stop_card_rect(stop)
}

fn picker_rect() -> Rect {
    let stops_bottom = stop_card_rect(Stop::Full).y + STOP_H;
    Rect {
        x: PAD,
        y: stops_bottom + GAP,
        w: CONTENT_W,
        h: PICKER_H,
    }
}

fn sv_rect() -> Rect {
    let p = picker_rect();
    Rect {
        x: p.x + 12.0,
        y: p.y + 12.0 + PICKER_HEADER,
        w: SV_W,
        h: SV_H,
    }
}

fn hue_rect() -> Rect {
    let sv = sv_rect();
    Rect {
        x: sv.x + sv.w + GAP,
        y: sv.y,
        w: HUE_W,
        h: sv.h,
    }
}

fn button_rect(id: ButtonId) -> Rect {
    let y = picker_rect().y + picker_rect().h + GAP;
    match id {
        ButtonId::Reset => Rect {
            x: PAD,
            y,
            w: 120.0,
            h: BTN_H,
        },
        ButtonId::Apply => Rect {
            x: PAD + CONTENT_W - 88.0,
            y,
            w: 88.0,
            h: BTN_H,
        },
    }
}

const fn rgb_u32(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

fn rgb_of(c: Rgb) -> u32 {
    rgb_u32(c.r, c.g, c.b)
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

    fn put_phys(&mut self, x: i32, y: i32, color: u32) {
        if x < 0 || y < 0 {
            return;
        }
        let x = x as usize;
        let y = y as usize;
        if x < self.w && y < self.h {
            self.buf[y * self.w + x] = color;
        }
    }

    fn fill_rect_phys(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u32) {
        let x0 = x0.max(0);
        let y0 = y0.max(0);
        let x1 = x1.min(self.w as i32);
        let y1 = y1.min(self.h as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                self.put_phys(x, y, color);
            }
        }
    }

    fn to_phys(&self, v: f64) -> i32 {
        (v * self.scale).round() as i32
    }

    fn round_rect(&mut self, x: f64, y: f64, w: f64, h: f64, fill: u32, border: Option<u32>) {
        let x0 = self.to_phys(x);
        let y0 = self.to_phys(y);
        let x1 = self.to_phys(x + w);
        let y1 = self.to_phys(y + h);
        let radius = self.to_phys(8.0).max(2);
        for py in y0..y1 {
            for px in x0..x1 {
                if inside_rounded(px, py, x0, y0, x1, y1, radius) {
                    self.put_phys(px, py, fill);
                }
            }
        }
        if let Some(border) = border {
            let t = self.to_phys(1.5).max(1);
            for py in y0..y1 {
                for px in x0..x1 {
                    if inside_rounded(px, py, x0, y0, x1, y1, radius)
                        && !inside_rounded(
                            px,
                            py,
                            x0 + t,
                            y0 + t,
                            x1 - t,
                            y1 - t,
                            (radius - t).max(0),
                        )
                    {
                        self.put_phys(px, py, border);
                    }
                }
            }
        }
    }

    fn text_width(&self, text: &str, size_logical: f32) -> f64 {
        let px = (size_logical as f64 * self.scale) as f32;
        text.chars()
            .map(|ch| self.font.metrics(ch, px).advance_width as f64)
            .sum::<f64>()
            / self.scale
    }

    /// Draw text with `y` as the top of the line in logical pixels.
    fn text(&mut self, x: f64, y: f64, text: &str, color: u32, size_logical: f32) {
        let px = (size_logical as f64 * self.scale) as f32;
        let ascent = self
            .font
            .horizontal_line_metrics(px)
            .map(|m| m.ascent)
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
                        if cover == 0 {
                            continue;
                        }
                        let dx = glyph_x + col as i32;
                        let dy = glyph_y + row as i32;
                        self.blend_phys(dx, dy, color, cover);
                    }
                }
            }
            pen_x += metrics.advance_width;
        }
    }

    fn blend_phys(&mut self, x: i32, y: i32, color: u32, cover: u8) {
        if x < 0 || y < 0 {
            return;
        }
        let x = x as usize;
        let y = y as usize;
        if x >= self.w || y >= self.h {
            return;
        }
        let i = y * self.w + x;
        let dst = self.buf[i];
        if cover == 255 {
            self.buf[i] = color;
            return;
        }
        let a = cover as u32;
        let inv = 255 - a;
        let sr = (color >> 16) & 0xff;
        let sg = (color >> 8) & 0xff;
        let sb = color & 0xff;
        let dr = (dst >> 16) & 0xff;
        let dg = (dst >> 8) & 0xff;
        let db = dst & 0xff;
        let r = (sr * a + dr * inv) / 255;
        let g = (sg * a + dg * inv) / 255;
        let b = (sb * a + db * inv) / 255;
        self.buf[i] = (r << 16) | (g << 8) | b;
    }
}

fn inside_rounded(px: i32, py: i32, x0: i32, y0: i32, x1: i32, y1: i32, r: i32) -> bool {
    if px < x0 || py < y0 || px >= x1 || py >= y1 {
        return false;
    }
    if r <= 0 {
        return true;
    }
    let r2 = r * r;
    let corners = [
        (x0 + r, y0 + r, px < x0 + r && py < y0 + r),
        (x1 - r - 1, y0 + r, px >= x1 - r && py < y0 + r),
        (x0 + r, y1 - r - 1, px < x0 + r && py >= y1 - r),
        (x1 - r - 1, y1 - r - 1, px >= x1 - r && py >= y1 - r),
    ];
    for (cx, cy, in_corner) in corners {
        if in_corner {
            let dx = px - cx;
            let dy = py - cy;
            return dx * dx + dy * dy <= r2;
        }
    }
    true
}

fn draw_spectrum_bar(
    fb: &mut Framebuffer<'_>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    spectrum: BatterySpectrum,
) {
    let x0 = fb.to_phys(x);
    let y0 = fb.to_phys(y);
    let x1 = fb.to_phys(x + w);
    let y1 = fb.to_phys(y + h);
    let span = (x1 - x0).max(1);
    for px in x0..x1 {
        let t = (px - x0) as f32 / span as f32;
        let percent = (100.0 - t * 100.0).round() as u8;
        let c = spectrum.color_at_percent(percent);
        fb.fill_rect_phys(px, y0, px + 1, y1, rgb_of(c));
    }
    let border = LINE;
    for px in x0..x1 {
        fb.put_phys(px, y0, border);
        fb.put_phys(px, y1 - 1, border);
    }
    for py in y0..y1 {
        fb.put_phys(x0, py, border);
        fb.put_phys(x1 - 1, py, border);
    }
}

fn draw_sv_square(fb: &mut Framebuffer<'_>, r: Rect, hue: f32, sat: f32, val: f32) {
    let x0 = fb.to_phys(r.x);
    let y0 = fb.to_phys(r.y);
    let x1 = fb.to_phys(r.x + r.w);
    let y1 = fb.to_phys(r.y + r.h);
    let ww = (x1 - x0).max(1) as f32;
    let hh = (y1 - y0).max(1) as f32;
    for py in y0..y1 {
        for px in x0..x1 {
            let s = (px - x0) as f32 / ww;
            let v = 1.0 - (py - y0) as f32 / hh;
            fb.put_phys(px, py, rgb_of(hsv_to_rgb(hue, s, v)));
        }
    }
    let cx = x0 + (sat * ww).round() as i32;
    let cy = y0 + ((1.0 - val) * hh).round() as i32;
    for d in -4..=4 {
        fb.put_phys(cx + d, cy, rgb_u32(255, 255, 255));
        fb.put_phys(cx, cy + d, rgb_u32(255, 255, 255));
        if d.abs() <= 3 {
            fb.put_phys(cx + d, cy, rgb_u32(0, 0, 0));
            fb.put_phys(cx, cy + d, rgb_u32(0, 0, 0));
        }
    }
}

fn draw_hue_strip(fb: &mut Framebuffer<'_>, r: Rect, hue: f32) {
    let x0 = fb.to_phys(r.x);
    let y0 = fb.to_phys(r.y);
    let x1 = fb.to_phys(r.x + r.w);
    let y1 = fb.to_phys(r.y + r.h);
    let hh = (y1 - y0).max(1) as f32;
    for py in y0..y1 {
        let h = (py - y0) as f32 / hh * 360.0;
        let c = rgb_of(hsv_to_rgb(h, 1.0, 1.0));
        fb.fill_rect_phys(x0, py, x1, py + 1, c);
    }
    let cy = y0 + ((hue / 360.0) * hh).round() as i32;
    fb.fill_rect_phys(x0 - 2, cy - 1, x1 + 2, cy + 2, rgb_u32(255, 255, 255));
    fb.fill_rect_phys(x0, cy, x1, cy + 1, rgb_u32(0, 0, 0));
}

fn draw_button(
    fb: &mut Framebuffer<'_>,
    r: Rect,
    label: &str,
    hot: bool,
    primary: bool,
    enabled: bool,
) {
    let (bg, ink, border) = if !enabled {
        (BTN_BG, MUTED, LINE)
    } else if primary {
        if hot {
            (BTN_BG_HOT, BTN_INK_HOT, ACCENT)
        } else {
            (ACCENT, BTN_INK_HOT, ACCENT)
        }
    } else if hot {
        (rgb_u32(220, 226, 235), INK, LINE)
    } else {
        (BTN_BG, INK, LINE)
    };
    fb.round_rect(r.x, r.y, r.w, r.h, bg, Some(border));
    let text_w = fb.text_width(label, FONT_BODY);
    let tx = r.x + (r.w - text_w) * 0.5;
    let ty = r.y + (r.h - FONT_BODY as f64) * 0.5;
    fb.text(tx, ty, label, ink, FONT_BODY);
}
