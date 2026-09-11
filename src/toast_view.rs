//! Overlay toast card rendered by the iced daemon.

use crate::theme;
use crate::toast::ToastMessage;
use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry, Path};
use iced::widget::{canvas as canvas_widget, column, container, mouse_area, row, space, text};
use iced::{
    Alignment, Color, Degrees, Element, Fill, Length, Pixels, Radians, Rectangle, Renderer, Shrink,
    Theme, alignment,
};

/// Logical width of the toast window.
pub const WIDTH: f32 = 340.0;
/// Logical height of the toast window.
pub const HEIGHT: f32 = 84.0;
/// Gap kept between the toast and the screen edge.
pub const MARGIN: f32 = 16.0;

const RAIL_WIDTH: f32 = 3.0;
const PADDING: f32 = 12.0;
const HEADING_SIZE: f32 = 13.0;
const BODY_SIZE: f32 = 12.0;
const RING_SIZE: f32 = 60.0;
const RING_STROKE: f32 = 3.5;
const RING_TEXT_SIZE: f32 = 20.0;
const RING_TEXT_SIZE_FULL: f32 = 16.0;

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
        row![rail, body, percent_ring(message.percent, accent)]
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

fn percent_ring<'a, Message: 'a>(percent: u8, color: Color) -> Element<'a, Message> {
    canvas_widget(PercentRing { percent, color })
        .width(Length::Fixed(RING_SIZE))
        .height(Length::Fixed(RING_SIZE))
        .into()
}

#[derive(Clone, Copy)]
struct PercentRing {
    percent: u8,
    color: Color,
}

impl<Message> canvas::Program<Message> for PercentRing {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let center = frame.center();
        let radius = (bounds.width.min(bounds.height) - RING_STROKE) / 2.0;
        if radius <= 0.0 {
            return vec![frame.into_geometry()];
        }

        let percent = self.percent.min(100);
        let track = canvas::Stroke::default()
            .with_color(theme::alpha(self.color, 0.28))
            .with_width(RING_STROKE)
            .with_line_cap(canvas::LineCap::Round);
        frame.stroke(&Path::circle(center, radius), track);

        let fill = canvas::Stroke::default()
            .with_color(self.color)
            .with_width(RING_STROKE)
            .with_line_cap(canvas::LineCap::Round);
        match filled_end_angle(percent) {
            None => {}
            Some(_) if percent >= 100 => {
                frame.stroke(&Path::circle(center, radius), fill);
            }
            Some(end) => {
                let start = ring_start_angle();
                let arc = Path::new(|builder| {
                    builder.arc(canvas::path::Arc {
                        center,
                        radius,
                        start_angle: start,
                        end_angle: end,
                    });
                });
                frame.stroke(&arc, fill);
            }
        }

        let size = if percent >= 100 {
            RING_TEXT_SIZE_FULL
        } else {
            RING_TEXT_SIZE
        };
        frame.fill_text(canvas::Text {
            content: format!("{percent}%"),
            position: center,
            color: theme::INK,
            size: Pixels(size),
            align_x: alignment::Horizontal::Center.into(),
            align_y: alignment::Vertical::Center,
            ..canvas::Text::default()
        });

        vec![frame.into_geometry()]
    }
}

fn ring_start_angle() -> Radians {
    Radians::from(Degrees(-90.0))
}

/// Clockwise end angle of the filled outline, starting at 12 o'clock.
/// `None` when there is no fill (0%).
fn filled_end_angle(percent: u8) -> Option<Radians> {
    let percent = percent.min(100);
    if percent == 0 {
        return None;
    }
    Some(ring_start_angle() + Degrees(360.0 * f32::from(percent) / 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filled_end_angle_skips_empty() {
        assert!(filled_end_angle(0).is_none());
    }

    #[test]
    fn filled_end_angle_quarter_is_noon_plus_90_degrees() {
        let end = filled_end_angle(25).expect("25% should draw an arc");
        let expected = ring_start_angle() + Degrees(90.0);
        assert!((f32::from(end) - f32::from(expected)).abs() < f32::EPSILON);
    }

    #[test]
    fn filled_end_angle_full_covers_a_turn() {
        let end = filled_end_angle(100).expect("100% should close the ring");
        let expected = ring_start_angle() + Degrees(360.0);
        assert!((f32::from(end) - f32::from(expected)).abs() < f32::EPSILON);
        assert!(filled_end_angle(140).is_some());
    }
}
