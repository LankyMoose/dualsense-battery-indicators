//! Overlay toast card rendered by the iced daemon.

use crate::theme;
use crate::toast::ToastMessage;
use iced::widget::{column, container, mouse_area, row, space, text};
use iced::{Alignment, Element, Fill, Length, Shrink};

/// Logical width of the toast window.
pub const WIDTH: f32 = 340.0;
/// Logical height of the toast window.
pub const HEIGHT: f32 = 76.0;
/// Gap kept between the toast and the screen edge.
pub const MARGIN: f32 = 16.0;

const RAIL_WIDTH: f32 = 3.0;
const PADDING: f32 = 12.0;
const HEADING_SIZE: f32 = 13.0;
const BODY_SIZE: f32 = 12.0;

/// Renders the toast card. Clicking anywhere on it emits `on_dismiss`.
pub fn view<'a, Message>(message: &'a ToastMessage, on_dismiss: Message) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let accent = theme::from_rgb(message.accent);

    let rail = container(space())
        .width(Length::Fixed(RAIL_WIDTH))
        .height(Fill)
        .style(theme::rail(accent));

    let body = column![
        text(message.heading.as_str())
            .size(HEADING_SIZE)
            .color(theme::INK)
            .width(Fill),
        text(message.body.as_str())
            .size(BODY_SIZE)
            .color(theme::MUTED)
            .width(Fill),
    ]
    .spacing(4)
    .width(Fill);

    let card = container(
        row![rail, body]
            .spacing(PADDING)
            .align_y(Alignment::Center)
            .height(Fill),
    )
    .padding(PADDING)
    .width(Fill)
    .height(Fill)
    .style(theme::toast_card(accent));

    mouse_area(card)
        .on_press(on_dismiss)
        .interaction(iced::mouse::Interaction::Pointer)
        .into()
}

/// Renders nothing when the toast window is open but idle between messages.
pub fn empty<'a, Message: 'a>() -> Element<'a, Message> {
    container(space()).width(Shrink).height(Shrink).into()
}
