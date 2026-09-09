//! Tray-anchored controller overview popup.

use crate::battery::ControllerStatus;
use crate::color::{BatterySpectrum, Rgb};
use crate::known::KnownController;
use crate::ui::layout::{self, LayoutTree, Rect};
use crate::ui::{self, Framebuffer};
use fontdue::Font;
use softbuffer::{Context, Surface};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};
use taffy::prelude::{
    AlignItems, Dimension, Display, FlexDirection, LengthPercentage, Size, Style,
};
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, OwnedDisplayHandle};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId, WindowLevel};

#[cfg(windows)]
use winit::platform::windows::{CornerPreference, WindowAttributesExtWindows};

const WIDTH: f64 = 360.0;
const MAX_VISIBLE_ROWS: usize = 6;

#[derive(Debug, Clone, Copy)]
pub struct TrayAnchor {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerRow {
    pub serial: String,
    pub product: String,
    pub connection: String,
    pub state: String,
    pub percent: u8,
    pub connected: bool,
    pub remembered: bool,
    pub remember_enabled: bool,
    pub low: bool,
}

impl ControllerRow {
    pub fn connected(
        controller: &ControllerStatus,
        remembered: bool,
        remember_enabled: bool,
    ) -> Self {
        Self {
            serial: controller.serial.clone(),
            product: controller.product.to_string(),
            connection: controller.connection.to_string(),
            state: if controller.is_low_battery() {
                "low battery".to_string()
            } else {
                controller.state.as_str().to_string()
            },
            percent: controller.percent,
            connected: true,
            remembered,
            remember_enabled,
            low: controller.is_low_battery(),
        }
    }

    pub fn disconnected(controller: &KnownController) -> Self {
        Self {
            serial: controller.serial.clone(),
            product: controller.product.clone(),
            connection: controller.connection.clone(),
            state: "disconnected".to_string(),
            percent: controller.percent,
            connected: false,
            remembered: true,
            remember_enabled: true,
            low: false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PopupAction {
    None,
    Identify(String),
    ToggleRemember(String),
    OpenSettings,
    Closed,
}

#[derive(Clone)]
struct PopupRowLayout {
    bounds: Rect,
    product: Rect,
    percent: Rect,
    detail: Rect,
    bar: Rect,
    remember: Rect,
    remember_label: Rect,
    remember_check: Rect,
}

#[derive(Clone)]
struct PopupLayout {
    header: Rect,
    icon: Rect,
    title: Rect,
    settings: Rect,
    status: Rect,
    empty_title: Option<Rect>,
    empty_body: Option<Rect>,
    rows: Vec<PopupRowLayout>,
    height: f64,
}

impl PopupLayout {
    fn compute(font: &Font, row_count: usize) -> Result<Self, String> {
        let visible_count = row_count.min(MAX_VISIBLE_ROWS);
        let mut tree = LayoutTree::new();

        let icon = tree.leaf(popup_fixed(
            Some(layout::WINDOW_HEADER_ICON_SIZE),
            Some(layout::WINDOW_HEADER_ICON_SIZE),
            0.0,
        ))?;
        let title = tree.text(
            "Controllers",
            layout::WINDOW_HEADER_TITLE_SIZE,
            popup_fixed(None, Some(layout::WINDOW_HEADER_ACTION_SIZE), 1.0),
        )?;
        let status = tree.text(
            "",
            10.0,
            popup_fixed(Some(60.0), Some(layout::WINDOW_HEADER_ACTION_SIZE), 0.0),
        )?;
        let settings = tree.leaf(popup_fixed(
            Some(layout::WINDOW_HEADER_ACTION_SIZE),
            Some(layout::WINDOW_HEADER_ACTION_SIZE),
            0.0,
        ))?;
        let header = tree.container(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: Some(AlignItems::CENTER),
                size: Size {
                    width: Dimension::length(WIDTH as f32),
                    height: Dimension::length(layout::WINDOW_HEADER_HEIGHT as f32),
                },
                padding: taffy::geometry::Rect {
                    left: LengthPercentage::length(layout::SPACE_3 as f32),
                    right: LengthPercentage::length(layout::SPACE_2 as f32),
                    top: LengthPercentage::length(layout::SPACE_1 as f32),
                    bottom: LengthPercentage::length(layout::SPACE_1 as f32),
                },
                gap: Size {
                    width: LengthPercentage::length(layout::SPACE_2 as f32),
                    height: LengthPercentage::length(0.0),
                },
                ..Default::default()
            },
            &[icon, title, status, settings],
        )?;

        let mut row_nodes = Vec::new();
        let mut row_parts = Vec::new();
        let mut empty_nodes = None;
        if visible_count == 0 {
            let empty_title = tree.text(
                "No controllers connected",
                13.0,
                popup_fixed(None, Some(20.0), 0.0),
            )?;
            let empty_body = tree.text(
                "Connect a DualSense to see it here.",
                11.0,
                popup_fixed(None, Some(18.0), 0.0),
            )?;
            empty_nodes = Some((empty_title, empty_body));
            row_nodes.push(tree.container(
                popup_column(Some(70.0), layout::SPACE_1, layout::SPACE_3),
                &[empty_title, empty_body],
            )?);
        } else {
            for _ in 0..visible_count {
                let product = tree.text("", 13.0, popup_flexible_text_style(Some(20.0), 1.0))?;
                let remember_label =
                    tree.text("Remember", 10.5, popup_fixed(None, Some(20.0), 0.0))?;
                let remember_check = tree.leaf(popup_fixed(Some(20.0), Some(20.0), 0.0))?;
                let remember = tree.container(
                    Style {
                        align_items: Some(AlignItems::CENTER),
                        gap: Size {
                            width: LengthPercentage::length(layout::SPACE_1 as f32),
                            height: LengthPercentage::length(0.0),
                        },
                        ..popup_row(Some(24.0))
                    },
                    &[remember_label, remember_check],
                )?;
                let title_row = tree.container(
                    Style {
                        gap: Size {
                            width: LengthPercentage::length(layout::SPACE_2 as f32),
                            height: LengthPercentage::length(0.0),
                        },
                        align_items: Some(AlignItems::CENTER),
                        ..popup_row(Some(24.0))
                    },
                    &[product, remember],
                )?;
                let detail = tree.text("", 10.5, popup_flexible_text_style(Some(16.0), 1.0))?;
                let percent = tree.text("100%", 11.0, popup_fixed(Some(42.0), Some(16.0), 0.0))?;
                let detail_row = tree.container(
                    Style {
                        gap: Size {
                            width: LengthPercentage::length(layout::SPACE_2 as f32),
                            height: LengthPercentage::length(0.0),
                        },
                        ..popup_row(Some(16.0))
                    },
                    &[detail, percent],
                )?;
                let bar = tree.leaf(popup_fixed(None, Some(6.0), 0.0))?;
                let row = tree.container(
                    Style {
                        padding: taffy::geometry::Rect {
                            left: LengthPercentage::length(layout::SPACE_3 as f32),
                            right: LengthPercentage::length(layout::SPACE_3 as f32),
                            top: LengthPercentage::length(6.0),
                            bottom: LengthPercentage::length(4.0),
                        },
                        gap: Size {
                            width: LengthPercentage::length(0.0),
                            height: LengthPercentage::length(layout::SPACE_1 as f32),
                        },
                        ..popup_column(Some(66.0), 0.0, 0.0)
                    },
                    &[title_row, detail_row, bar],
                )?;
                row_nodes.push(row);
                row_parts.push((
                    row,
                    product,
                    percent,
                    detail,
                    bar,
                    remember,
                    remember_label,
                    remember_check,
                ));
            }
        }

        let content = tree.container(
            Style {
                padding: taffy::geometry::Rect {
                    left: LengthPercentage::length(10.0),
                    right: LengthPercentage::length(10.0),
                    top: LengthPercentage::length(10.0),
                    bottom: LengthPercentage::length(10.0),
                },
                gap: Size {
                    width: LengthPercentage::length(0.0),
                    height: LengthPercentage::length(6.0),
                },
                ..popup_column(None, 0.0, 0.0)
            },
            &row_nodes,
        )?;
        let root = tree.container(
            Style {
                size: Size {
                    width: Dimension::length(WIDTH as f32),
                    height: Dimension::auto(),
                },
                ..popup_column(None, 0.0, 0.0)
            },
            &[header, content],
        )?;
        tree.compute(root, WIDTH, None, font)?;

        let root_rect = tree.rect(root)?;
        let rows = row_parts
            .into_iter()
            .map(
                |(
                    bounds,
                    product,
                    percent,
                    detail,
                    bar,
                    remember,
                    remember_label,
                    remember_check,
                )|
                 -> Result<_, String> {
                    Ok(PopupRowLayout {
                        bounds: tree.rect(bounds)?,
                        product: tree.rect(product)?,
                        percent: tree.rect(percent)?,
                        detail: tree.rect(detail)?,
                        bar: tree.rect(bar)?,
                        remember: tree.rect(remember)?,
                        remember_label: tree.rect(remember_label)?,
                        remember_check: tree.rect(remember_check)?,
                    })
                },
            )
            .collect::<Result<Vec<_>, _>>()?;
        let (empty_title, empty_body) = match empty_nodes {
            Some((title, body)) => (Some(tree.rect(title)?), Some(tree.rect(body)?)),
            None => (None, None),
        };
        Ok(Self {
            header: tree.rect(header)?,
            icon: tree.rect(icon)?,
            title: tree.rect(title)?,
            settings: tree.rect(settings)?,
            status: tree.rect(status)?,
            empty_title,
            empty_body,
            rows,
            height: root_rect.h,
        })
    }
}

fn popup_fixed(width: Option<f64>, height: Option<f64>, grow: f32) -> Style {
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

fn popup_flexible_text_style(height: Option<f64>, grow: f32) -> Style {
    Style {
        min_size: Size {
            width: Dimension::length(0.0),
            height: Dimension::auto(),
        },
        ..popup_fixed(None, height, grow)
    }
}

fn popup_row(height: Option<f64>) -> Style {
    Style {
        display: Display::Flex,
        flex_direction: FlexDirection::Row,
        ..popup_fixed(None, height, 0.0)
    }
}

fn popup_column(height: Option<f64>, gap: f64, padding: f64) -> Style {
    Style {
        display: Display::Flex,
        flex_direction: FlexDirection::Column,
        gap: Size {
            width: LengthPercentage::length(0.0),
            height: LengthPercentage::length(gap as f32),
        },
        padding: layout::points(padding),
        ..popup_fixed(None, height, 0.0)
    }
}

pub struct ControllerPopup {
    window: Rc<Window>,
    surface: Surface<OwnedDisplayHandle, Rc<Window>>,
    font: Font,
    rows: Vec<ControllerRow>,
    spectrum: BatterySpectrum,
    cursor: Option<(f64, f64)>,
    anchor: Option<TrayAnchor>,
    scroll: usize,
    visible: bool,
    last_hidden: Option<Instant>,
    layout: PopupLayout,
}

impl ControllerPopup {
    pub fn open(event_loop: &ActiveEventLoop, display: OwnedDisplayHandle) -> Result<Self, String> {
        let font = ui::load_system_ui_font()?;
        let layout = PopupLayout::compute(&font, 0)?;
        let attrs = WindowAttributes::default()
            .with_title("DualSense controllers")
            .with_inner_size(LogicalSize::new(WIDTH, layout.height))
            .with_resizable(false)
            .with_decorations(false)
            .with_visible(false)
            .with_window_level(WindowLevel::AlwaysOnTop);
        #[cfg(windows)]
        let attrs = attrs
            .with_skip_taskbar(true)
            .with_undecorated_shadow(true)
            .with_corner_preference(CornerPreference::Round);

        let window = Rc::new(
            event_loop
                .create_window(attrs)
                .map_err(|e| format!("create controller popup: {e}"))?,
        );
        let context = Context::new(display).map_err(|e| format!("popup context: {e}"))?;
        let surface =
            Surface::new(&context, window.clone()).map_err(|e| format!("popup surface: {e}"))?;
        Ok(Self {
            window,
            surface,
            font,
            rows: Vec::new(),
            spectrum: BatterySpectrum::default_spectrum(),
            cursor: None,
            anchor: None,
            scroll: 0,
            visible: false,
            last_hidden: None,
            layout,
        })
    }

    pub fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub fn sync(&mut self, rows: Vec<ControllerRow>, spectrum: BatterySpectrum) {
        let layout = match PopupLayout::compute(&self.font, rows.len()) {
            Ok(layout) => layout,
            Err(err) => {
                crate::app_log::warn(format!("controller popup layout failed: {err}"));
                return;
            }
        };
        self.rows = rows;
        self.spectrum = spectrum;
        self.layout = layout;
        self.scroll = self.scroll.min(self.max_scroll());
        if self.visible {
            self.reposition();
            self.window.request_redraw();
        }
    }

    pub fn toggle(&mut self, anchor: TrayAnchor) {
        if self.visible {
            self.hide();
        } else if self
            .last_hidden
            .is_some_and(|instant| instant.elapsed() < Duration::from_millis(250))
        {
            // Clicking the tray icon moves focus first on some Windows builds,
            // so Focused(false) can hide us before the tray click is delivered.
        } else {
            self.show(anchor);
        }
    }

    pub fn show(&mut self, anchor: TrayAnchor) {
        self.anchor = Some(anchor);
        self.visible = true;
        self.reposition();
        self.window.set_visible(true);
        self.window.focus_window();
        self.window.request_redraw();
    }

    pub fn hide(&mut self) {
        self.window.set_visible(false);
        if self.visible {
            self.last_hidden = Some(Instant::now());
        }
        self.visible = false;
        self.cursor = None;
    }

    pub fn handle(&mut self, event: &WindowEvent) -> PopupAction {
        match event {
            WindowEvent::CloseRequested => return PopupAction::Closed,
            WindowEvent::Focused(false) if self.visible => return PopupAction::Closed,
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.logical_key == Key::Named(NamedKey::Escape) =>
            {
                return PopupAction::Closed;
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self.window.scale_factor();
                self.cursor = Some((position.x / scale, position.y / scale));
                self.window.request_redraw();
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor = None;
                self.window.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let direction = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y.signum() as i32,
                    MouseScrollDelta::PixelDelta(position) => position.y.signum() as i32,
                };
                if direction > 0 {
                    self.scroll = self.scroll.saturating_sub(1);
                } else if direction < 0 {
                    self.scroll = (self.scroll + 1).min(self.max_scroll());
                }
                self.window.request_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => return self.on_click(),
            WindowEvent::RedrawRequested => {
                if let Err(err) = self.paint() {
                    crate::app_log::warn(format!("controller popup paint failed: {err}"));
                }
            }
            WindowEvent::Resized(_) => self.window.request_redraw(),
            WindowEvent::ScaleFactorChanged { .. } => {
                self.reposition();
                self.window.request_redraw();
            }
            _ => {}
        }
        PopupAction::None
    }

    fn on_click(&self) -> PopupAction {
        let Some((x, y)) = self.cursor else {
            return PopupAction::None;
        };
        action_at(&self.rows, self.scroll, &self.layout, x, y)
    }

    fn max_scroll(&self) -> usize {
        self.rows.len().saturating_sub(MAX_VISIBLE_ROWS)
    }

    fn logical_height(&self) -> f64 {
        self.layout.height
    }

    fn reposition(&self) {
        let Some(anchor) = self.anchor else {
            return;
        };
        let scale = self.window.scale_factor();
        let width = (WIDTH * scale).round() as u32;
        let height = (self.logical_height() * scale).round() as u32;
        let area = work_area(&self.window, anchor);
        let gap = (layout::SPACE_2 * scale).round() as i32;
        let (x, y) = popup_position(area, anchor, width, height, gap);
        let _ = self
            .window
            .request_inner_size(winit::dpi::PhysicalSize::new(width, height));
        self.window.set_outer_position(PhysicalPosition::new(x, y));
    }

    fn paint(&mut self) -> Result<(), String> {
        let size = self.window.inner_size();
        let width = NonZeroU32::new(size.width.max(1)).unwrap();
        let height = NonZeroU32::new(size.height.max(1)).unwrap();
        let rows = self.rows.clone();
        let spectrum = self.spectrum.clone();
        let cursor = self.cursor;
        let scroll = self.scroll;
        let visible_count = rows.len().min(MAX_VISIBLE_ROWS);
        let layout = self.layout.clone();
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
        Self::paint_header(&mut fb, &layout, &spectrum, cursor, rows.len(), scroll);
        if rows.is_empty() {
            fb.text_in_rect(
                layout.empty_title.unwrap_or_default(),
                "No controllers connected",
                ui::INK,
                13.0,
                ui::HorizontalAlign::Left,
                ui::VerticalAlign::Center,
            );
            fb.text_in_rect(
                layout.empty_body.unwrap_or_default(),
                "Connect a DualSense to see it here.",
                ui::MUTED,
                11.0,
                ui::HorizontalAlign::Left,
                ui::VerticalAlign::Center,
            );
        } else {
            for visible_index in 0..visible_count {
                let row_index = scroll + visible_index;
                Self::paint_row(
                    &mut fb,
                    &layout.rows[visible_index],
                    &rows[row_index],
                    &spectrum,
                    cursor,
                );
            }
        }
        buffer.present().map_err(|e| format!("present: {e}"))
    }

    fn paint_header(
        fb: &mut Framebuffer<'_>,
        layout: &PopupLayout,
        spectrum: &BatterySpectrum,
        cursor: Option<(f64, f64)>,
        row_count: usize,
        scroll: usize,
    ) {
        fb.fill_rect(
            layout.header.x,
            layout.header.y,
            layout.header.w,
            layout.header.h,
            ui::PANEL,
        );
        fb.fill_rect(
            layout.header.x,
            layout.header.y,
            layout::SPACE_1,
            layout.header.h,
            ui::rgb_of(spectrum.accent()),
        );
        fb.icon(
            layout.icon.x,
            layout.icon.y,
            layout.icon.w.min(layout.icon.h),
            spectrum.accent(),
        );
        fb.text_in_rect(
            layout.title,
            "Controllers",
            ui::INK,
            layout::WINDOW_HEADER_TITLE_SIZE,
            ui::HorizontalAlign::Left,
            ui::VerticalAlign::Center,
        );
        let settings = layout.settings;
        let hot = cursor.is_some_and(|(x, y)| settings.contains(x, y));
        fb.round_rect(
            (settings.x, settings.y, settings.w, settings.h),
            8.0,
            if hot { ui::PANEL_HOVER } else { ui::PANEL },
            None,
        );
        paint_settings_cog(fb, settings, hot);
        if row_count > MAX_VISIBLE_ROWS {
            let message = format!(
                "{}–{} of {}",
                scroll + 1,
                scroll + row_count.min(MAX_VISIBLE_ROWS),
                row_count
            );
            fb.text_in_rect(
                layout.status,
                &message,
                ui::DIM,
                10.0,
                ui::HorizontalAlign::Right,
                ui::VerticalAlign::Center,
            );
        }
    }

    fn paint_row(
        fb: &mut Framebuffer<'_>,
        layout: &PopupRowLayout,
        row: &ControllerRow,
        spectrum: &BatterySpectrum,
        cursor: Option<(f64, f64)>,
    ) {
        let rect = layout.bounds;
        let hot = cursor.is_some_and(|(x, y)| rect.contains(x, y));
        fb.round_rect(
            (rect.x, rect.y, rect.w, rect.h),
            8.0,
            if hot && row.connected {
                ui::PANEL_HOVER
            } else {
                ui::PANEL
            },
            Some(ui::LINE),
        );
        let title_color = if row.connected { ui::INK } else { ui::MUTED };
        fb.text_in_rect(
            layout.product,
            &row.product,
            title_color,
            13.0,
            ui::HorizontalAlign::Left,
            ui::VerticalAlign::Center,
        );
        let percent = format!("{}%", row.percent);
        fb.text_in_rect(
            layout.percent,
            &percent,
            title_color,
            12.0,
            ui::HorizontalAlign::Right,
            ui::VerticalAlign::Center,
        );
        let detail = format!("{} · {}", row.connection, row.state);
        fb.text_in_rect(
            layout.detail,
            &detail,
            if row.low {
                ui::rgb(255, 139, 74)
            } else {
                ui::MUTED
            },
            10.5,
            ui::HorizontalAlign::Left,
            ui::VerticalAlign::Center,
        );
        let bar = layout.bar;
        fb.round_rect((bar.x, bar.y, bar.w, bar.h), 3.0, ui::LINE, None);
        let fill_width = bar.w * row.percent.min(100) as f64 / 100.0;
        if fill_width > 0.0 {
            let color = if row.connected {
                spectrum.color_at_percent(row.percent)
            } else {
                Rgb::new(100, 108, 120)
            };
            fb.round_rect(
                (bar.x, bar.y, fill_width, bar.h),
                3.0,
                ui::rgb_of(color),
                None,
            );
        }
        paint_remember(fb, layout.remember_label, layout.remember_check, row);
    }
}

fn action_at(
    rows: &[ControllerRow],
    scroll: usize,
    layout: &PopupLayout,
    x: f64,
    y: f64,
) -> PopupAction {
    if layout.settings.contains(x, y) {
        return PopupAction::OpenSettings;
    }
    for (row, row_layout) in rows
        .iter()
        .skip(scroll)
        .take(MAX_VISIBLE_ROWS)
        .zip(&layout.rows)
    {
        if row_layout.remember.contains(x, y) {
            return if row.remember_enabled {
                PopupAction::ToggleRemember(row.serial.clone())
            } else {
                PopupAction::None
            };
        }
        if row_layout.bounds.contains(x, y) && row.connected {
            return PopupAction::Identify(row.serial.clone());
        }
    }
    PopupAction::None
}

fn paint_remember(fb: &mut Framebuffer<'_>, label: Rect, rect: Rect, row: &ControllerRow) {
    fb.text_in_rect(
        label,
        "Remember",
        if row.remember_enabled {
            ui::MUTED
        } else {
            ui::DIM
        },
        10.5,
        ui::HorizontalAlign::Left,
        ui::VerticalAlign::Center,
    );
    let checkbox = rect.inset(2.0);
    fb.round_rect(
        (checkbox.x, checkbox.y, checkbox.w, checkbox.h),
        4.0,
        if row.remembered {
            ui::rgb(65, 65, 251)
        } else {
            ui::BG
        },
        Some(if row.remember_enabled {
            ui::MUTED
        } else {
            ui::LINE
        }),
    );
    if row.remembered {
        fb.line(
            (checkbox.x + 4.5, checkbox.y + 8.0),
            (checkbox.x + 6.5, checkbox.y + 10.0),
            1.0,
            ui::INK,
        );
        fb.line(
            (checkbox.x + 6.5, checkbox.y + 10.0),
            (checkbox.x + 11.5, checkbox.y + 5.0),
            1.0,
            ui::INK,
        );
    }
}

fn paint_settings_cog(fb: &mut Framebuffer<'_>, rect: Rect, hot: bool) {
    let color = if hot { ui::INK } else { ui::MUTED };
    let background = if hot { ui::PANEL_HOVER } else { ui::PANEL };
    let cx = rect.x + rect.w / 2.0;
    let cy = rect.y + rect.h / 2.0;
    for (from, to) in [
        ((cx, cy - 4.0), (cx, cy - 8.0)),
        ((cx + 4.0, cy), (cx + 8.0, cy)),
        ((cx, cy + 4.0), (cx, cy + 8.0)),
        ((cx - 4.0, cy), (cx - 8.0, cy)),
        ((cx + 3.0, cy - 3.0), (cx + 6.0, cy - 6.0)),
        ((cx + 3.0, cy + 3.0), (cx + 6.0, cy + 6.0)),
        ((cx - 3.0, cy + 3.0), (cx - 6.0, cy + 6.0)),
        ((cx - 3.0, cy - 3.0), (cx - 6.0, cy - 6.0)),
    ] {
        fb.line(from, to, 2.0, color);
    }
    fb.round_rect((cx - 6.0, cy - 6.0, 12.0, 12.0), 6.0, color, None);
    fb.round_rect((cx - 2.5, cy - 2.5, 5.0, 5.0), 2.5, background, None);
}

#[derive(Clone, Copy)]
struct ScreenArea {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

fn popup_position(
    area: ScreenArea,
    anchor: TrayAnchor,
    width: u32,
    height: u32,
    gap: i32,
) -> (i32, i32) {
    let area_right = area.x + area.width as i32;
    let area_bottom = area.y + area.height as i32;
    let anchor_center_x = anchor.x + anchor.width as i32 / 2;
    let anchor_center_y = anchor.y + anchor.height as i32 / 2;
    let x = if anchor_center_x > area.x + area.width as i32 / 2 {
        anchor.x + anchor.width as i32 - width as i32
    } else {
        anchor.x
    }
    .clamp(area.x, (area_right - width as i32).max(area.x));
    let y = if anchor_center_y > area.y + area.height as i32 / 2 {
        anchor.y - height as i32 - gap
    } else {
        anchor.y + anchor.height as i32 + gap
    }
    .clamp(area.y, (area_bottom - height as i32).max(area.y));
    (x, y)
}

#[cfg(not(windows))]
fn work_area(window: &Window, _anchor: TrayAnchor) -> ScreenArea {
    window
        .current_monitor()
        .map(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            ScreenArea {
                x: position.x,
                y: position.y,
                width: size.width,
                height: size.height,
            }
        })
        .unwrap_or(ScreenArea {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        })
}

#[cfg(windows)]
fn work_area(_window: &Window, anchor: TrayAnchor) -> ScreenArea {
    let point = WinPoint {
        x: anchor.x + anchor.width as i32 / 2,
        y: anchor.y + anchor.height as i32 / 2,
    };
    let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTOPRIMARY) };
    let mut info = MonitorInfo {
        size: std::mem::size_of::<MonitorInfo>() as u32,
        monitor: WinRect::default(),
        work: WinRect::default(),
        flags: 0,
    };
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return ScreenArea {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
    }
    ScreenArea {
        x: info.work.left,
        y: info.work.top,
        width: (info.work.right - info.work.left).max(1) as u32,
        height: (info.work.bottom - info.work.top).max(1) as u32,
    }
}

#[cfg(windows)]
#[derive(Clone, Copy)]
#[repr(C)]
struct WinPoint {
    x: i32,
    y: i32,
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
#[link(name = "user32")]
unsafe extern "system" {
    fn MonitorFromPoint(point: WinPoint, flags: u32) -> isize;
    fn GetMonitorInfoW(monitor: isize, info: *mut MonitorInfo) -> i32;
}

#[cfg(windows)]
const MONITOR_DEFAULTTOPRIMARY: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battery::PowerState;

    #[test]
    fn popup_height_caps_at_six_rows() {
        let font = ui::load_system_ui_font().unwrap();
        assert_eq!(PopupLayout::compute(&font, 0).unwrap().height, 130.0);
        assert_eq!(PopupLayout::compute(&font, 2).unwrap().height, 198.0);
        assert_eq!(PopupLayout::compute(&font, 20).unwrap().height, 486.0);
        assert_eq!(
            PopupLayout::compute(&font, 1).unwrap().header.h,
            layout::WINDOW_HEADER_HEIGHT
        );
    }

    #[test]
    fn popup_row_children_are_computed_inside_each_row() {
        let font = ui::load_system_ui_font().unwrap();
        let layout = PopupLayout::compute(&font, 6).unwrap();
        assert_eq!(layout.rows.len(), 6);
        for row in &layout.rows {
            for child in [
                row.product,
                row.percent,
                row.detail,
                row.bar,
                row.remember,
                row.remember_label,
                row.remember_check,
            ] {
                assert!(child.x >= row.bounds.x);
                assert!(child.y >= row.bounds.y);
                assert!(child.right() <= row.bounds.right());
                assert!(child.bottom() <= row.bounds.bottom());
            }
            assert!(row.product.right() <= row.remember.x);
            assert!(row.detail.right() <= row.percent.x);
            assert!(row.remember_label.right() <= row.remember_check.x);
        }
    }

    #[test]
    fn popup_anchors_above_bottom_right_tray() {
        let area = ScreenArea {
            x: 0,
            y: 0,
            width: 1920,
            height: 1040,
        };
        let anchor = TrayAnchor {
            x: 1850,
            y: 1040,
            width: 32,
            height: 32,
        };
        assert_eq!(popup_position(area, anchor, 360, 206, 8), (1522, 826));
    }

    #[test]
    fn popup_placement_handles_each_screen_corner() {
        let area = ScreenArea {
            x: 0,
            y: 0,
            width: 1920,
            height: 1040,
        };
        let anchor = |x, y| TrayAnchor {
            x,
            y,
            width: 32,
            height: 32,
        };
        assert_eq!(popup_position(area, anchor(8, 0), 360, 206, 8), (8, 40));
        assert_eq!(
            popup_position(area, anchor(1880, 0), 360, 206, 8),
            (1552, 40)
        );
        assert_eq!(popup_position(area, anchor(8, 1008), 360, 206, 8), (8, 794));
        assert_eq!(
            popup_position(area, anchor(1880, 1008), 360, 206, 8),
            (1552, 794)
        );
    }

    #[test]
    fn connected_rows_keep_identification_state() {
        let status = ControllerStatus {
            index: 1,
            product: "DualSense",
            connection: "USB",
            serial: "abc".to_string(),
            percent: 55,
            state: PowerState::Charging,
        };
        let row = ControllerRow::connected(&status, true, true);
        assert!(row.connected);
        assert!(row.remembered);
        assert_eq!(row.state, "charging");
    }

    #[test]
    fn remember_hit_takes_precedence_over_identify() {
        let row = ControllerRow {
            serial: "abc".into(),
            product: "DualSense".into(),
            connection: "USB".into(),
            state: "discharging".into(),
            percent: 50,
            connected: true,
            remembered: false,
            remember_enabled: true,
            low: false,
        };
        let font = ui::load_system_ui_font().unwrap();
        let layout = PopupLayout::compute(&font, 1).unwrap();
        let bounds = layout.rows[0].bounds;
        let remember = layout.rows[0].remember;
        let center = (remember.x + remember.w / 2.0, remember.y + remember.h / 2.0);
        assert_eq!(
            action_at(std::slice::from_ref(&row), 0, &layout, center.0, center.1),
            PopupAction::ToggleRemember("abc".into())
        );
        assert_eq!(
            action_at(
                std::slice::from_ref(&row),
                0,
                &layout,
                bounds.x + 12.0,
                bounds.y + 12.0
            ),
            PopupAction::Identify("abc".into())
        );
    }
}
