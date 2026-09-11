//! Tray-anchored controller overview, rendered by the iced daemon.

use crate::battery::ControllerStatus;
use crate::color::BatterySpectrum;
use crate::known::KnownController;
use crate::svg_icon;
use crate::theme;
use iced::widget::{
    Column, button, checkbox, column, container, row, scrollable, space, svg, text, text_input,
};
use iced::{Alignment, Color, Element, Fill, Length, Shrink};

/// Logical width of the popup window.
pub const WIDTH: f32 = 360.0;
/// Maximum number of rows shown before the list scrolls.
pub const MAX_VISIBLE_ROWS: usize = 6;

const HEADER_HEIGHT: f32 = 38.0;
const ROW_HEIGHT: f32 = 72.0;
const ROW_SPACING: f32 = 6.0;
const PADDING: f32 = 10.0;
const EMPTY_HEIGHT: f32 = 56.0;
const ICON_SIZE: f32 = 18.0;
const GLYPH_SIZE: f32 = 28.0;
const METER_WIDTH: f32 = 132.0;
const METER_HEIGHT: f32 = 6.0;
const NICKNAME_MAX_CHARS: usize = 32;

/// Widget id of the nickname editor, so the daemon can focus it on demand.
pub fn nickname_input_id() -> iced::widget::Id {
    iced::widget::Id::new("popup-nickname")
}

/// A single controller entry: either live, or remembered-but-disconnected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerRow {
    pub serial: String,
    pub product: String,
    pub nickname: Option<String>,
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
        nickname: Option<String>,
    ) -> Self {
        Self {
            serial: controller.serial.clone(),
            product: controller.product.to_string(),
            nickname,
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

    pub fn disconnected(controller: &KnownController, nickname: Option<String>) -> Self {
        Self {
            serial: controller.serial.clone(),
            product: controller.product.clone(),
            nickname,
            connection: controller.connection.clone(),
            state: "disconnected".to_string(),
            percent: controller.percent,
            connected: false,
            remembered: true,
            remember_enabled: true,
            low: false,
        }
    }

    pub fn display_name(&self) -> &str {
        self.nickname
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(self.product.as_str())
    }

    pub fn show_identify(&self) -> bool {
        self.connected
    }

    pub fn show_power_off(&self) -> bool {
        self.connected && self.connection == "Bluetooth"
    }

    pub fn show_edit(&self) -> bool {
        self.remember_enabled
    }
}

/// Transient popup UI state owned by the daemon.
#[derive(Debug, Default)]
pub struct State {
    /// Serial of the row whose nickname is being edited.
    pub editing_serial: Option<String>,
    /// Current nickname draft.
    pub draft: String,
}

impl State {
    pub fn begin_edit(&mut self, serial: &str, current: Option<&str>) {
        self.editing_serial = Some(serial.to_string());
        self.draft = current.unwrap_or_default().to_string();
    }

    pub fn cancel(&mut self) {
        self.editing_serial = None;
        self.draft.clear();
    }

    /// Finish editing and return `(serial, nickname)`; `None` clears the nickname.
    pub fn commit(&mut self) -> Option<(String, Option<String>)> {
        let serial = self.editing_serial.take()?;
        let draft = std::mem::take(&mut self.draft);
        let trimmed = draft.trim().to_string();
        Some((serial, (!trimmed.is_empty()).then_some(trimmed)))
    }

    pub fn is_editing(&self, serial: &str) -> bool {
        self.editing_serial.as_deref() == Some(serial)
    }

    pub fn is_editing_any(&self) -> bool {
        self.editing_serial.is_some()
    }
}

#[derive(Debug, Clone)]
pub enum PopupMessage {
    OpenSettings,
    Identify(String),
    PowerOff(String),
    ToggleRemember(String),
    BeginEdit(String),
    DraftChanged(String),
    CommitNickname,
    CancelEdit,
}

/// Height the popup window should be given for `rows` entries.
pub fn window_height(rows: usize) -> f32 {
    let visible = rows.min(MAX_VISIBLE_ROWS);
    let list = if visible == 0 {
        EMPTY_HEIGHT
    } else {
        visible as f32 * ROW_HEIGHT + (visible.saturating_sub(1) as f32) * ROW_SPACING
    };
    HEADER_HEIGHT + list + PADDING * 2.0
}

pub fn view<'a>(
    state: &'a State,
    rows: &'a [ControllerRow],
    spectrum: &BatterySpectrum,
) -> Element<'a, PopupMessage> {
    let header = row![
        text("Controllers")
            .size(13.0)
            .color(theme::INK)
            .width(Fill),
        icon_button(
            svg_icon::SETTINGS_SVG,
            theme::MUTED,
            PopupMessage::OpenSettings,
        ),
    ]
    .align_y(Alignment::Center)
    .spacing(6)
    .padding([0, 2]);

    let body: Element<'_, PopupMessage> = if rows.is_empty() {
        container(
            text("No DualSense controllers connected")
                .size(12.0)
                .color(theme::DIM),
        )
        .center_x(Fill)
        .center_y(Length::Fixed(EMPTY_HEIGHT))
        .into()
    } else {
        let list = rows
            .iter()
            .fold(Column::new().spacing(ROW_SPACING), |list, entry| {
                list.push(controller_row(state, entry, spectrum))
            })
            .width(Fill);

        scrollable(list).height(Fill).into()
    };

    container(
        column![header, body]
            .spacing(8)
            .width(Fill)
            .height(Fill),
    )
    .padding(PADDING)
    .width(Fill)
    .height(Fill)
    .style(theme::root)
    .into()
}

fn controller_row<'a>(
    state: &'a State,
    entry: &'a ControllerRow,
    spectrum: &BatterySpectrum,
) -> Element<'a, PopupMessage> {
    let accent = theme::from_rgb(spectrum.color_at_percent(entry.percent));
    let glyph_color = if entry.connected { accent } else { theme::DIM };

    let glyph = container(
        svg(svg::Handle::from_memory(svg_icon::DUALSENSE_SVG.as_bytes()))
            .width(Length::Fixed(GLYPH_SIZE))
            .height(Length::Fixed(GLYPH_SIZE))
            .style(move |_theme, _status| svg::Style {
                color: Some(glyph_color),
            }),
    )
    .width(Length::Fixed(GLYPH_SIZE))
    .height(Length::Fixed(GLYPH_SIZE));

    let name: Element<'_, PopupMessage> = if state.is_editing(&entry.serial) {
        text_input("Nickname", &state.draft)
            .id(nickname_input_id())
            .on_input(|value| {
                PopupMessage::DraftChanged(value.chars().take(NICKNAME_MAX_CHARS).collect())
            })
            .on_submit(PopupMessage::CommitNickname)
            .size(12.0)
            .padding([2, 6])
            .width(Fill)
            .style(theme::input)
            .into()
    } else {
        text(entry.display_name())
            .size(13.0)
            .color(if entry.connected {
                theme::INK
            } else {
                theme::MUTED
            })
            .width(Fill)
            .into()
    };

    let mut actions = row![].spacing(2).align_y(Alignment::Center);
    if state.is_editing(&entry.serial) {
        actions = actions.push(icon_button(
            svg_icon::CHECK_SVG,
            theme::SUCCESS,
            PopupMessage::CommitNickname,
        ));
        actions = actions.push(icon_button(
            svg_icon::CLOSE_SVG,
            theme::MUTED,
            PopupMessage::CancelEdit,
        ));
    } else {
        if entry.show_edit() {
            actions = actions.push(icon_button(
                svg_icon::EDIT_SVG,
                theme::MUTED,
                PopupMessage::BeginEdit(entry.serial.clone()),
            ));
        }
        if entry.show_identify() {
            actions = actions.push(icon_button(
                svg_icon::IDENTIFY_SVG,
                theme::MUTED,
                PopupMessage::Identify(entry.serial.clone()),
            ));
        }
        if entry.show_power_off() {
            actions = actions.push(icon_button(
                svg_icon::POWER_SVG,
                theme::MUTED,
                PopupMessage::PowerOff(entry.serial.clone()),
            ));
        }
    }

    let meta = text(format!(
        "{} · {} · {}%",
        entry.connection, entry.state, entry.percent
    ))
    .size(11.0)
    .color(if entry.low { theme::WARNING } else { theme::DIM })
    .width(Fill);

    let remember: Element<'_, PopupMessage> = if entry.remember_enabled {
        let serial = entry.serial.clone();
        checkbox(entry.remembered)
            .label("Remember")
            .size(13.0)
            .text_size(11.0)
            .spacing(5)
            .on_toggle(move |_| PopupMessage::ToggleRemember(serial.clone()))
            .into()
    } else {
        space().into()
    };

    let details = column![
        row![name, actions].spacing(6).align_y(Alignment::Center),
        meter(entry.percent, accent),
        row![meta, remember].spacing(6).align_y(Alignment::Center),
    ]
    .spacing(5)
    .width(Fill);

    container(
        row![glyph, details]
            .spacing(10)
            .align_y(Alignment::Center)
            .width(Fill),
    )
    .padding([8, 10])
    .width(Fill)
    .height(Length::Fixed(ROW_HEIGHT))
    .style(theme::surface)
    .into()
}

fn meter<'a, Message: 'a>(percent: u8, color: Color) -> Element<'a, Message> {
    let filled = METER_WIDTH * (percent.min(100) as f32 / 100.0);

    container(
        container(space().height(Length::Fixed(METER_HEIGHT)))
            .width(Length::Fixed(filled))
            .height(Length::Fixed(METER_HEIGHT))
            .style(theme::meter_fill(color)),
    )
    .width(Length::Fixed(METER_WIDTH))
    .height(Length::Fixed(METER_HEIGHT))
    .style(theme::meter_track)
    .into()
}

fn icon_button<'a>(
    source: &'static str,
    color: Color,
    message: PopupMessage,
) -> Element<'a, PopupMessage> {
    button(
        svg(svg::Handle::from_memory(source.as_bytes()))
            .width(Length::Fixed(ICON_SIZE))
            .height(Length::Fixed(ICON_SIZE))
            .style(move |_theme, _status| svg::Style { color: Some(color) }),
    )
    .padding(4)
    .width(Shrink)
    .height(Shrink)
    .on_press(message)
    .style(theme::ghost)
    .into()
}
