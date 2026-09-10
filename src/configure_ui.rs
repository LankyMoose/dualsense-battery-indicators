//! Custom-painted dark configuration window.

use crate::app_meta::DISPLAY_NAME;
use crate::color::{BatterySpectrum, Rgb, hsv_to_rgb};
#[cfg(feature = "dev-emulate")]
use crate::emulate::Preset;
use crate::prefs::ToastPosition;
use crate::svg_icon;
use crate::ui::layout::{self, Rect};
use crate::ui::{self, Framebuffer};
use fontdue::Font;
use softbuffer::{Context, Surface};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, OwnedDisplayHandle};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

#[cfg(windows)]
use winit::platform::windows::{CornerPreference, WindowAttributesExtWindows};

const WIN_W: f64 = 320.0;
const FONT_HEADING: f32 = 15.0;
const FONT_BODY: f32 = 12.0;
#[cfg(not(windows))]
const FONT_SMALL: f32 = 10.5;
const ACCORDION_HEADER_H: f64 = 22.0;
const ACCORDION_GAP: f64 = 6.0;
const ACCORDION_ANIM_MS: u64 = 180;
const HEADER_TITLE_PAD_X: f64 = 12.0;
const CONTENT_PAD_X: f64 = 12.0;
const CONTENT_PAD_Y: f64 = 8.0;
const PREVIEW_H: f64 = 72.0;
const PICKER_H: f64 = 146.0;
const RESET_H: f64 = 32.0;
const SYSTEM_ROW_H: f64 = 32.0;
const NOTIFICATION_ROW_H: f64 = 28.0;
/// Representative toast aspect for the position diagram (max width / typical height).
const TOAST_ASPECT: f64 = 360.0 / 60.0;
const POSITION_TOAST_W: f64 = 56.0;
const POSITION_TOAST_MARGIN: f64 = 8.0;
const POSITION_STAGE_DARK: u32 = ui::rgb(15, 17, 22);

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

#[derive(Debug, Clone, Copy)]
enum DragKind {
    Sv,
    Hue,
    Stop { index: usize },
}

const STOP_HANDLE_W: f64 = 18.0;
const STOP_HANDLE_H: f64 = 28.0;
const STOP_REMOVE_DISTANCE: f64 = 28.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccordionSection {
    System,
    Notifications,
    ToastPosition,
    LightbarColors,
    #[cfg(feature = "dev-emulate")]
    Developer,
}

impl AccordionSection {
    fn visible(show_developer: bool) -> Vec<Self> {
        let sections = vec![
            Self::System,
            Self::Notifications,
            Self::ToastPosition,
            Self::LightbarColors,
        ];
        #[cfg(not(feature = "dev-emulate"))]
        let _ = show_developer;
        #[cfg(feature = "dev-emulate")]
        let mut sections = sections;
        #[cfg(feature = "dev-emulate")]
        if show_developer {
            sections.push(Self::Developer);
        }
        sections
    }

    fn title(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Notifications => "Notifications",
            Self::ToastPosition => "Toast position",
            Self::LightbarColors => "Lightbar colors",
            #[cfg(feature = "dev-emulate")]
            Self::Developer => "Developer",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AccordionAnimation {
    from: AccordionSection,
    to: AccordionSection,
    started_at: Instant,
}

impl AccordionAnimation {
    fn progress(self) -> f64 {
        let elapsed = self.started_at.elapsed().as_secs_f64();
        let raw =
            (elapsed / Duration::from_millis(ACCORDION_ANIM_MS).as_secs_f64()).clamp(0.0, 1.0);
        1.0 - (1.0 - raw).powi(3)
    }
}

pub struct ConfigureWindow {
    window: Rc<Window>,
    surface: Surface<OwnedDisplayHandle, Rc<Window>>,
    font: Font,
    settings: ConfigureSettings,
    draft: BatterySpectrum,
    selected: usize,
    hue: f32,
    sat: f32,
    val: f32,
    drag: Option<DragKind>,
    remove_armed: bool,
    pending_commit: bool,
    cursor: Option<(f64, f64)>,
    expanded_section: AccordionSection,
    accordion_anim: Option<AccordionAnimation>,
}

#[derive(Clone)]
struct PaintState {
    settings: ConfigureSettings,
    draft: BatterySpectrum,
    selected: usize,
    hue: f32,
    sat: f32,
    val: f32,
    cursor: Option<(f64, f64)>,
    remove_armed: bool,
    dragging_stop: Option<usize>,
    expanded_section: AccordionSection,
    accordion_anim: Option<AccordionAnimation>,
}

impl PaintState {
    fn selected_stop(&self) -> Option<&crate::color::GradientStop> {
        self.draft.stops.get(self.selected)
    }

    fn accent(&self) -> Rgb {
        self.draft.accent()
    }

    fn section_amount(&self, section: AccordionSection) -> f64 {
        match self.accordion_anim {
            Some(anim) => {
                let progress = anim.progress();
                if section == anim.from {
                    1.0 - progress
                } else if section == anim.to {
                    progress
                } else {
                    0.0
                }
            }
            None => (section == self.expanded_section) as u8 as f64,
        }
    }

    fn is_animating(&self) -> bool {
        self.accordion_anim
            .is_some_and(|anim| anim.progress() < 1.0)
    }

    fn notification_enabled(&self, setting: NotificationSetting) -> bool {
        match setting {
            NotificationSetting::Connect => self.settings.notify_connect,
            NotificationSetting::Disconnect => self.settings.notify_disconnect,
            NotificationSetting::Low => self.settings.notify_low,
            NotificationSetting::Charged => self.settings.notify_charged,
        }
    }
}

#[derive(Clone)]
struct ConfigureLayout {
    title_drag: Rect,
    title_icon: Rect,
    title_label: Rect,
    minimize: Rect,
    close: Rect,
    sections: Vec<SectionLayout>,
    notification_rows: [Rect; 4],
    notification_labels: [Rect; 4],
    notification_switches: [Rect; 4],
    position_stage: Rect,
    position_choices: [Rect; 6],
    autostart_row: Rect,
    autostart_label: Rect,
    autostart_switch: Rect,
    preview: Rect,
    spectrum_bar: Rect,
    picker: Rect,
    picker_label: Rect,
    sv: Rect,
    hue: Rect,
    reset: Rect,
    height: f64,
    #[cfg(feature = "dev-emulate")]
    developer_buttons: Vec<Rect>,
}

#[derive(Clone, Copy)]
struct StopHandleLayout {
    bounds: Rect,
    tip: (f64, f64),
    swatch: Rect,
}

#[derive(Clone, Copy)]
struct SectionLayout {
    section: AccordionSection,
    header: Rect,
    content: Rect,
    full_height: f64,
    visible_height: f64,
}

impl ConfigureLayout {
    fn compute(state: &PaintState, show_developer: bool) -> Self {
        let title_drag = Rect::new(
            layout::SPACE_3,
            layout::SPACE_1,
            WIN_W
                - layout::SPACE_3
                - (layout::WINDOW_HEADER_ACTION_SIZE * 2.0 + layout::SPACE_1 * 3.0),
            layout::WINDOW_HEADER_HEIGHT,
        );
        let title_icon = Rect::new(
            title_drag.x,
            layout::SPACE_1 + 4.0,
            layout::WINDOW_HEADER_ICON_SIZE,
            layout::WINDOW_HEADER_ICON_SIZE,
        );
        let title_label = Rect::new(
            title_icon.right() + layout::WINDOW_HEADER_PADDING,
            layout::SPACE_1,
            title_drag.right() - title_icon.right() - layout::WINDOW_HEADER_PADDING,
            layout::WINDOW_HEADER_HEIGHT - layout::SPACE_1 * 2.0,
        );
        let minimize = Rect::new(
            WIN_W - layout::WINDOW_HEADER_ACTION_SIZE * 2.0 - layout::SPACE_1 * 2.0,
            layout::SPACE_1,
            layout::WINDOW_HEADER_ACTION_SIZE,
            layout::WINDOW_HEADER_ACTION_SIZE,
        );
        let close = Rect::new(
            WIN_W - layout::WINDOW_HEADER_ACTION_SIZE - layout::SPACE_1,
            layout::SPACE_1,
            layout::WINDOW_HEADER_ACTION_SIZE,
            layout::WINDOW_HEADER_ACTION_SIZE,
        );

        let sections = AccordionSection::visible(show_developer);
        let content_x = 0.0;
        let content_w = WIN_W;
        let base_height = layout::WINDOW_HEADER_HEIGHT
            + sections.len() as f64 * ACCORDION_HEADER_H
            + (sections.len().saturating_sub(1)) as f64 * ACCORDION_GAP;
        let max_panel_h = sections
            .iter()
            .map(|section| section_full_height(*section))
            .fold(0.0, f64::max);
        let window_h = base_height + max_panel_h;

        let mut y = layout::WINDOW_HEADER_HEIGHT;
        let mut section_layouts = Vec::new();
        for (index, section) in sections.iter().copied().enumerate() {
            let header = Rect::new(content_x, y, content_w, ACCORDION_HEADER_H);
            y += ACCORDION_HEADER_H;
            let full_height = section_full_height(section);
            let visible_height = full_height * state.section_amount(section);
            let content = Rect::new(content_x, y, content_w, full_height);
            section_layouts.push(SectionLayout {
                section,
                header,
                content,
                full_height,
                visible_height,
            });
            y += visible_height;
            if index + 1 < sections.len() {
                y += ACCORDION_GAP;
            }
        }

        let notifications = section_layouts
            .iter()
            .find(|layout| layout.section == AccordionSection::Notifications)
            .copied()
            .unwrap();
        let system = section_layouts
            .iter()
            .find(|layout| layout.section == AccordionSection::System)
            .copied()
            .unwrap();
        let position = section_layouts
            .iter()
            .find(|layout| layout.section == AccordionSection::ToastPosition)
            .copied()
            .unwrap();
        let lightbar = section_layouts
            .iter()
            .find(|layout| layout.section == AccordionSection::LightbarColors)
            .copied()
            .unwrap();

        let notification_rows = std::array::from_fn(|index| {
            Rect::new(
                notifications.content.x + CONTENT_PAD_X,
                notifications.content.y + CONTENT_PAD_Y + index as f64 * NOTIFICATION_ROW_H,
                notifications.content.w - CONTENT_PAD_X * 2.0,
                NOTIFICATION_ROW_H,
            )
        });
        let notification_labels =
            notification_rows.map(|row| Rect::new(row.x, row.y, row.w - 46.0, row.h));
        let notification_switches = notification_rows
            .map(|row| Rect::new(row.right() - 38.0, row.y + (row.h - 20.0) / 2.0, 38.0, 20.0));

        let system_row = Rect::new(
            system.content.x + CONTENT_PAD_X,
            system.content.y + CONTENT_PAD_Y,
            system.content.w - CONTENT_PAD_X * 2.0,
            SYSTEM_ROW_H,
        );
        let system_label = Rect::new(
            system_row.x,
            system_row.y,
            system_row.w - 46.0,
            system_row.h,
        );
        let system_switch = Rect::new(
            system_row.right() - 38.0,
            system_row.y + (system_row.h - 20.0) / 2.0,
            38.0,
            20.0,
        );

        let position_stage = position_stage_rect(position.content);
        let toast_h = POSITION_TOAST_W / TOAST_ASPECT;
        let center_x = position_stage.x + (position_stage.w - POSITION_TOAST_W) / 2.0;
        let position_choices = [
            Rect::new(
                position_stage.x + POSITION_TOAST_MARGIN,
                position_stage.y + POSITION_TOAST_MARGIN,
                POSITION_TOAST_W,
                toast_h,
            ),
            Rect::new(
                center_x,
                position_stage.y + POSITION_TOAST_MARGIN,
                POSITION_TOAST_W,
                toast_h,
            ),
            Rect::new(
                position_stage.right() - POSITION_TOAST_MARGIN - POSITION_TOAST_W,
                position_stage.y + POSITION_TOAST_MARGIN,
                POSITION_TOAST_W,
                toast_h,
            ),
            Rect::new(
                position_stage.x + POSITION_TOAST_MARGIN,
                position_stage.bottom() - POSITION_TOAST_MARGIN - toast_h,
                POSITION_TOAST_W,
                toast_h,
            ),
            Rect::new(
                center_x,
                position_stage.bottom() - POSITION_TOAST_MARGIN - toast_h,
                POSITION_TOAST_W,
                toast_h,
            ),
            Rect::new(
                position_stage.right() - POSITION_TOAST_MARGIN - POSITION_TOAST_W,
                position_stage.bottom() - POSITION_TOAST_MARGIN - toast_h,
                POSITION_TOAST_W,
                toast_h,
            ),
        ];

        let preview = Rect::new(
            lightbar.content.x + CONTENT_PAD_X,
            lightbar.content.y + CONTENT_PAD_Y,
            lightbar.content.w - CONTENT_PAD_X * 2.0,
            PREVIEW_H,
        );
        let spectrum_bar = Rect::new(
            preview.x + CONTENT_PAD_X,
            preview.y + CONTENT_PAD_X,
            preview.w - CONTENT_PAD_X * 2.0,
            24.0,
        );
        let picker = Rect::new(
            lightbar.content.x + CONTENT_PAD_X,
            preview.bottom() + ACCORDION_GAP,
            lightbar.content.w - CONTENT_PAD_X * 2.0,
            PICKER_H,
        );
        let picker_label = Rect::new(picker.x + 12.0, picker.y + 10.0, picker.w - 24.0, 18.0);
        let sv = Rect::new(picker.x + 12.0, picker.y + 34.0, picker.w - 52.0, 100.0);
        let hue = Rect::new(picker.right() - 30.0, picker.y + 34.0, 18.0, 100.0);
        let reset = Rect::new(
            lightbar.content.x + CONTENT_PAD_X,
            picker.bottom() + ACCORDION_GAP,
            124.0,
            RESET_H,
        );

        #[cfg(feature = "dev-emulate")]
        let developer_buttons = {
            if let Some(dev) = section_layouts
                .iter()
                .find(|layout| layout.section == AccordionSection::Developer)
                .copied()
            {
                let mut buttons = Vec::new();
                let inner_w = dev.content.w - CONTENT_PAD_X * 2.0;
                let bw = (inner_w - layout::SPACE_2 * 3.0) / 4.0;
                let by = dev.content.y + CONTENT_PAD_Y;
                for row in 0..2 {
                    for col in 0..4 {
                        buttons.push(Rect::new(
                            dev.content.x + CONTENT_PAD_X + col as f64 * (bw + layout::SPACE_2),
                            by + row as f64 * (30.0 + 5.0),
                            bw,
                            30.0,
                        ));
                    }
                }
                buttons
            } else {
                Vec::new()
            }
        };

        Self {
            title_drag,
            title_icon,
            title_label,
            minimize,
            close,
            sections: section_layouts,
            notification_rows,
            notification_labels,
            notification_switches,
            position_stage,
            position_choices,
            autostart_row: system_row,
            autostart_label: system_label,
            autostart_switch: system_switch,
            preview,
            spectrum_bar,
            picker,
            picker_label,
            sv,
            hue,
            reset,
            height: window_h,
            #[cfg(feature = "dev-emulate")]
            developer_buttons,
        }
    }

    fn notification_row(&self, setting: NotificationSetting) -> Rect {
        self.notification_rows[notification_index(setting)]
    }

    fn position_choice(&self, position: ToastPosition) -> Rect {
        self.position_choices[position_index(position)]
    }

    #[cfg(test)]
    fn section(&self, section: AccordionSection) -> SectionLayout {
        self.sections
            .iter()
            .find(|layout| layout.section == section)
            .copied()
            .unwrap_or(SectionLayout {
                section,
                header: Rect::default(),
                content: Rect::default(),
                full_height: 0.0,
                visible_height: 0.0,
            })
    }

    fn stop_handles_for(&self, spectrum: &BatterySpectrum) -> Vec<StopHandleLayout> {
        spectrum
            .stops
            .iter()
            .map(|stop| stop_handle_layout(self.spectrum_bar, stop.percent))
            .collect()
    }
}

fn percent_from_bar_x(bar: Rect, x: f64) -> u8 {
    if bar.w <= 0.0 {
        return 100;
    }
    let t = ((x - bar.x) / bar.w).clamp(0.0, 1.0);
    (100.0 - t * 100.0).round() as u8
}

fn bar_x_from_percent(bar: Rect, percent: u8) -> f64 {
    bar.x + (1.0 - percent.min(100) as f64 / 100.0) * bar.w
}

fn stop_handle_rect(bar: Rect, percent: u8) -> Rect {
    let cx = bar_x_from_percent(bar, percent);
    Rect::new(
        cx - STOP_HANDLE_W / 2.0,
        bar.bottom() + 2.0,
        STOP_HANDLE_W,
        STOP_HANDLE_H,
    )
}

fn stop_handle_layout(bar: Rect, percent: u8) -> StopHandleLayout {
    let bounds = stop_handle_rect(bar, percent);
    let cx = bounds.x + bounds.w / 2.0;
    StopHandleLayout {
        bounds,
        tip: (cx, bar.bottom()),
        swatch: Rect::new(bounds.x + 3.0, bounds.y + 10.0, 12.0, 12.0),
    }
}

fn section_full_height(section: AccordionSection) -> f64 {
    match section {
        AccordionSection::System => CONTENT_PAD_Y * 2.0 + SYSTEM_ROW_H,
        AccordionSection::Notifications => CONTENT_PAD_Y * 2.0 + NOTIFICATION_ROW_H * 4.0,
        AccordionSection::ToastPosition => {
            let stage = position_stage_rect(Rect::new(0.0, 0.0, WIN_W, 0.0));
            CONTENT_PAD_Y * 2.0 + stage.h
        }
        AccordionSection::LightbarColors => {
            CONTENT_PAD_Y * 2.0 + PREVIEW_H + ACCORDION_GAP + PICKER_H + ACCORDION_GAP + RESET_H
        }
        #[cfg(feature = "dev-emulate")]
        AccordionSection::Developer => CONTENT_PAD_Y * 2.0 + 65.0,
    }
}

fn position_stage_rect(content: Rect) -> Rect {
    let width = content.w - CONTENT_PAD_X * 2.0;
    let height = width * 9.0 / 16.0;
    Rect::new(
        content.x + CONTENT_PAD_X,
        content.y + CONTENT_PAD_Y,
        width,
        height,
    )
}

fn notification_index(setting: NotificationSetting) -> usize {
    match setting {
        NotificationSetting::Connect => 0,
        NotificationSetting::Disconnect => 1,
        NotificationSetting::Low => 2,
        NotificationSetting::Charged => 3,
    }
}

fn position_index(position: ToastPosition) -> usize {
    match position {
        ToastPosition::TopLeft => 0,
        ToastPosition::TopCenter => 1,
        ToastPosition::TopRight => 2,
        ToastPosition::BottomLeft => 3,
        ToastPosition::BottomCenter => 4,
        ToastPosition::BottomRight => 5,
    }
}

impl ConfigureWindow {
    pub fn open(
        event_loop: &ActiveEventLoop,
        display: OwnedDisplayHandle,
        initial: BatterySpectrum,
        settings: ConfigureSettings,
    ) -> Result<Self, String> {
        let font = ui::load_system_ui_font()?;
        let expanded_section = AccordionSection::visible(settings.show_developer)
            .into_iter()
            .next()
            .unwrap_or(AccordionSection::System);
        let preview_state = PaintState {
            settings,
            draft: initial.clone(),
            selected: 0,
            hue: 0.0,
            sat: 0.0,
            val: 0.0,
            cursor: None,
            remove_armed: false,
            dragging_stop: None,
            expanded_section,
            accordion_anim: None,
        };
        let layout = ConfigureLayout::compute(&preview_state, settings.show_developer);
        let attrs = WindowAttributes::default()
            .with_title(format!("{DISPLAY_NAME} — Configure"))
            .with_inner_size(LogicalSize::new(WIN_W, layout.height))
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
        let (hue, sat, val) = initial.accent().to_hsv();
        let editor = Self {
            window,
            surface,
            font,
            settings,
            draft: initial,
            selected: 0,
            hue,
            sat,
            val,
            drag: None,
            remove_armed: false,
            pending_commit: false,
            cursor: None,
            expanded_section,
            accordion_anim: None,
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
        self.resize_to_content();
        self.window.request_redraw();
    }

    fn layout(&self) -> Result<ConfigureLayout, String> {
        let state = PaintState {
            settings: self.settings,
            draft: self.draft.clone(),
            selected: self.selected,
            hue: self.hue,
            sat: self.sat,
            val: self.val,
            cursor: self.cursor,
            remove_armed: self.remove_armed,
            dragging_stop: match self.drag {
                Some(DragKind::Stop { index }) => Some(index),
                _ => None,
            },
            expanded_section: self.expanded_section,
            accordion_anim: self.accordion_anim,
        };
        Ok(ConfigureLayout::compute(
            &state,
            self.settings.show_developer,
        ))
    }

    fn resize_to_content(&self) {
        let Ok(layout) = self.layout() else {
            crate::app_log::warn("configure layout failed during resize");
            return;
        };
        let (max_width, max_height) = self
            .window
            .current_monitor()
            .map(|monitor| {
                let scale = monitor.scale_factor().max(1.0);
                let size = monitor.size();
                (
                    size.width as f64 / scale - layout::SPACE_5 * 2.0,
                    size.height as f64 / scale - layout::SPACE_5 * 2.0,
                )
            })
            .unwrap_or((WIN_W, layout.height));
        let _ = self.window.request_inner_size(LogicalSize::new(
            WIN_W.min(max_width),
            layout.height.min(max_height),
        ));
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
                if self
                    .accordion_anim
                    .is_some_and(|anim| anim.progress() >= 1.0)
                {
                    self.accordion_anim = None;
                }
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
                ElementState::Released => {
                    let action = self.on_release();
                    self.window.request_redraw();
                    return action;
                }
            },
            WindowEvent::ScaleFactorChanged { .. } => {
                self.resize_to_content();
                self.window.request_redraw();
            }
            _ => {}
        }
        ConfigureAction::None
    }

    fn on_release(&mut self) -> ConfigureAction {
        let Some(drag) = self.drag.take() else {
            return ConfigureAction::None;
        };
        if let DragKind::Stop { index } = drag {
            if self.remove_armed {
                if let Ok(next) = self.draft.remove_stop(index) {
                    self.select_stop(next);
                    self.remove_armed = false;
                    self.pending_commit = false;
                    return ConfigureAction::ApplySpectrum(self.draft.clone());
                }
            }
        }
        self.remove_armed = false;
        if self.pending_commit {
            self.pending_commit = false;
            return ConfigureAction::ApplySpectrum(self.draft.clone());
        }
        ConfigureAction::None
    }

    fn on_press(&mut self) -> ConfigureAction {
        let Some((x, y)) = self.cursor else {
            return ConfigureAction::None;
        };
        let Ok(layout) = self.layout() else {
            crate::app_log::warn("configure layout failed during input");
            return ConfigureAction::None;
        };
        if layout.close.contains(x, y) {
            return ConfigureAction::Closed;
        }
        if layout.minimize.contains(x, y) {
            self.window.set_minimized(true);
            return ConfigureAction::None;
        }
        if layout.title_drag.contains(x, y) {
            let _ = self.window.drag_window();
            return ConfigureAction::None;
        }

        for section in &layout.sections {
            if section.header.contains(x, y) {
                if section.section != self.expanded_section {
                    self.accordion_anim = Some(AccordionAnimation {
                        from: self.expanded_section,
                        to: section.section,
                        started_at: Instant::now(),
                    });
                    self.expanded_section = section.section;
                    self.window.request_redraw();
                }
                return ConfigureAction::None;
            }
        }

        if self.expanded_section == AccordionSection::Notifications {
            for setting in [
                NotificationSetting::Connect,
                NotificationSetting::Disconnect,
                NotificationSetting::Low,
                NotificationSetting::Charged,
            ] {
                if layout.notification_row(setting).contains(x, y) {
                    let enabled = !self.notification_enabled(setting);
                    self.set_notification(setting, enabled);
                    return ConfigureAction::SetNotification(setting, enabled);
                }
            }
        }

        if self.expanded_section == AccordionSection::ToastPosition {
            for position in positions() {
                if layout.position_choice(position).contains(x, y) {
                    self.settings.toast_position = position;
                    return ConfigureAction::SelectToastPosition(position);
                }
            }
        }

        #[cfg(windows)]
        if self.expanded_section == AccordionSection::System && layout.autostart_row.contains(x, y)
        {
            self.settings.autostart = !self.settings.autostart;
            return ConfigureAction::SetAutostart(self.settings.autostart);
        }

        #[cfg(feature = "dev-emulate")]
        if self.expanded_section == AccordionSection::Developer && self.settings.show_developer {
            for (index, preset) in Preset::ALL.iter().copied().enumerate() {
                if layout
                    .developer_buttons
                    .get(index)
                    .is_some_and(|rect| rect.contains(x, y))
                {
                    return ConfigureAction::DeveloperPreset(preset);
                }
            }
        }

        if self.expanded_section == AccordionSection::LightbarColors {
            let handles = layout.stop_handles_for(&self.draft);
            for (index, handle) in handles.iter().enumerate() {
                if handle.bounds.contains(x, y) {
                    self.select_stop(index);
                    self.drag = Some(DragKind::Stop { index });
                    self.remove_armed = false;
                    return ConfigureAction::None;
                }
            }
            if layout.spectrum_bar.contains(x, y)
                || (y >= layout.spectrum_bar.y
                    && y <= layout.spectrum_bar.bottom() + STOP_HANDLE_H + 4.0
                    && x >= layout.spectrum_bar.x
                    && x <= layout.spectrum_bar.right())
            {
                let percent = percent_from_bar_x(layout.spectrum_bar, x);
                match self.draft.add_stop_at(percent) {
                    Ok(index) => {
                        self.select_stop(index);
                        return ConfigureAction::ApplySpectrum(self.draft.clone());
                    }
                    Err(_) => {
                        if let Some(index) = self
                            .draft
                            .stops
                            .iter()
                            .position(|stop| stop.percent == percent)
                        {
                            self.select_stop(index);
                        }
                        return ConfigureAction::None;
                    }
                }
            }
            if layout.sv.contains(x, y) {
                self.drag = Some(DragKind::Sv);
                self.apply_drag();
                return ConfigureAction::None;
            }
            if layout.hue.contains(x, y) {
                self.drag = Some(DragKind::Hue);
                self.apply_drag();
                return ConfigureAction::None;
            }
            if layout.reset.contains(x, y) {
                self.draft = BatterySpectrum::default_spectrum();
                self.select_stop(0);
                return ConfigureAction::ApplySpectrum(self.draft.clone());
            }
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

    fn select_stop(&mut self, index: usize) {
        let index = index.min(self.draft.stops.len().saturating_sub(1));
        self.selected = index;
        if let Some(stop) = self.draft.stops.get(index) {
            let (hue, sat, val) = stop.color.to_hsv();
            self.hue = hue;
            self.sat = sat;
            self.val = val;
        }
    }

    fn set_selected_color(&mut self, color: Rgb) {
        if self.draft.set_stop_color(self.selected, color).is_ok() {
            self.pending_commit = true;
        }
    }

    fn apply_drag(&mut self) {
        let Some((x, y)) = self.cursor else {
            return;
        };
        let Ok(layout) = self.layout() else {
            crate::app_log::warn("configure layout failed during drag");
            return;
        };
        match self.drag {
            Some(DragKind::Sv) => {
                let rect = layout.sv;
                self.sat = ((x - rect.x) / rect.w).clamp(0.0, 1.0) as f32;
                self.val = (1.0 - (y - rect.y) / rect.h).clamp(0.0, 1.0) as f32;
                self.set_selected_color(hsv_to_rgb(self.hue, self.sat, self.val));
            }
            Some(DragKind::Hue) => {
                let rect = layout.hue;
                self.hue = ((y - rect.y) / rect.h).clamp(0.0, 1.0) as f32 * 360.0;
                self.set_selected_color(hsv_to_rgb(self.hue, self.sat, self.val));
            }
            Some(DragKind::Stop { index }) => {
                let percent = percent_from_bar_x(layout.spectrum_bar, x);
                let distance = (y - layout.spectrum_bar.bottom()).abs();
                self.remove_armed = distance >= STOP_REMOVE_DISTANCE
                    && self.draft.stops.len() > BatterySpectrum::MIN_STOPS;
                if !self.remove_armed {
                    if let Ok(new_index) = self.draft.set_stop_percent(index, percent) {
                        self.selected = new_index;
                        self.drag = Some(DragKind::Stop { index: new_index });
                        self.pending_commit = true;
                    }
                }
            }
            None => {}
        }
    }

    fn to_logical(&self, position: PhysicalPosition<f64>) -> (f64, f64) {
        let scale = self.window.scale_factor();
        (position.x / scale, position.y / scale)
    }

    fn paint(&mut self) -> Result<(), String> {
        let size = self.window.inner_size();
        let width = NonZeroU32::new(size.width.max(1)).unwrap();
        let height = NonZeroU32::new(size.height.max(1)).unwrap();
        let dragging_stop = match self.drag {
            Some(DragKind::Stop { index }) => Some(index),
            _ => None,
        };
        let state = PaintState {
            settings: self.settings,
            draft: self.draft.clone(),
            selected: self.selected,
            hue: self.hue,
            sat: self.sat,
            val: self.val,
            cursor: self.cursor,
            remove_armed: self.remove_armed,
            dragging_stop,
            expanded_section: self.expanded_section,
            accordion_anim: self.accordion_anim,
        };
        let layout = self.layout()?;
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
        Self::paint_titlebar(&state, &layout, &mut fb);
        for section in &layout.sections {
            paint_section_header(&mut fb, section, &state);
            if state.section_amount(section.section) <= 0.0 {
                continue;
            }
            match section.section {
                AccordionSection::System => Self::paint_system(&state, &layout, &mut fb),
                AccordionSection::Notifications => {
                    Self::paint_notifications(&state, &layout, &mut fb)
                }
                AccordionSection::ToastPosition => paint_position_diagram(&mut fb, &layout, &state),
                AccordionSection::LightbarColors => Self::paint_editor(&state, &layout, &mut fb),
                #[cfg(feature = "dev-emulate")]
                AccordionSection::Developer => Self::paint_developer(&state, &layout, &mut fb),
            }
            cover_section_overflow(&mut fb, section);
        }
        if state.is_animating() {
            self.window.request_redraw();
        }
        buffer.present().map_err(|e| format!("present: {e}"))
    }

    fn paint_titlebar(state: &PaintState, layout: &ConfigureLayout, fb: &mut Framebuffer<'_>) {
        let accent = state.accent();
        fb.fill_rect(0.0, 0.0, WIN_W, layout::WINDOW_HEADER_HEIGHT, ui::PANEL);
        fb.fill_rect(
            0.0,
            0.0,
            layout::SPACE_1,
            layout::WINDOW_HEADER_HEIGHT,
            ui::rgb_of(accent),
        );
        fb.icon(
            layout.title_icon.x,
            layout.title_icon.y,
            layout.title_icon.w.min(layout.title_icon.h),
            accent,
        );
        fb.text_in_rect(
            layout.title_label,
            DISPLAY_NAME,
            ui::INK,
            layout::WINDOW_HEADER_TITLE_SIZE,
            ui::HorizontalAlign::Left,
            ui::VerticalAlign::Center,
        );
        let cursor = state.cursor;
        paint_title_button(
            fb,
            layout.minimize,
            svg_icon::MINIMIZE_SVG,
            cursor.is_some_and(|(x, y)| layout.minimize.contains(x, y)),
            false,
        );
        paint_title_button(
            fb,
            layout.close,
            svg_icon::CLOSE_SVG,
            cursor.is_some_and(|(x, y)| layout.close.contains(x, y)),
            true,
        );
    }

    fn paint_system(state: &PaintState, layout: &ConfigureLayout, fb: &mut Framebuffer<'_>) {
        #[cfg(windows)]
        {
            let row = layout.autostart_row;
            let switch = layout.autostart_switch;
            fb.text_in_rect(
                layout.autostart_label,
                "Start with Windows",
                ui::INK,
                FONT_BODY,
                ui::HorizontalAlign::Left,
                ui::VerticalAlign::Center,
            );
            paint_switch(
                fb,
                switch,
                state.settings.autostart,
                state.cursor.is_some_and(|(x, y)| row.contains(x, y)),
                state.accent(),
            );
        }
        #[cfg(not(windows))]
        {
            let _ = state;
            fb.text(
                layout.autostart_row.x + 12.0,
                layout.autostart_row.y + 14.0,
                "Managed by your operating system",
                ui::MUTED,
                FONT_SMALL,
            );
        }
    }

    fn paint_notifications(state: &PaintState, layout: &ConfigureLayout, fb: &mut Framebuffer<'_>) {
        for setting in [
            NotificationSetting::Connect,
            NotificationSetting::Disconnect,
            NotificationSetting::Low,
            NotificationSetting::Charged,
        ] {
            let row = layout.notification_row(setting);
            let label = match setting {
                NotificationSetting::Connect => "Controller connected",
                NotificationSetting::Disconnect => "Controller disconnected",
                NotificationSetting::Low => "Low battery",
                NotificationSetting::Charged => "Fully charged",
            };
            let index = notification_index(setting);
            let switch = layout.notification_switches[index];
            fb.text_in_rect(
                layout.notification_labels[index],
                label,
                ui::INK,
                FONT_BODY,
                ui::HorizontalAlign::Left,
                ui::VerticalAlign::Center,
            );
            paint_switch(
                fb,
                switch,
                state.notification_enabled(setting),
                state.cursor.is_some_and(|(x, y)| row.contains(x, y)),
                state.accent(),
            );
        }
    }

    fn paint_editor(state: &PaintState, layout: &ConfigureLayout, fb: &mut Framebuffer<'_>) {
        let preview = layout.preview;
        fb.round_rect(
            (preview.x, preview.y, preview.w, preview.h),
            layout::RADIUS_CARD,
            ui::PANEL,
            Some(ui::rgb_of(state.accent())),
        );
        draw_spectrum_bar(fb, layout.spectrum_bar, &state.draft);

        for (index, stop) in state.draft.stops.iter().enumerate() {
            let handle = stop_handle_layout(layout.spectrum_bar, stop.percent);
            let selected = index == state.selected;
            let removing = state.remove_armed && state.dragging_stop == Some(index);
            let hot = state
                .cursor
                .is_some_and(|(x, y)| handle.bounds.contains(x, y));
            let border = if removing {
                ui::rgb(196, 43, 54)
            } else if selected {
                ui::INK
            } else if hot {
                ui::MUTED
            } else {
                ui::LINE
            };
            fb.line(
                (handle.tip.0, handle.tip.1),
                (handle.tip.0, handle.swatch.y),
                1.0,
                border,
            );
            fb.round_rect(
                (
                    handle.swatch.x - 2.0,
                    handle.swatch.y - 2.0,
                    handle.swatch.w + 4.0,
                    handle.swatch.h + 4.0,
                ),
                4.0,
                if removing {
                    ui::rgb(80, 28, 32)
                } else {
                    ui::PANEL
                },
                Some(border),
            );
            fb.round_rect(
                (
                    handle.swatch.x,
                    handle.swatch.y,
                    handle.swatch.w,
                    handle.swatch.h,
                ),
                3.0,
                ui::rgb_of(stop.color),
                None,
            );
        }

        let picker = layout.picker;
        card(fb, picker);
        let label = state
            .selected_stop()
            .map(|stop| format!("{}% · {}", stop.percent, stop.color.to_hex()))
            .unwrap_or_else(|| "Select a stop".to_string());
        fb.text_in_rect(
            layout.picker_label,
            &label,
            ui::INK,
            FONT_BODY,
            ui::HorizontalAlign::Left,
            ui::VerticalAlign::Center,
        );
        draw_sv_square(fb, layout.sv, state.hue, state.sat, state.val);
        draw_hue_strip(fb, layout.hue, state.hue);

        paint_button(
            fb,
            layout.reset,
            "Reset defaults",
            state
                .cursor
                .is_some_and(|(x, y)| layout.reset.contains(x, y)),
            false,
            true,
            state.accent(),
        );
    }

    #[cfg(feature = "dev-emulate")]
    fn paint_developer(state: &PaintState, layout: &ConfigureLayout, fb: &mut Framebuffer<'_>) {
        for (index, preset) in Preset::ALL.iter().copied().enumerate() {
            let rect = layout.developer_buttons[index];
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
                state.accent(),
            );
        }
    }
}

fn positions() -> [ToastPosition; 6] {
    [
        ToastPosition::TopLeft,
        ToastPosition::TopCenter,
        ToastPosition::TopRight,
        ToastPosition::BottomLeft,
        ToastPosition::BottomCenter,
        ToastPosition::BottomRight,
    ]
}

fn card(fb: &mut Framebuffer<'_>, rect: Rect) {
    fb.round_rect(
        (rect.x, rect.y, rect.w, rect.h),
        layout::RADIUS_CARD,
        ui::PANEL,
        Some(ui::LINE),
    );
}

fn section_heading(fb: &mut Framebuffer<'_>, rect: Rect, title: &str) {
    let label = Rect::new(
        rect.x + HEADER_TITLE_PAD_X,
        rect.y,
        (rect.w - HEADER_TITLE_PAD_X * 2.0 - 14.0).max(0.0),
        rect.h,
    );
    fb.text_in_rect(
        label,
        title,
        ui::INK,
        FONT_HEADING,
        ui::HorizontalAlign::Left,
        ui::VerticalAlign::Center,
    );
}

fn paint_section_header(fb: &mut Framebuffer<'_>, section: &SectionLayout, state: &PaintState) {
    let hot = state
        .cursor
        .is_some_and(|(x, y)| section.header.contains(x, y));
    let active = section.section == state.expanded_section;
    if hot || active {
        fb.fill_rect(
            section.header.x,
            section.header.y,
            section.header.w,
            section.header.h,
            if active { ui::PANEL } else { ui::PANEL_HOVER },
        );
    }
    section_heading(fb, section.header, section.section.title());
    paint_accordion_affordance(fb, section.header, state.section_amount(section.section));
    fb.fill_rect(
        section.header.x,
        section.header.bottom(),
        section.header.w,
        1.0,
        ui::LINE,
    );
}

fn cover_section_overflow(fb: &mut Framebuffer<'_>, section: &SectionLayout) {
    if section.visible_height + 0.5 >= section.full_height {
        return;
    }
    let hidden_y = section.content.y + section.visible_height;
    fb.fill_rect(
        section.content.x,
        hidden_y,
        section.content.w,
        (section.full_height - section.visible_height).max(0.0),
        ui::BG,
    );
}

fn paint_accordion_affordance(fb: &mut Framebuffer<'_>, header: Rect, amount: f64) {
    let rect = Rect::new(
        header.right() - HEADER_TITLE_PAD_X - 14.0,
        header.y + (header.h - 14.0) / 2.0,
        14.0,
        14.0,
    );
    let color = ui::MUTED;
    fb.fill_rect(rect.x + 2.0, rect.y + 6.0, 10.0, 2.0, color);
    let vertical_h = ((1.0 - amount) * 10.0).round();
    if vertical_h > 0.0 {
        let top = rect.y + (14.0 - vertical_h) / 2.0;
        fb.fill_rect(rect.x + 6.0, top, 2.0, vertical_h, color);
    }
}

fn paint_position_diagram(fb: &mut Framebuffer<'_>, layout: &ConfigureLayout, state: &PaintState) {
    let stage = layout.position_stage;
    fb.round_rect(
        (stage.x, stage.y, stage.w, stage.h),
        layout::RADIUS_CARD,
        POSITION_STAGE_DARK,
        Some(ui::LINE),
    );
    for position in positions() {
        let hit = layout.position_choice(position);
        let hot = state.cursor.is_some_and(|(x, y)| hit.contains(x, y));
        let selected = position == state.settings.toast_position;
        let radius = (hit.h / 2.0).min(4.0);
        fb.round_rect(
            (hit.x, hit.y, hit.w, hit.h),
            radius,
            if selected {
                ui::rgb_of(state.accent())
            } else if hot {
                ui::PANEL_HOVER
            } else {
                ui::PANEL
            },
            Some(if selected { ui::INK } else { ui::LINE }),
        );
        if selected || hot {
            let rail_w = 2.0;
            let rail_inset = 2.0;
            fb.round_rect(
                (
                    hit.x,
                    hit.y + rail_inset,
                    rail_w,
                    (hit.h - rail_inset * 2.0).max(0.0),
                ),
                rail_w / 2.0,
                if selected { ui::INK } else { ui::MUTED },
                None,
            );
        }
    }
}

fn paint_title_button(fb: &mut Framebuffer<'_>, rect: Rect, svg: &str, hot: bool, danger: bool) {
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
    let inset = 7.0;
    let size = (rect.w.min(rect.h) - inset * 2.0).max(10.0);
    let color = if hot { ui::INK } else { ui::MUTED };
    fb.svg_icon(
        rect.x + (rect.w - size) / 2.0,
        rect.y + (rect.h - size) / 2.0,
        size,
        svg,
        color,
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
    fb.round_rect(
        (rect.x, rect.y, rect.w, rect.h),
        layout::RADIUS_CONTROL,
        fill,
        Some(border),
    );
    fb.text_in_rect(
        rect,
        label,
        ink,
        FONT_BODY,
        ui::HorizontalAlign::Center,
        ui::VerticalAlign::Center,
    );
}

fn draw_spectrum_bar(fb: &mut Framebuffer<'_>, rect: Rect, spectrum: &BatterySpectrum) {
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
        let state = PaintState {
            settings: ConfigureSettings {
                notify_low: true,
                notify_charged: true,
                notify_connect: true,
                notify_disconnect: true,
                toast_position: ToastPosition::TopRight,
                #[cfg(windows)]
                autostart: false,
                show_developer: false,
            },
            draft: BatterySpectrum::default_spectrum(),
            selected: 0,
            hue: 0.0,
            sat: 0.0,
            val: 0.0,
            cursor: None,
            remove_armed: false,
            dragging_stop: None,
            expanded_section: AccordionSection::ToastPosition,
            accordion_anim: None,
        };
        let layout = ConfigureLayout::compute(&state, false);
        let stage = layout.position_stage;
        assert!((stage.w / stage.h - 16.0 / 9.0).abs() < 1e-6);
        assert!(stage.x >= layout.section(AccordionSection::ToastPosition).content.x);
        for position in positions() {
            let rect = layout.position_choice(position);
            assert!((rect.w / rect.h - TOAST_ASPECT).abs() < 1e-6);
            assert!(stage.contains(rect.x + 0.5, rect.y + 0.5));
            assert!(stage.contains(rect.right() - 0.5, rect.bottom() - 0.5));
            let center = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
            assert!(rect.contains(center.0, center.1));
            assert_eq!(
                positions()
                    .into_iter()
                    .filter(|candidate| {
                        layout
                            .position_choice(*candidate)
                            .contains(center.0, center.1)
                    })
                    .count(),
                1
            );
        }
    }

    #[test]
    fn section_content_shares_horizontal_padding() {
        let layout = ConfigureLayout::compute(&preview_state(AccordionSection::System), false);
        let content_left = CONTENT_PAD_X;
        assert!((layout.autostart_row.x - content_left).abs() < 1e-6);
        let notifications =
            ConfigureLayout::compute(&preview_state(AccordionSection::Notifications), false);
        assert!((notifications.notification_rows[0].x - content_left).abs() < 1e-6);
        let toast =
            ConfigureLayout::compute(&preview_state(AccordionSection::ToastPosition), false);
        assert!((toast.position_stage.x - content_left).abs() < 1e-6);
        let lightbar =
            ConfigureLayout::compute(&preview_state(AccordionSection::LightbarColors), false);
        assert!((lightbar.preview.x - content_left).abs() < 1e-6);
    }

    #[test]
    fn title_controls_do_not_overlap_drag_region() {
        let state = preview_state(AccordionSection::System);
        let layout = ConfigureLayout::compute(&state, false);
        assert!(layout.title_drag.right() <= layout.minimize.x);
        assert!(layout.minimize.right() <= layout.close.x);
        assert_eq!(layout.title_drag.h, layout::WINDOW_HEADER_HEIGHT);
    }

    #[test]
    fn first_section_is_default_and_exactly_one_is_open() {
        let state = preview_state(AccordionSection::System);
        let layout = ConfigureLayout::compute(&state, false);
        let expanded = layout
            .sections
            .iter()
            .filter(|section| section.visible_height > 0.0)
            .count();
        assert_eq!(expanded, 1);
        assert_eq!(layout.sections[0].section, AccordionSection::System);
        assert!(layout.section(AccordionSection::System).visible_height > 0.0);
    }

    #[test]
    fn window_height_matches_largest_panel() {
        let system = ConfigureLayout::compute(&preview_state(AccordionSection::System), false);
        let notifications =
            ConfigureLayout::compute(&preview_state(AccordionSection::Notifications), false);
        let position =
            ConfigureLayout::compute(&preview_state(AccordionSection::ToastPosition), false);
        let lightbar =
            ConfigureLayout::compute(&preview_state(AccordionSection::LightbarColors), false);
        assert_eq!(system.height, notifications.height);
        assert_eq!(notifications.height, position.height);
        assert_eq!(position.height, lightbar.height);
    }

    #[test]
    fn stop_handles_track_percent_positions() {
        let layout =
            ConfigureLayout::compute(&preview_state(AccordionSection::LightbarColors), false);
        let spectrum = BatterySpectrum::default_spectrum();
        let handles = layout.stop_handles_for(&spectrum);
        assert_eq!(handles.len(), 3);
        assert!(handles[0].bounds.x < handles[1].bounds.x);
        assert!(handles[1].bounds.x < handles[2].bounds.x);
        for handle in &handles {
            assert!(handle.bounds.bottom() <= layout.height + 1.0);
        }
    }

    #[test]
    fn accordion_animation_splits_heights_between_sections() {
        let mut state = preview_state(AccordionSection::System);
        state.accordion_anim = Some(AccordionAnimation {
            from: AccordionSection::System,
            to: AccordionSection::LightbarColors,
            started_at: Instant::now() - Duration::from_millis(ACCORDION_ANIM_MS / 2),
        });
        state.expanded_section = AccordionSection::LightbarColors;
        let layout = ConfigureLayout::compute(&state, false);
        let system = layout.section(AccordionSection::System);
        let lightbar = layout.section(AccordionSection::LightbarColors);
        assert!(system.visible_height > 0.0);
        assert!(lightbar.visible_height > 0.0);
        assert!(system.visible_height < system.full_height);
        assert!(lightbar.visible_height < lightbar.full_height);
    }

    #[cfg(feature = "dev-emulate")]
    #[test]
    fn developer_controls_are_present_and_bounded() {
        let layout = ConfigureLayout::compute(&preview_state(AccordionSection::Developer), true);
        assert!(
            layout
                .sections
                .iter()
                .any(|section| section.section == AccordionSection::Developer)
        );
        assert!(layout.developer_buttons.len() >= Preset::ALL.len());
        assert!(
            layout
                .developer_buttons
                .iter()
                .all(|rect| rect.bottom() <= layout.height)
        );
    }

    fn preview_state(section: AccordionSection) -> PaintState {
        PaintState {
            settings: ConfigureSettings {
                notify_low: true,
                notify_charged: true,
                notify_connect: true,
                notify_disconnect: true,
                toast_position: ToastPosition::TopRight,
                #[cfg(windows)]
                autostart: false,
                show_developer: false,
            },
            draft: BatterySpectrum::default_spectrum(),
            selected: 0,
            hue: 0.0,
            sat: 0.0,
            val: 0.0,
            cursor: None,
            remove_armed: false,
            dragging_stop: None,
            expanded_section: section,
            accordion_anim: None,
        }
    }
}
