//! Tray-anchored controller overview popup.

use crate::battery::ControllerStatus;
use crate::color::{BatterySpectrum, Rgb};
use crate::known::KnownController;
use crate::svg_icon;
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
const RAIL_W: f64 = 3.0;
const GLYPH_SIZE: f64 = 28.0;
const ACTION_SIZE: f64 = 22.0;
const ROW_INNER_HEIGHT: f64 = 72.0;

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

    pub fn show_identify(&self) -> bool {
        self.connected
    }

    pub fn show_power_off(&self) -> bool {
        self.connected && self.connection == "Bluetooth"
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PopupAction {
    None,
    Identify(String),
    PowerOff(String),
    ToggleRemember(String),
    OpenSettings,
    Closed,
}

#[derive(Clone)]
struct PopupRowLayout {
    bounds: Rect,
    rail: Rect,
    glyph: Rect,
    product: Rect,
    percent: Rect,
    detail: Rect,
    bar: Rect,
    identify: Option<Rect>,
    turn_off: Option<Rect>,
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
    fn compute(font: &Font, rows: &[ControllerRow]) -> Result<Self, String> {
        let visible = &rows[..rows.len().min(MAX_VISIBLE_ROWS)];
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
        if visible.is_empty() {
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
            for row in visible {
                let rail = tree.leaf(popup_fixed(Some(RAIL_W), None, 0.0))?;
                let glyph = tree.leaf(popup_fixed(Some(GLYPH_SIZE), Some(GLYPH_SIZE), 0.0))?;
                let product = tree.text("", 13.0, popup_flexible_text_style(Some(20.0), 1.0))?;
                let identify = if row.show_identify() {
                    Some(tree.leaf(popup_fixed(Some(ACTION_SIZE), Some(ACTION_SIZE), 0.0))?)
                } else {
                    None
                };
                let turn_off = if row.show_power_off() {
                    Some(tree.leaf(popup_fixed(Some(ACTION_SIZE), Some(ACTION_SIZE), 0.0))?)
                } else {
                    None
                };
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

                let mut title_children = vec![glyph, product];
                if let Some(node) = identify {
                    title_children.push(node);
                }
                if let Some(node) = turn_off {
                    title_children.push(node);
                }
                title_children.push(remember);
                let title_row = tree.container(
                    Style {
                        gap: Size {
                            width: LengthPercentage::length(layout::SPACE_2 as f32),
                            height: LengthPercentage::length(0.0),
                        },
                        align_items: Some(AlignItems::CENTER),
                        ..popup_row(Some(GLYPH_SIZE.max(24.0)))
                    },
                    &title_children,
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
                let body = tree.container(
                    Style {
                        padding: taffy::geometry::Rect {
                            left: LengthPercentage::length(layout::SPACE_2 as f32),
                            right: LengthPercentage::length(layout::SPACE_3 as f32),
                            top: LengthPercentage::length(6.0),
                            bottom: LengthPercentage::length(6.0),
                        },
                        gap: Size {
                            width: LengthPercentage::length(0.0),
                            height: LengthPercentage::length(layout::SPACE_1 as f32),
                        },
                        flex_grow: 1.0,
                        ..popup_column(None, 0.0, 0.0)
                    },
                    &[title_row, detail_row, bar],
                )?;
                let row_node = tree.container(
                    Style {
                        display: Display::Flex,
                        flex_direction: FlexDirection::Row,
                        align_items: Some(AlignItems::STRETCH),
                        size: Size {
                            width: Dimension::auto(),
                            height: Dimension::length(ROW_INNER_HEIGHT as f32),
                        },
                        ..Default::default()
                    },
                    &[rail, body],
                )?;
                row_nodes.push(row_node);
                row_parts.push((
                    row_node,
                    rail,
                    glyph,
                    product,
                    percent,
                    detail,
                    bar,
                    identify,
                    turn_off,
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
                    height: LengthPercentage::length(layout::SPACE_2 as f32),
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
                    rail,
                    glyph,
                    product,
                    percent,
                    detail,
                    bar,
                    identify,
                    turn_off,
                    remember,
                    remember_label,
                    remember_check,
                )|
                 -> Result<_, String> {
                    Ok(PopupRowLayout {
                        bounds: tree.rect(bounds)?,
                        rail: tree.rect(rail)?,
                        glyph: tree.rect(glyph)?,
                        product: tree.rect(product)?,
                        percent: tree.rect(percent)?,
                        detail: tree.rect(detail)?,
                        bar: tree.rect(bar)?,
                        identify: identify.map(|n| tree.rect(n)).transpose()?,
                        turn_off: turn_off.map(|n| tree.rect(n)).transpose()?,
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
        let layout = PopupLayout::compute(&font, &[])?;
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
        let layout = match PopupLayout::compute(&self.font, &rows) {
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
                if visible_index + 1 < visible_count {
                    let bounds = layout.rows[visible_index].bounds;
                    let y = bounds.bottom() + layout::SPACE_2 / 2.0;
                    fb.fill_rect(
                        bounds.x + RAIL_W + layout::SPACE_2,
                        y,
                        (bounds.w - RAIL_W - layout::SPACE_2).max(0.0),
                        1.0,
                        ui::LINE,
                    );
                }
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
        paint_action_icon(fb, settings, svg_icon::SETTINGS_SVG, hot);
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
        let rail_color = if row.connected {
            ui::rgb_of(spectrum.color_at_percent(row.percent))
        } else {
            ui::rgb(100, 108, 120)
        };
        let rail = layout.rail;
        fb.round_rect(
            (rail.x, rail.y + 2.0, rail.w, (rail.h - 4.0).max(0.0)),
            (rail.w / 2.0).max(1.0),
            rail_color,
            None,
        );

        let glyph = layout.glyph;
        let glyph_size = glyph.w.min(glyph.h);
        if row.connected {
            fb.icon(
                glyph.x,
                glyph.y,
                glyph_size,
                spectrum.color_at_percent(row.percent),
            );
        } else {
            fb.icon_dim(glyph.x, glyph.y, glyph_size);
        }

        let title_color = if row.connected { ui::INK } else { ui::MUTED };
        fb.text_in_rect(
            layout.product,
            &row.product,
            title_color,
            13.0,
            ui::HorizontalAlign::Left,
            ui::VerticalAlign::Center,
        );

        if let Some(identify) = layout.identify {
            let hot = cursor.is_some_and(|(x, y)| identify.contains(x, y));
            paint_action_icon(fb, identify, svg_icon::IDENTIFY_SVG, hot);
        }
        if let Some(turn_off) = layout.turn_off {
            let hot = cursor.is_some_and(|(x, y)| turn_off.contains(x, y));
            paint_action_icon(fb, turn_off, svg_icon::POWER_SVG, hot);
        }

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
        if row_layout
            .turn_off
            .is_some_and(|rect| rect.contains(x, y))
        {
            return PopupAction::PowerOff(row.serial.clone());
        }
        if row_layout
            .identify
            .is_some_and(|rect| rect.contains(x, y))
        {
            return PopupAction::Identify(row.serial.clone());
        }
    }
    PopupAction::None
}

fn paint_action_icon(fb: &mut Framebuffer<'_>, rect: Rect, svg: &str, hot: bool) {
    fb.round_rect(
        (rect.x, rect.y, rect.w, rect.h),
        6.0,
        if hot { ui::PANEL_HOVER } else { ui::BG },
        None,
    );
    let inset = 3.0;
    let size = (rect.w.min(rect.h) - inset * 2.0).max(8.0);
    let color = if hot { ui::INK } else { ui::MUTED };
    fb.svg_icon(rect.x + inset, rect.y + inset, size, svg, color);
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
        let inset = 2.5;
        let size = (checkbox.w.min(checkbox.h) - inset * 2.0).max(8.0);
        fb.svg_icon(
            checkbox.x + inset,
            checkbox.y + inset,
            size,
            svg_icon::CHECK_SVG,
            ui::INK,
        );
    }
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

    fn sample_row(connection: &str, connected: bool) -> ControllerRow {
        ControllerRow {
            serial: "abc".into(),
            product: "DualSense".into(),
            connection: connection.into(),
            state: if connected {
                "discharging".into()
            } else {
                "disconnected".into()
            },
            percent: 50,
            connected,
            remembered: false,
            remember_enabled: true,
            low: false,
        }
    }

    fn rows_n(n: usize, connection: &str) -> Vec<ControllerRow> {
        (0..n)
            .map(|i| {
                let mut row = sample_row(connection, true);
                row.serial = format!("pad{i}");
                row
            })
            .collect()
    }

    #[test]
    fn popup_height_caps_at_six_rows() {
        let font = ui::load_system_ui_font().unwrap();
        let empty = PopupLayout::compute(&font, &[]).unwrap();
        let two = PopupLayout::compute(&font, &rows_n(2, "USB")).unwrap();
        let many = PopupLayout::compute(&font, &rows_n(20, "USB")).unwrap();
        let one = PopupLayout::compute(&font, &rows_n(1, "USB")).unwrap();
        assert!(empty.height > 100.0);
        assert!(two.height > empty.height);
        assert_eq!(many.rows.len(), MAX_VISIBLE_ROWS);
        assert!(many.height > two.height);
        assert_eq!(one.header.h, layout::WINDOW_HEADER_HEIGHT);
    }

    #[test]
    fn popup_row_children_are_computed_inside_each_row() {
        let font = ui::load_system_ui_font().unwrap();
        let layout = PopupLayout::compute(&font, &rows_n(6, "Bluetooth")).unwrap();
        assert_eq!(layout.rows.len(), 6);
        for row in &layout.rows {
            for child in [
                row.rail,
                row.glyph,
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
                assert!(child.right() <= row.bounds.right() + 0.5);
                assert!(child.bottom() <= row.bounds.bottom() + 0.5);
            }
            assert!(row.identify.is_some());
            assert!(row.turn_off.is_some());
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
        assert!(row.show_identify());
        assert!(!row.show_power_off());
        assert_eq!(row.state, "charging");
    }

    #[test]
    fn action_icons_and_remember_hit_targets() {
        let bt = sample_row("Bluetooth", true);
        let usb = sample_row("USB", true);
        let font = ui::load_system_ui_font().unwrap();

        let bt_layout = PopupLayout::compute(&font, std::slice::from_ref(&bt)).unwrap();
        let remember = bt_layout.rows[0].remember;
        let identify = bt_layout.rows[0].identify.unwrap();
        let power = bt_layout.rows[0].turn_off.unwrap();
        let bounds = bt_layout.rows[0].bounds;

        assert_eq!(
            action_at(
                std::slice::from_ref(&bt),
                0,
                &bt_layout,
                remember.x + remember.w / 2.0,
                remember.y + remember.h / 2.0
            ),
            PopupAction::ToggleRemember("abc".into())
        );
        assert_eq!(
            action_at(
                std::slice::from_ref(&bt),
                0,
                &bt_layout,
                power.x + power.w / 2.0,
                power.y + power.h / 2.0
            ),
            PopupAction::PowerOff("abc".into())
        );
        assert_eq!(
            action_at(
                std::slice::from_ref(&bt),
                0,
                &bt_layout,
                identify.x + identify.w / 2.0,
                identify.y + identify.h / 2.0
            ),
            PopupAction::Identify("abc".into())
        );
        assert_eq!(
            action_at(
                std::slice::from_ref(&bt),
                0,
                &bt_layout,
                bounds.x + 12.0,
                bounds.y + 12.0
            ),
            PopupAction::None
        );

        let usb_layout = PopupLayout::compute(&font, std::slice::from_ref(&usb)).unwrap();
        assert!(usb_layout.rows[0].identify.is_some());
        assert!(usb_layout.rows[0].turn_off.is_none());
        let usb_identify = usb_layout.rows[0].identify.unwrap();
        assert_eq!(
            action_at(
                std::slice::from_ref(&usb),
                0,
                &usb_layout,
                usb_identify.x + usb_identify.w / 2.0,
                usb_identify.y + usb_identify.h / 2.0
            ),
            PopupAction::Identify("abc".into())
        );
    }
}
