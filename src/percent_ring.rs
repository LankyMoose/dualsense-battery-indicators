//! Circular battery-percent outline used by overlay toasts and the controller popup.

use crate::theme;
use iced::mouse;
use iced::widget::canvas as canvas_widget;
use iced::widget::canvas::{self, Frame, Geometry, Path};
use iced::{
    Color, Degrees, Element, Length, Pixels, Point, Radians, Rectangle, Renderer, Theme, alignment,
};

/// Reference size used by overlay toasts; stroke and type scale from this.
pub const TOAST_SIZE: f32 = 60.0;
/// Size that fits a controller-popup row.
pub const POPUP_SIZE: f32 = 56.0;

const REF_SIZE: f32 = 60.0;
const REF_STROKE: f32 = 3.5;
const REF_TEXT: f32 = 20.0;
const REF_TEXT_FULL: f32 = 16.0;
const REF_ETA_TEXT: f32 = 12.0;

/// Draws a circular outline filled clockwise to `percent`, with the value centered inside.
/// When `eta` is set (e.g. `est. 8h`), it sits under the percent inside the ring.
pub fn percent_ring<'a, Message: 'a>(
    percent: u8,
    color: Color,
    size: f32,
    eta: Option<String>,
) -> Element<'a, Message> {
    canvas_widget(PercentRing {
        percent,
        color,
        size,
        eta,
    })
    .width(Length::Fixed(size))
    .height(Length::Fixed(size))
    .into()
}

#[derive(Clone)]
struct PercentRing {
    percent: u8,
    color: Color,
    size: f32,
    eta: Option<String>,
}

impl PercentRing {
    fn stroke_width(&self) -> f32 {
        REF_STROKE * (self.size / REF_SIZE)
    }

    fn text_size(&self, percent: u8) -> f32 {
        let base = if percent >= 100 {
            REF_TEXT_FULL
        } else {
            REF_TEXT
        };
        let scale = if self.eta.is_some() { 0.82 } else { 1.0 };
        base * (self.size / REF_SIZE) * scale
    }

    fn eta_text_size(&self) -> f32 {
        REF_ETA_TEXT * (self.size / REF_SIZE)
    }
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
        let stroke_width = self.stroke_width();
        let radius = (bounds.width.min(bounds.height) - stroke_width) / 2.0;
        if radius <= 0.0 {
            return vec![frame.into_geometry()];
        }

        let percent = self.percent.min(100);
        let track = canvas::Stroke::default()
            .with_color(theme::alpha(self.color, 0.28))
            .with_width(stroke_width)
            .with_line_cap(canvas::LineCap::Round);
        frame.stroke(&Path::circle(center, radius), track);

        let fill = canvas::Stroke::default()
            .with_color(self.color)
            .with_width(stroke_width)
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

        let percent_size = self.text_size(percent);
        let (percent_pos, eta_pos) = if self.eta.is_some() {
            let gap = percent_size * 0.75;
            (
                Point::new(center.x, center.y - gap * 0.42),
                Some(Point::new(center.x, center.y + gap * 0.92)),
            )
        } else {
            (center, None)
        };

        frame.fill_text(canvas::Text {
            content: format!("{percent}%"),
            position: percent_pos,
            color: theme::INK,
            size: Pixels(percent_size),
            align_x: alignment::Horizontal::Center.into(),
            align_y: alignment::Vertical::Center,
            ..canvas::Text::default()
        });

        if let (Some(eta), Some(pos)) = (self.eta.as_ref(), eta_pos) {
            // Between MUTED and INK — slightly closer to muted for contrast with %.
            let eta_color = theme::rgb(200, 206, 214);
            frame.fill_text(canvas::Text {
                content: eta.clone(),
                position: pos,
                color: eta_color,
                size: Pixels(self.eta_text_size()),
                align_x: alignment::Horizontal::Center.into(),
                align_y: alignment::Vertical::Center,
                ..canvas::Text::default()
            });
        }

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
