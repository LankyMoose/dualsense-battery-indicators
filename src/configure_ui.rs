//! Custom-painted dark configuration window.

use crate::app_meta::DISPLAY_NAME;
use crate::color::{BatterySpectrum, Rgb, hsv_to_rgb};
#[cfg(feature = "dev-emulate")]
use crate::emulate::Preset;
use crate::prefs::ToastPosition;
use crate::ui::{self, Framebuffer};
use fontdue::Font;
use softbuffer::{Context, Surface};
use std::num::NonZeroU32;
use std::rc::Rc;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, OwnedDisplayHandle};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

#[cfg(windows)]
use winit::platform::windows::{CornerPreference, WindowAttributesExtWindows};

const WIN_W: f64 = 760.0;
const WIN_H: f64 = 560.0;
const DEV_WIN_H: f64 = 670.0;
const TITLE_H: f64 = 40.0;
const PAD: f64 = 16.0;
const GAP: f64 = 16.0;
const LEFT_W: f64 = 260.0;
const RIGHT_X: f64 = PAD + LEFT_W + GAP;
const RIGHT_W: f64 = WIN_W - RIGHT_X - PAD;
const FONT_TITLE: f32 = 16.0;
const FONT_HEADING: f32 = 15.0;
const FONT_BODY: f32 = 12.0;
const FONT_SMALL: f32 = 10.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationSetting {
    Connect,
    Disconnect,
    Low,
    Charged,
}

pub enum ConfigureAction {
    None,
    ApplySpectrum(BatterySpectrum),
    SetNotification(NotificationSetting, bool),
    SelectToastPosition(ToastPosition),
    #[cfg(windows)]
    SetAutostart(bool),
    #[cfg(feature = "dev-emulate")]
    DeveloperPreset(Preset),
    Closed,
}

#[derive(Debug, Clone, Copy)]
pub struct ConfigureSettings {
    pub notify_low: bool,
    pub notify_charged: bool,
    pub notify_connect: bool,
    pub notify_disconnect: bool,
    pub toast_position: ToastPosition,
    #[cfg(windows)]
    pub autostart: bool,
    pub show_developer: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Full,
    Mid,
    Empty,
}

impl Stop {
    const ALL: [Self; 3] = [Self::Full, Self::Mid, Self::Empty];

    fn label(self) -> &'static str {
        match self {
            Self::Full => "Full",
            Self::Mid => "Mid",
            Self::Empty => "Empty",
        }
    }

    fn percent(self) -> &'static str {
        match self {
            Self::Full => "100%",
            Self::Mid => "50%",
            Self::Empty => "0%",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum DragKind {
    Sv,
    Hue,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

pub struct ConfigureWindow {
    window: Rc<Window>,
    surface: Surface<OwnedDisplayHandle, Rc<Window>>,
    font: Font,
    settings: ConfigureSettings,
    draft: BatterySpectrum,
    selected: Stop,
    hue: f32,
    sat: f32,
    val: f32,
    drag: Option<DragKind>,
    cursor: Option<(f64, f64)>,
    dirty: bool,
}

#[derive(Clone, Copy)]
struct PaintState {
    settings: ConfigureSettings,
    draft: BatterySpectrum,
    selected: Stop,
    hue: f32,
    sat: f32,
    val: f32,
    cursor: Option<(f64, f64)>,
    dirty: bool,
}

impl PaintState {
    fn color_of(self, stop: Stop) -> Rgb {
        match stop {
            Stop::Full => self.draft.full,
            Stop::Mid => self.draft.mid,
            Stop::Empty => self.draft.empty,
        }
    }

    fn notification_enabled(self, setting: NotificationSetting) -> bool {
        match setting {
            NotificationSetting::Connect => self.settings.notify_connect,
            NotificationSetting::Disconnect => self.settings.notify_disconnect,
            NotificationSetting::Low => self.settings.notify_low,
            NotificationSetting::Charged => self.settings.notify_charged,
        }
    }
}

impl ConfigureWindow {
    pub fn open(
        event_loop: &ActiveEventLoop,
        display: OwnedDisplayHandle,
        initial: BatterySpectrum,
        settings: ConfigureSettings,
    ) -> Result<Self, String> {
        let height = if settings.show_developer {
            DEV_WIN_H
        } else {
            WIN_H
        };
        let attrs = WindowAttributes::default()
            .with_title(format!("{DISPLAY_NAME} — Configure"))
            .with_inner_size(LogicalSize::new(WIN_W, height))
            .with_resizable(false)
            .with_decorations(false);
        #[cfg(windows)]
        let attrs = attrs
            .with_undecorated_shadow(true)
            .with_corner_preference(CornerPreference::Round);

        let window = Rc::new(
            event_loop
                .create_window(attrs)
                .map_err(|e| format!("create configure window: {e}"))?,
        );
        let context = Context::new(display).map_err(|e| format!("softbuffer context: {e}"))?;
        let surface = Surface::new(&context, window.clone())
            .map_err(|e| format!("softbuffer surface: {e}"))?;
        let (hue, sat, val) = initial.full.to_hsv();
        let editor = Self {
            window,
            surface,
            font: ui::load_system_ui_font()?,
            settings,
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

    pub fn focus(&self) {
        self.window.set_visible(true);
        self.window.focus_window();
        self.window.request_redraw();
    }

    pub fn sync_settings(&mut self, settings: ConfigureSettings) {
        self.settings = settings;
        self.window.request_redraw();
    }

    pub fn handle(&mut self, event: &WindowEvent) -> ConfigureAction {
        match event {
            WindowEvent::CloseRequested => return ConfigureAction::Closed,
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.logical_key == Key::Named(NamedKey::Escape) =>
            {
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
                    let action = self.on_press();
                    self.window.request_redraw();
                    return action;
                }
                ElementState::Released => self.drag = None,
            },
            WindowEvent::ScaleFactorChanged { .. } => self.window.request_redraw(),
            _ => {}
        }
        ConfigureAction::None
    }

    fn on_press(&mut self) -> ConfigureAction {
        let Some((x, y)) = self.cursor else {
            return ConfigureAction::None;
        };
        if close_rect().contains(x, y) {
            return ConfigureAction::Closed;
        }
        if minimize_rect().contains(x, y) {
            self.window.set_minimized(true);
            return ConfigureAction::None;
        }
        if title_drag_rect().contains(x, y) {
            let _ = self.window.drag_window();
            return ConfigureAction::None;
        }

        for setting in [
            NotificationSetting::Connect,
            NotificationSetting::Disconnect,
            NotificationSetting::Low,
            NotificationSetting::Charged,
        ] {
            if notification_row(setting).contains(x, y) {
                let enabled = !self.notification_enabled(setting);
                self.set_notification(setting, enabled);
                return ConfigureAction::SetNotification(setting, enabled);
            }
        }

        for position in positions() {
            if position_rect(position).contains(x, y) {
                self.settings.toast_position = position;
                return ConfigureAction::SelectToastPosition(position);
            }
        }

        #[cfg(windows)]
        if autostart_row().contains(x, y) {
            self.settings.autostart = !self.settings.autostart;
            return ConfigureAction::SetAutostart(self.settings.autostart);
        }

        #[cfg(feature = "dev-emulate")]
        if self.settings.show_developer {
            for (index, preset) in Preset::ALL.iter().copied().enumerate() {
                if developer_button(index).contains(x, y) {
                    return ConfigureAction::DeveloperPreset(preset);
                }
            }
        }

        for stop in Stop::ALL {
            if stop_rect(stop).contains(x, y) {
                self.select_stop(stop);
                return ConfigureAction::None;
            }
        }
        if sv_rect().contains(x, y) {
            self.drag = Some(DragKind::Sv);
            self.apply_drag();
        } else if hue_rect().contains(x, y) {
            self.drag = Some(DragKind::Hue);
            self.apply_drag();
        } else if reset_rect().contains(x, y) {
            self.draft = BatterySpectrum::DEFAULT;
            self.select_stop(self.selected);
            self.dirty = true;
        } else if apply_rect().contains(x, y) && self.dirty {
            self.dirty = false;
            return ConfigureAction::ApplySpectrum(self.draft);
        }
        ConfigureAction::None
    }

    fn notification_enabled(&self, setting: NotificationSetting) -> bool {
        match setting {
            NotificationSetting::Connect => self.settings.notify_connect,
            NotificationSetting::Disconnect => self.settings.notify_disconnect,
            NotificationSetting::Low => self.settings.notify_low,
            NotificationSetting::Charged => self.settings.notify_charged,
        }
    }

    fn set_notification(&mut self, setting: NotificationSetting, enabled: bool) {
        match setting {
            NotificationSetting::Connect => self.settings.notify_connect = enabled,
            NotificationSetting::Disconnect => self.settings.notify_disconnect = enabled,
            NotificationSetting::Low => self.settings.notify_low = enabled,
            NotificationSetting::Charged => self.settings.notify_charged = enabled,
        }
    }

    fn select_stop(&mut self, stop: Stop) {
        self.selected = stop;
        let (hue, sat, val) = self.color_of(stop).to_hsv();
        self.hue = hue;
        self.sat = sat;
        self.val = val;
    }

    fn color_of(&self, stop: Stop) -> Rgb {
        match stop {
            Stop::Full => self.draft.full,
            Stop::Mid => self.draft.mid,
            Stop::Empty => self.draft.empty,
        }
    }

    fn set_color_of(&mut self, color: Rgb) {
        match self.selected {
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
                let rect = sv_rect();
                self.sat = ((x - rect.x) / rect.w).clamp(0.0, 1.0) as f32;
                self.val = (1.0 - (y - rect.y) / rect.h).clamp(0.0, 1.0) as f32;
            }
            Some(DragKind::Hue) => {
                let rect = hue_rect();
                self.hue = ((y - rect.y) / rect.h).clamp(0.0, 1.0) as f32 * 360.0;
            }
            None => return,
        }
        self.set_color_of(hsv_to_rgb(self.hue, self.sat, self.val));
    }

    fn to_logical(&self, position: PhysicalPosition<f64>) -> (f64, f64) {
        let scale = self.window.scale_factor();
        (position.x / scale, position.y / scale)
    }

    fn paint(&mut self) -> Result<(), String> {
        let size = self.window.inner_size();
        let width = NonZeroU32::new(size.width.max(1)).unwrap();
        let height = NonZeroU32::new(size.height.max(1)).unwrap();
        let state = PaintState {
            settings: self.settings,
            draft: self.draft,
            selected: self.selected,
            hue: self.hue,
            sat: self.sat,
            val: self.val,
            cursor: self.cursor,
            dirty: self.dirty,
        };
        self.surface
            .resize(width, height)
            .map_err(|e| format!("resize: {e}"))?;
        let mut buffer = self
            .surface
            .buffer_mut()
            .map_err(|e| format!("buffer: {e}"))?;
        let mut fb = Framebuffer::new(
            &mut buffer,
            width.get() as usize,
            height.get() as usize,
            self.window.scale_factor(),
            &self.font,
        );
        fb.clear(ui::BG);
        Self::paint_titlebar(state, &mut fb);
        Self::paint_settings(state, &mut fb);
        Self::paint_editor(state, &mut fb);
        #[cfg(feature = "dev-emulate")]
        if state.settings.show_developer {
            Self::paint_developer(state, &mut fb);
        }
        buffer.present().map_err(|e| format!("present: {e}"))
    }

    fn paint_titlebar(state: PaintState, fb: &mut Framebuffer<'_>) {
        let accent = state.draft.full;
        fb.fill_rect(0.0, 0.0, WIN_W, TITLE_H, ui::PANEL);
        fb.fill_rect(0.0, 0.0, 4.0, TITLE_H, ui::rgb_of(accent));
        fb.icon(12.0, 6.0, 28.0, accent);
        fb.text(48.0, 10.0, DISPLAY_NAME, ui::INK, FONT_TITLE);
        let cursor = state.cursor;
        paint_title_button(
            fb,
            minimize_rect(),
            "—",
            cursor.is_some_and(|(x, y)| minimize_rect().contains(x, y)),
            false,
        );
        paint_title_button(
            fb,
            close_rect(),
            "×",
            cursor.is_some_and(|(x, y)| close_rect().contains(x, y)),
            true,
        );
    }

    fn paint_settings(state: PaintState, fb: &mut Framebuffer<'_>) {
        section_heading(fb, PAD, 58.0, "Settings", "Controller notifications");
        card(fb, notifications_card());
        for setting in [
            NotificationSetting::Connect,
            NotificationSetting::Disconnect,
            NotificationSetting::Low,
            NotificationSetting::Charged,
        ] {
            let row = notification_row(setting);
            let label = match setting {
                NotificationSetting::Connect => "Controller connected",
                NotificationSetting::Disconnect => "Controller disconnected",
                NotificationSetting::Low => "Low battery",
                NotificationSetting::Charged => "Fully charged",
            };
            fb.text(row.x + 12.0, row.y + 11.0, label, ui::INK, FONT_BODY);
            paint_switch(
                fb,
                switch_rect(row),
                state.notification_enabled(setting),
                state.cursor.is_some_and(|(x, y)| row.contains(x, y)),
                state.draft.full,
            );
        }

        section_heading(
            fb,
            PAD,
            270.0,
            "Toast position",
            "Select a corner to preview",
        );
        card(fb, position_card());
        for position in positions() {
            let rect = position_rect(position);
            let selected = position == state.settings.toast_position;
            let hot = state.cursor.is_some_and(|(x, y)| rect.contains(x, y));
            let label = match position {
                ToastPosition::TopLeft => "Top left",
                ToastPosition::TopRight => "Top right",
                ToastPosition::BottomLeft => "Bottom left",
                ToastPosition::BottomRight => "Bottom right",
            };
            paint_choice(fb, rect, label, selected, hot, state.draft.full);
        }

        section_heading(fb, PAD, 432.0, "System", "Launch preferences");
        card(fb, system_card());
        #[cfg(windows)]
        {
            let row = autostart_row();
            fb.text(
                row.x + 12.0,
                row.y + 11.0,
                "Start with Windows",
                ui::INK,
                FONT_BODY,
            );
            paint_switch(
                fb,
                switch_rect(row),
                state.settings.autostart,
                state.cursor.is_some_and(|(x, y)| row.contains(x, y)),
                state.draft.full,
            );
        }
        #[cfg(not(windows))]
        fb.text(
            system_card().x + 12.0,
            system_card().y + 18.0,
            "Managed by your operating system",
            ui::MUTED,
            FONT_SMALL,
        );
    }

    fn paint_editor(state: PaintState, fb: &mut Framebuffer<'_>) {
        section_heading(
            fb,
            RIGHT_X,
            58.0,
            "Lightbar colors",
            "Blend three colors across battery level",
        );
        let preview = preview_rect();
        card(fb, preview);
        fb.text(
            preview.x + 12.0,
            preview.y + 10.0,
            "BATTERY SPECTRUM",
            ui::MUTED,
            FONT_SMALL,
        );
        draw_spectrum_bar(
            fb,
            Rect {
                x: preview.x + 12.0,
                y: preview.y + 36.0,
                w: preview.w - 24.0,
                h: 24.0,
            },
            state.draft,
        );

        for stop in Stop::ALL {
            let rect = stop_rect(stop);
            let selected = stop == state.selected;
            let hot = state.cursor.is_some_and(|(x, y)| rect.contains(x, y));
            fb.round_rect(
                (rect.x, rect.y, rect.w, rect.h),
                8.0,
                if hot { ui::PANEL_HOVER } else { ui::PANEL },
                Some(if selected {
                    ui::rgb_of(state.color_of(stop))
                } else {
                    ui::LINE
                }),
            );
            fb.round_rect(
                (rect.x + 10.0, rect.y + 10.0, 34.0, 34.0),
                6.0,
                ui::rgb_of(state.color_of(stop)),
                None,
            );
            fb.text(
                rect.x + 52.0,
                rect.y + 9.0,
                stop.label(),
                ui::INK,
                FONT_BODY,
            );
            fb.text(
                rect.x + 52.0,
                rect.y + 28.0,
                stop.percent(),
                ui::MUTED,
                FONT_SMALL,
            );
            fb.text(
                rect.x + 10.0,
                rect.y + 57.0,
                &state.color_of(stop).to_hex(),
                ui::MUTED,
                FONT_SMALL,
            );
        }

        let picker = picker_rect();
        card(fb, picker);
        fb.text(
            picker.x + 12.0,
            picker.y + 10.0,
            &format!(
                "Editing {} · {}",
                state.selected.label(),
                state.selected.percent()
            ),
            ui::INK,
            FONT_BODY,
        );
        draw_sv_square(fb, sv_rect(), state.hue, state.sat, state.val);
        draw_hue_strip(fb, hue_rect(), state.hue);

        paint_button(
            fb,
            reset_rect(),
            "Reset defaults",
            state
                .cursor
                .is_some_and(|(x, y)| reset_rect().contains(x, y)),
            false,
            true,
            state.draft.full,
        );
        paint_button(
            fb,
            apply_rect(),
            "Apply",
            state
                .cursor
                .is_some_and(|(x, y)| apply_rect().contains(x, y)),
            true,
            state.dirty,
            state.draft.full,
        );
    }

    #[cfg(feature = "dev-emulate")]
    fn paint_developer(state: PaintState, fb: &mut Framebuffer<'_>) {
        section_heading(fb, PAD, 552.0, "Developer", "Emulate controller states");
        for (index, preset) in Preset::ALL.iter().copied().enumerate() {
            let rect = developer_button(index);
            let label = preset
                .menu_label()
                .strip_prefix("Emulate: ")
                .unwrap_or(preset.menu_label());
            paint_button(
                fb,
                rect,
                label,
                state.cursor.is_some_and(|(x, y)| rect.contains(x, y)),
                false,
                true,
                state.draft.full,
            );
        }
    }
}

fn title_drag_rect() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: WIN_W - 96.0,
        h: TITLE_H,
    }
}

fn minimize_rect() -> Rect {
    Rect {
        x: WIN_W - 96.0,
        y: 0.0,
        w: 48.0,
        h: TITLE_H,
    }
}

fn close_rect() -> Rect {
    Rect {
        x: WIN_W - 48.0,
        y: 0.0,
        w: 48.0,
        h: TITLE_H,
    }
}

fn notifications_card() -> Rect {
    Rect {
        x: PAD,
        y: 92.0,
        w: LEFT_W,
        h: 162.0,
    }
}

fn notification_row(setting: NotificationSetting) -> Rect {
    let index = match setting {
        NotificationSetting::Connect => 0,
        NotificationSetting::Disconnect => 1,
        NotificationSetting::Low => 2,
        NotificationSetting::Charged => 3,
    };
    Rect {
        x: PAD + 4.0,
        y: 96.0 + index as f64 * 38.0,
        w: LEFT_W - 8.0,
        h: 36.0,
    }
}

fn switch_rect(row: Rect) -> Rect {
    Rect {
        x: row.x + row.w - 48.0,
        y: row.y + 9.0,
        w: 38.0,
        h: 20.0,
    }
}

fn position_card() -> Rect {
    Rect {
        x: PAD,
        y: 304.0,
        w: LEFT_W,
        h: 112.0,
    }
}

fn positions() -> [ToastPosition; 4] {
    [
        ToastPosition::TopLeft,
        ToastPosition::TopRight,
        ToastPosition::BottomLeft,
        ToastPosition::BottomRight,
    ]
}

fn position_rect(position: ToastPosition) -> Rect {
    let (column, row) = match position {
        ToastPosition::TopLeft => (0, 0),
        ToastPosition::TopRight => (1, 0),
        ToastPosition::BottomLeft => (0, 1),
        ToastPosition::BottomRight => (1, 1),
    };
    Rect {
        x: PAD + 8.0 + column as f64 * 122.0,
        y: 310.0 + row as f64 * 49.0,
        w: 114.0,
        h: 41.0,
    }
}

fn system_card() -> Rect {
    Rect {
        x: PAD,
        y: 466.0,
        w: LEFT_W,
        h: 60.0,
    }
}

#[cfg(windows)]
fn autostart_row() -> Rect {
    Rect {
        x: PAD + 4.0,
        y: 477.0,
        w: LEFT_W - 8.0,
        h: 38.0,
    }
}

fn preview_rect() -> Rect {
    Rect {
        x: RIGHT_X,
        y: 92.0,
        w: RIGHT_W,
        h: 76.0,
    }
}

fn stop_rect(stop: Stop) -> Rect {
    let index = match stop {
        Stop::Full => 0,
        Stop::Mid => 1,
        Stop::Empty => 2,
    };
    let width = (RIGHT_W - 16.0) / 3.0;
    Rect {
        x: RIGHT_X + index as f64 * (width + 8.0),
        y: 176.0,
        w: width,
        h: 82.0,
    }
}

fn picker_rect() -> Rect {
    Rect {
        x: RIGHT_X,
        y: 270.0,
        w: RIGHT_W,
        h: 204.0,
    }
}

fn sv_rect() -> Rect {
    let picker = picker_rect();
    Rect {
        x: picker.x + 12.0,
        y: picker.y + 38.0,
        w: picker.w - 52.0,
        h: 150.0,
    }
}

fn hue_rect() -> Rect {
    let sv = sv_rect();
    Rect {
        x: sv.x + sv.w + 10.0,
        y: sv.y,
        w: 18.0,
        h: sv.h,
    }
}

fn reset_rect() -> Rect {
    Rect {
        x: RIGHT_X,
        y: 488.0,
        w: 124.0,
        h: 36.0,
    }
}

fn apply_rect() -> Rect {
    Rect {
        x: RIGHT_X + RIGHT_W - 92.0,
        y: 488.0,
        w: 92.0,
        h: 36.0,
    }
}

#[cfg(feature = "dev-emulate")]
fn developer_button(index: usize) -> Rect {
    let column = index % 4;
    let row = index / 4;
    let width = (WIN_W - PAD * 2.0 - 24.0) / 4.0;
    Rect {
        x: PAD + column as f64 * (width + 8.0),
        y: 588.0 + row as f64 * 38.0,
        w: width,
        h: 32.0,
    }
}

fn card(fb: &mut Framebuffer<'_>, rect: Rect) {
    fb.round_rect(
        (rect.x, rect.y, rect.w, rect.h),
        10.0,
        ui::PANEL,
        Some(ui::LINE),
    );
}

fn section_heading(fb: &mut Framebuffer<'_>, x: f64, y: f64, title: &str, subtitle: &str) {
    fb.text(x, y, title, ui::INK, FONT_HEADING);
    fb.text(x, y + 20.0, subtitle, ui::MUTED, FONT_SMALL);
}

fn paint_title_button(fb: &mut Framebuffer<'_>, rect: Rect, label: &str, hot: bool, danger: bool) {
    if hot {
        fb.fill_rect(
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            if danger {
                ui::rgb(196, 43, 54)
            } else {
                ui::PANEL_HOVER
            },
        );
    }
    let width = fb.text_width(label, 18.0);
    fb.text(
        rect.x + (rect.w - width) / 2.0,
        rect.y + 15.0,
        label,
        ui::INK,
        18.0,
    );
}

fn paint_switch(fb: &mut Framebuffer<'_>, rect: Rect, enabled: bool, hot: bool, accent: Rgb) {
    let track = if enabled {
        ui::rgb_of(accent)
    } else if hot {
        ui::DIM
    } else {
        ui::LINE
    };
    fb.round_rect((rect.x, rect.y, rect.w, rect.h), 10.0, track, None);
    let knob_x = if enabled {
        rect.x + rect.w - 18.0
    } else {
        rect.x + 2.0
    };
    fb.round_rect((knob_x, rect.y + 2.0, 16.0, 16.0), 8.0, ui::INK, None);
}

fn paint_choice(
    fb: &mut Framebuffer<'_>,
    rect: Rect,
    label: &str,
    selected: bool,
    hot: bool,
    accent: Rgb,
) {
    fb.round_rect(
        (rect.x, rect.y, rect.w, rect.h),
        7.0,
        if hot { ui::PANEL_HOVER } else { ui::BG },
        Some(if selected {
            ui::rgb_of(accent)
        } else {
            ui::LINE
        }),
    );
    let width = fb.text_width(label, FONT_SMALL);
    fb.text(
        rect.x + (rect.w - width) / 2.0,
        rect.y + 16.0,
        label,
        if selected { ui::INK } else { ui::MUTED },
        FONT_SMALL,
    );
}

fn paint_button(
    fb: &mut Framebuffer<'_>,
    rect: Rect,
    label: &str,
    hot: bool,
    primary: bool,
    enabled: bool,
    accent: Rgb,
) {
    let (fill, ink, border) = if !enabled {
        (ui::PANEL, ui::DIM, ui::LINE)
    } else if primary {
        (
            ui::rgb_of(if hot {
                accent.lerp(Rgb::WHITE, 0.12)
            } else {
                accent
            }),
            ui::INK,
            ui::rgb_of(accent),
        )
    } else {
        (
            if hot { ui::PANEL_HOVER } else { ui::PANEL },
            ui::INK,
            ui::LINE,
        )
    };
    fb.round_rect((rect.x, rect.y, rect.w, rect.h), 7.0, fill, Some(border));
    let width = fb.text_width(label, FONT_BODY);
    fb.text(
        rect.x + (rect.w - width) / 2.0,
        rect.y + 11.0,
        label,
        ink,
        FONT_BODY,
    );
}

fn draw_spectrum_bar(fb: &mut Framebuffer<'_>, rect: Rect, spectrum: BatterySpectrum) {
    let x0 = fb.to_phys(rect.x);
    let y0 = fb.to_phys(rect.y);
    let x1 = fb.to_phys(rect.x + rect.w);
    let y1 = fb.to_phys(rect.y + rect.h);
    let span = (x1 - x0).max(1);
    for x in x0..x1 {
        let percent = (100.0 - (x - x0) as f32 / span as f32 * 100.0).round() as u8;
        fb.fill_rect_phys(
            x,
            y0,
            x + 1,
            y1,
            ui::rgb_of(spectrum.color_at_percent(percent)),
        );
    }
}

fn draw_sv_square(fb: &mut Framebuffer<'_>, rect: Rect, hue: f32, sat: f32, val: f32) {
    let x0 = fb.to_phys(rect.x);
    let y0 = fb.to_phys(rect.y);
    let x1 = fb.to_phys(rect.x + rect.w);
    let y1 = fb.to_phys(rect.y + rect.h);
    let width = (x1 - x0).max(1) as f32;
    let height = (y1 - y0).max(1) as f32;
    for y in y0..y1 {
        for x in x0..x1 {
            let saturation = (x - x0) as f32 / width;
            let value = 1.0 - (y - y0) as f32 / height;
            fb.put_phys(x, y, ui::rgb_of(hsv_to_rgb(hue, saturation, value)));
        }
    }
    let cx = x0 + (sat * width).round() as i32;
    let cy = y0 + ((1.0 - val) * height).round() as i32;
    for delta in -5..=5 {
        fb.put_phys(cx + delta, cy, ui::INK);
        fb.put_phys(cx, cy + delta, ui::INK);
    }
    for delta in -3..=3 {
        fb.put_phys(cx + delta, cy, ui::BG);
        fb.put_phys(cx, cy + delta, ui::BG);
    }
}

fn draw_hue_strip(fb: &mut Framebuffer<'_>, rect: Rect, hue: f32) {
    let x0 = fb.to_phys(rect.x);
    let y0 = fb.to_phys(rect.y);
    let x1 = fb.to_phys(rect.x + rect.w);
    let y1 = fb.to_phys(rect.y + rect.h);
    let height = (y1 - y0).max(1) as f32;
    for y in y0..y1 {
        let color = hsv_to_rgb((y - y0) as f32 / height * 360.0, 1.0, 1.0);
        fb.fill_rect_phys(x0, y, x1, y + 1, ui::rgb_of(color));
    }
    let cy = y0 + (hue / 360.0 * height).round() as i32;
    fb.fill_rect_phys(x0 - 2, cy - 2, x1 + 2, cy + 2, ui::INK);
    fb.fill_rect_phys(x0, cy - 1, x1, cy + 1, ui::BG);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_choices_map_to_distinct_hit_regions() {
        for position in positions() {
            let rect = position_rect(position);
            let center = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
            assert!(rect.contains(center.0, center.1));
            assert_eq!(
                positions()
                    .into_iter()
                    .filter(|candidate| position_rect(*candidate).contains(center.0, center.1))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn title_controls_do_not_overlap_drag_region() {
        assert!(title_drag_rect().x + title_drag_rect().w <= minimize_rect().x);
        assert!(minimize_rect().x + minimize_rect().w <= close_rect().x);
    }
}
