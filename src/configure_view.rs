//! Configure window UI: accordion sections, notification toggles, toast
//! position picker and the lightbar spectrum editor.

use crate::app_meta::{DISPLAY_NAME, PKG_VERSION};
use crate::color::{BatterySpectrum, GradientStop, hsv_to_rgb};
#[cfg(feature = "dev-emulate")]
use crate::emulate::Preset;
use crate::prefs::ToastPosition;
use crate::svg_icon;
use crate::theme;
use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry, Path};
use iced::widget::{
    Column, Row, button, canvas as canvas_widget, checkbox, column, container, mouse_area, row,
    scrollable, slider, space, svg, text,
};
use iced::{
    Alignment, Color, Element, Event, Fill, Length, Point, Rectangle, Renderer, Size, Theme,
};

/// Logical width of the configure window.
pub const WIDTH: f32 = 320.0;
/// Logical height of the configure window.
pub const HEIGHT: f32 = 560.0;

const BAR_HEIGHT: f32 = 44.0;
const SV_HEIGHT: f32 = 112.0;
const HANDLE_WIDTH: f32 = 10.0;
const HIT_RADIUS: f32 = 12.0;
/// Vertical distance outside the bar that arms stop removal (matches softbuffer UI).
const STOP_REMOVE_DISTANCE: f32 = 28.0;
const ICON_SIZE: f32 = 16.0;
/// Representative toast aspect for the position diagram (max width / typical height).
const TOAST_ASPECT: f32 = 360.0 / 60.0;
const POSITION_TOAST_W: f32 = 56.0;
const POSITION_TOAST_MARGIN: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    System,
    Notifications,
    ToastPosition,
    Lightbar,
    #[cfg(feature = "dev-emulate")]
    Developer,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Notifications => "Notifications",
            Self::ToastPosition => "Toast position",
            Self::Lightbar => "Lightbar colors",
            #[cfg(feature = "dev-emulate")]
            Self::Developer => "Developer",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationSetting {
    Connect,
    Disconnect,
    Low,
    Charged,
}

/// Read-only snapshot of app preferences shown by the configure window.
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

#[derive(Debug, Clone)]
pub enum ConfigureMessage {
    ToggleSection(Section),
    SetNotification(NotificationSetting, bool),
    SetToastPosition(ToastPosition),
    #[cfg(windows)]
    SetAutostart(bool),
    SelectStop(usize),
    /// Move the currently selected stop to `percent` (selection tracks reorder).
    MoveStop(u8),
    /// Insert a stop at the clicked percent on the spectrum bar.
    AddStopAt(u8),
    RemoveStop,
    HueChanged(f32),
    SaturationValueChanged(f32, f32),
    ResetSpectrum,
    #[cfg(feature = "dev-emulate")]
    DeveloperPreset(Preset),
    /// Begin an OS window drag (title-bar press on the undecorated window).
    DragWindow,
    Close,
}

/// Mutable UI state owned by the daemon.
#[derive(Debug)]
pub struct ConfigureState {
    pub section: Option<Section>,
    pub spectrum: BatterySpectrum,
    pub selected: usize,
    pub hue: f32,
    pub saturation: f32,
    pub value: f32,
    pub error: Option<String>,
}

impl ConfigureState {
    pub fn new(spectrum: BatterySpectrum) -> Self {
        let mut state = Self {
            section: Some(Section::System),
            spectrum,
            selected: 0,
            hue: 0.0,
            saturation: 0.0,
            value: 0.0,
            error: None,
        };
        state.sync_hsv();
        state
    }

    fn sync_hsv(&mut self) {
        if self.spectrum.stops.is_empty() {
            return;
        }
        self.selected = self.selected.min(self.spectrum.stops.len() - 1);
        let (hue, saturation, value) = self.spectrum.stops[self.selected].color.to_hsv();
        // Keep the current hue when the color is fully desaturated, otherwise the
        // picker snaps back to red every time the user drags value to zero.
        if saturation > f32::EPSILON || value > f32::EPSILON {
            self.hue = hue;
        }
        self.saturation = saturation;
        self.value = value;
    }

    /// Adopt a spectrum applied elsewhere (e.g. loaded prefs).
    pub fn set_spectrum(&mut self, spectrum: BatterySpectrum) {
        self.spectrum = spectrum;
        self.sync_hsv();
    }

    pub fn toggle_section(&mut self, section: Section) {
        self.section = if self.section == Some(section) {
            None
        } else {
            Some(section)
        };
    }

    pub fn select(&mut self, index: usize) {
        if index < self.spectrum.stops.len() {
            self.selected = index;
            self.sync_hsv();
            self.error = None;
        }
    }

    fn apply_selected_color(&mut self) -> Option<BatterySpectrum> {
        let color = hsv_to_rgb(self.hue, self.saturation, self.value);
        match self.spectrum.set_stop_color(self.selected, color) {
            Ok(()) => {
                self.error = None;
                Some(self.spectrum.clone())
            }
            Err(err) => {
                self.error = Some(err);
                None
            }
        }
    }

    pub fn set_hue(&mut self, hue: f32) -> Option<BatterySpectrum> {
        self.hue = hue.rem_euclid(360.0);
        self.apply_selected_color()
    }

    pub fn set_saturation_value(&mut self, saturation: f32, value: f32) -> Option<BatterySpectrum> {
        self.saturation = saturation.clamp(0.0, 1.0);
        self.value = value.clamp(0.0, 1.0);
        self.apply_selected_color()
    }

    pub fn move_stop(&mut self, percent: u8) -> Option<BatterySpectrum> {
        match self.spectrum.set_stop_percent(self.selected, percent) {
            Ok(new_index) => {
                self.selected = new_index;
                self.error = None;
                Some(self.spectrum.clone())
            }
            Err(err) => {
                self.error = Some(err);
                None
            }
        }
    }

    pub fn add_stop_at(&mut self, percent: u8) -> Option<BatterySpectrum> {
        match self.spectrum.add_stop_at(percent) {
            Ok(index) => {
                self.selected = index;
                self.sync_hsv();
                self.error = None;
                Some(self.spectrum.clone())
            }
            Err(err) => {
                if let Some(index) = self
                    .spectrum
                    .stops
                    .iter()
                    .position(|stop| stop.percent == percent)
                {
                    self.select(index);
                    return None;
                }
                self.error = Some(err);
                None
            }
        }
    }

    pub fn remove_selected(&mut self) -> Option<BatterySpectrum> {
        match self.spectrum.remove_stop(self.selected) {
            Ok(index) => {
                self.selected = index;
                self.sync_hsv();
                self.error = None;
                Some(self.spectrum.clone())
            }
            Err(err) => {
                self.error = Some(err);
                None
            }
        }
    }

    pub fn reset(&mut self) -> BatterySpectrum {
        self.spectrum = BatterySpectrum::default_spectrum();
        self.selected = 0;
        self.sync_hsv();
        self.error = None;
        self.spectrum.clone()
    }
}

/// Midpoint of the widest gap between existing stops.
#[cfg(test)]
pub fn suggested_stop_percent(spectrum: &BatterySpectrum) -> u8 {
    let mut percents: Vec<u8> = spectrum.stops.iter().map(|stop| stop.percent).collect();
    percents.sort_unstable();
    percents.dedup();

    let mut best = 50u8;
    let mut widest = 0u16;
    let mut previous = 0u8;
    for (index, percent) in percents.iter().copied().enumerate() {
        let low = if index == 0 { 0 } else { previous };
        let gap = percent.saturating_sub(low) as u16;
        if gap > widest {
            widest = gap;
            best = low + (gap / 2) as u8;
        }
        previous = percent;
    }
    let tail = 100u16.saturating_sub(previous as u16);
    if tail > widest {
        best = previous + (tail / 2) as u8;
    }

    if percents.contains(&best) {
        (0..=100u8)
            .find(|candidate| !percents.contains(candidate))
            .unwrap_or(best)
    } else {
        best
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub fn view<'a>(
    state: &'a ConfigureState,
    settings: &ConfigureSettings,
) -> Element<'a, ConfigureMessage> {
    let title = mouse_area(
        container(text("Settings").size(15.0).color(theme::INK))
            .width(Fill)
            .padding([4, 0]),
    )
    .on_press(ConfigureMessage::DragWindow)
    .interaction(mouse::Interaction::Grab);

    let header = row![
        title,
        button(
            svg(svg::Handle::from_memory(svg_icon::CLOSE_SVG.as_bytes()))
                .width(Length::Fixed(ICON_SIZE))
                .height(Length::Fixed(ICON_SIZE))
                .style(|_theme, _status| svg::Style {
                    color: Some(theme::MUTED)
                }),
        )
        .padding(4)
        .on_press(ConfigureMessage::Close)
        .style(theme::ghost),
    ]
    .align_y(Alignment::Center)
    .spacing(6);

    let mut sections = Column::new().spacing(6).width(Fill);
    sections = sections.push(section(state, Section::System, system_view(settings)));
    sections = sections.push(section(
        state,
        Section::Notifications,
        notifications_view(settings),
    ));
    sections = sections.push(section(
        state,
        Section::ToastPosition,
        toast_position_view(settings, theme::from_rgb(state.spectrum.accent())),
    ));
    sections = sections.push(section(state, Section::Lightbar, lightbar_view(state)));

    #[cfg(feature = "dev-emulate")]
    if settings.show_developer {
        sections = sections.push(section(state, Section::Developer, developer_view()));
    }
    #[cfg(not(feature = "dev-emulate"))]
    let _ = settings.show_developer;

    container(
        column![header, scrollable(sections).height(Fill)]
            .spacing(10)
            .width(Fill)
            .height(Fill),
    )
    .padding(10)
    .width(Fill)
    .height(Fill)
    .style(theme::root)
    .into()
}

fn section<'a>(
    state: &ConfigureState,
    id: Section,
    content: Element<'a, ConfigureMessage>,
) -> Element<'a, ConfigureMessage> {
    let open = state.section == Some(id);

    let header = button(
        row![
            text(id.title()).size(13.0).width(Fill),
            text(if open { "−" } else { "+" }).size(13.0),
        ]
        .align_y(Alignment::Center),
    )
    .padding([6, 10])
    .width(Fill)
    .on_press(ConfigureMessage::ToggleSection(id))
    .style(theme::section);

    let mut body = Column::new().spacing(6).width(Fill).push(header);
    if open {
        body = body.push(
            container(content)
                .padding([8, 10])
                .width(Fill)
                .style(theme::panel),
        );
    }
    body.into()
}

fn system_view<'a>(settings: &ConfigureSettings) -> Element<'a, ConfigureMessage> {
    let mut items = Column::new().spacing(8).width(Fill);

    #[cfg(windows)]
    {
        items = items.push(
            checkbox(settings.autostart)
                .label("Start with Windows")
                .size(15.0)
                .text_size(12.0)
                .spacing(8)
                .on_toggle(ConfigureMessage::SetAutostart),
        );
    }

    items = items.push(
        text(format!("{DISPLAY_NAME} {PKG_VERSION}"))
            .size(11.0)
            .color(theme::DIM),
    );

    let _ = settings;
    items.into()
}

fn notifications_view<'a>(settings: &ConfigureSettings) -> Element<'a, ConfigureMessage> {
    let toggle = |label: &'static str, value: bool, setting: NotificationSetting| {
        checkbox(value)
            .label(label)
            .size(15.0)
            .text_size(12.0)
            .spacing(8)
            .on_toggle(move |enabled| ConfigureMessage::SetNotification(setting, enabled))
    };

    column![
        toggle(
            "Connected",
            settings.notify_connect,
            NotificationSetting::Connect
        ),
        toggle(
            "Disconnected",
            settings.notify_disconnect,
            NotificationSetting::Disconnect
        ),
        toggle("Low battery", settings.notify_low, NotificationSetting::Low),
        toggle(
            "Finished charging",
            settings.notify_charged,
            NotificationSetting::Charged
        ),
    ]
    .spacing(8)
    .width(Fill)
    .into()
}

fn toast_position_view<'a>(
    settings: &ConfigureSettings,
    accent: Color,
) -> Element<'a, ConfigureMessage> {
    // Match the softbuffer diagram: 16:9 “monitor” with mini toast cards at each corner/edge.
    let stage_width = WIDTH - 40.0;
    let stage_height = stage_width * 9.0 / 16.0;
    let toast_h = (POSITION_TOAST_W / TOAST_ASPECT).max(10.0);

    let marker = |position: ToastPosition| {
        let selected = settings.toast_position == position;
        button(
            row![
                container(space())
                    .width(Length::Fixed(2.0))
                    .height(Fill)
                    .style(theme::position_rail(selected))
            ]
            .padding([2, 0])
            .height(Fill),
        )
        .padding(0)
        .width(Length::Fixed(POSITION_TOAST_W))
        .height(Length::Fixed(toast_h))
        .on_press(ConfigureMessage::SetToastPosition(position))
        .style(theme::position_marker(selected, accent))
    };

    let row_of = |left: ToastPosition, center: ToastPosition, right: ToastPosition| {
        Row::new()
            .spacing(0)
            .width(Fill)
            .align_y(Alignment::Center)
            .push(marker(left))
            .push(space().width(Fill))
            .push(marker(center))
            .push(space().width(Fill))
            .push(marker(right))
    };

    container(
        column![
            row_of(
                ToastPosition::TopLeft,
                ToastPosition::TopCenter,
                ToastPosition::TopRight,
            ),
            space().height(Fill),
            row_of(
                ToastPosition::BottomLeft,
                ToastPosition::BottomCenter,
                ToastPosition::BottomRight,
            ),
        ]
        .width(Fill)
        .height(Fill),
    )
    .padding(POSITION_TOAST_MARGIN)
    .width(Fill)
    .height(Length::Fixed(stage_height))
    .style(theme::position_stage)
    .into()
}

fn lightbar_view<'a>(state: &'a ConfigureState) -> Element<'a, ConfigureMessage> {
    let bar = canvas_widget(SpectrumBar {
        stops: state.spectrum.stops.clone(),
        selected: state.selected,
    })
    .width(Fill)
    .height(Length::Fixed(BAR_HEIGHT));

    let stops = state.spectrum.stops.iter().enumerate().fold(
        Column::new().spacing(4).width(Fill),
        |list, (index, stop)| {
            let selected = index == state.selected;
            list.push(
                button(
                    row![
                        container(space())
                            .width(Length::Fixed(14.0))
                            .height(Length::Fixed(14.0))
                            .style(theme::swatch(theme::from_rgb(stop.color))),
                        text(format!("{}%", stop.percent)).size(11.0).width(Fill),
                        text(stop.color.to_hex()).size(11.0).color(theme::MUTED),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding([4, 6])
                .width(Fill)
                .on_press(ConfigureMessage::SelectStop(index))
                .style(theme::chip(selected)),
            )
        },
    );

    let sv = canvas_widget(SvSquare {
        hue: state.hue,
        saturation: state.saturation,
        value: state.value,
    })
    .width(Fill)
    .height(Length::Fixed(SV_HEIGHT));

    let hue = slider(0.0..=360.0, state.hue, ConfigureMessage::HueChanged).step(1.0_f32);

    let reset = button(text("Reset defaults").size(11.0).center().width(Fill))
        .padding([6, 4])
        .width(Fill)
        .on_press(ConfigureMessage::ResetSpectrum)
        .style(theme::chip(false));

    let mut content = column![
        container(bar).width(Fill).style(theme::well),
        stops,
        container(sv).width(Fill).style(theme::well),
        hue,
        reset,
    ]
    .spacing(8)
    .width(Fill);

    if let Some(error) = state.error.as_deref() {
        content = content.push(text(error).size(11.0).color(theme::WARNING));
    }

    content.into()
}

#[cfg(feature = "dev-emulate")]
fn developer_view<'a>() -> Element<'a, ConfigureMessage> {
    Preset::ALL
        .iter()
        .copied()
        .fold(Column::new().spacing(4).width(Fill), |list, preset| {
            list.push(
                button(text(preset.menu_label()).size(11.0).width(Fill))
                    .padding([5, 8])
                    .width(Fill)
                    .on_press(ConfigureMessage::DeveloperPreset(preset))
                    .style(theme::row_button),
            )
        })
        .into()
}

// ---------------------------------------------------------------------------
// Spectrum bar canvas
// ---------------------------------------------------------------------------

/// Gradient preview with draggable stop handles.
/// Click empty bar to add; drag a stop vertically off the bar to remove.
struct SpectrumBar {
    stops: Vec<GradientStop>,
    selected: usize,
}

#[derive(Debug, Default)]
struct BarState {
    dragging: bool,
    remove_armed: bool,
}

impl SpectrumBar {
    fn spectrum(&self) -> BatterySpectrum {
        BatterySpectrum {
            stops: self.stops.clone(),
        }
    }

    fn percent_at(x: f32, width: f32) -> u8 {
        if width <= 0.0 {
            return 0;
        }
        ((x / width).clamp(0.0, 1.0) * 100.0).round() as u8
    }

    fn handle_x(percent: u8, width: f32) -> f32 {
        width * (percent.min(100) as f32 / 100.0)
    }

    fn hit(&self, x: f32, width: f32) -> Option<usize> {
        self.stops
            .iter()
            .enumerate()
            .map(|(index, stop)| (index, (Self::handle_x(stop.percent, width) - x).abs()))
            .filter(|(_, distance)| *distance <= HIT_RADIUS)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }

    fn outside_vertical_distance(y: f32, bounds: Rectangle) -> f32 {
        if y < bounds.y {
            bounds.y - y
        } else if y > bounds.y + bounds.height {
            y - (bounds.y + bounds.height)
        } else {
            0.0
        }
    }
}

impl canvas::Program<ConfigureMessage> for SpectrumBar {
    type State = BarState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<ConfigureMessage>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let position = cursor.position_in(bounds)?;
                if let Some(index) = self.hit(position.x, bounds.width) {
                    state.dragging = true;
                    state.remove_armed = false;
                    Some(canvas::Action::publish(ConfigureMessage::SelectStop(index)).and_capture())
                } else {
                    let percent = Self::percent_at(position.x, bounds.width);
                    Some(
                        canvas::Action::publish(ConfigureMessage::AddStopAt(percent)).and_capture(),
                    )
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.dragging => {
                let point = cursor.position()?;
                let can_remove = self.stops.len() > BatterySpectrum::MIN_STOPS;
                let remove_armed = can_remove
                    && Self::outside_vertical_distance(point.y, bounds) >= STOP_REMOVE_DISTANCE;
                state.remove_armed = remove_armed;
                if remove_armed {
                    return Some(canvas::Action::request_redraw().and_capture());
                }
                let percent = Self::percent_at(point.x - bounds.x, bounds.width);
                Some(canvas::Action::publish(ConfigureMessage::MoveStop(percent)).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.dragging => {
                let remove = state.remove_armed;
                state.dragging = false;
                state.remove_armed = false;
                if remove {
                    Some(canvas::Action::publish(ConfigureMessage::RemoveStop).and_capture())
                } else {
                    Some(canvas::Action::request_redraw().and_capture())
                }
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let width = bounds.width;
        let height = bounds.height;
        let spectrum = self.spectrum();

        // Draw the gradient as thin columns so it matches `color_at_percent` exactly.
        let columns = width.max(1.0).ceil() as usize;
        let step = width / columns as f32;
        for column in 0..columns {
            let x = column as f32 * step;
            let percent = Self::percent_at(x + step / 2.0, width);
            frame.fill_rectangle(
                Point::new(x, 0.0),
                Size::new(step + 1.0, height),
                theme::from_rgb(spectrum.color_at_percent(percent)),
            );
        }

        for (index, stop) in self.stops.iter().enumerate() {
            let x = Self::handle_x(stop.percent, width);
            let left = (x - HANDLE_WIDTH / 2.0).clamp(0.0, (width - HANDLE_WIDTH).max(0.0));
            let selected = index == self.selected;
            let removing = state.remove_armed && state.dragging && selected;

            let outline = Path::rectangle(
                Point::new(left - 1.0, -1.0),
                Size::new(HANDLE_WIDTH + 2.0, height + 2.0),
            );
            frame.fill(
                &outline,
                if removing {
                    theme::DANGER
                } else if selected {
                    theme::INK
                } else {
                    theme::DIM
                },
            );

            let handle =
                Path::rectangle(Point::new(left, 2.0), Size::new(HANDLE_WIDTH, height - 4.0));
            frame.fill(&handle, theme::from_rgb(stop.color));
        }

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging {
            return if state.remove_armed {
                mouse::Interaction::NotAllowed
            } else {
                mouse::Interaction::Grabbing
            };
        }
        match cursor.position_in(bounds) {
            Some(position) if self.hit(position.x, bounds.width).is_some() => {
                mouse::Interaction::Grab
            }
            Some(_) if self.stops.len() < BatterySpectrum::MAX_STOPS => {
                mouse::Interaction::Crosshair
            }
            _ => mouse::Interaction::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Saturation / value square
// ---------------------------------------------------------------------------

struct SvSquare {
    hue: f32,
    saturation: f32,
    value: f32,
}

#[derive(Debug, Default)]
struct SvState {
    dragging: bool,
}

impl SvSquare {
    fn sample(bounds: Rectangle, cursor: mouse::Cursor) -> Option<(f32, f32)> {
        let point = cursor.position()?;
        let x = ((point.x - bounds.x) / bounds.width.max(1.0)).clamp(0.0, 1.0);
        let y = ((point.y - bounds.y) / bounds.height.max(1.0)).clamp(0.0, 1.0);
        Some((x, 1.0 - y))
    }
}

impl canvas::Program<ConfigureMessage> for SvSquare {
    type State = SvState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<ConfigureMessage>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if cursor.is_over(bounds) =>
            {
                state.dragging = true;
                let (saturation, value) = Self::sample(bounds, cursor)?;
                Some(
                    canvas::Action::publish(ConfigureMessage::SaturationValueChanged(
                        saturation, value,
                    ))
                    .and_capture(),
                )
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.dragging => {
                let (saturation, value) = Self::sample(bounds, cursor)?;
                Some(
                    canvas::Action::publish(ConfigureMessage::SaturationValueChanged(
                        saturation, value,
                    ))
                    .and_capture(),
                )
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.dragging => {
                state.dragging = false;
                Some(canvas::Action::request_redraw().and_capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let size = bounds.size();
        let pure = theme::from_rgb(hsv_to_rgb(self.hue, 1.0, 1.0));

        let saturation =
            canvas::gradient::Linear::new(Point::new(0.0, 0.0), Point::new(size.width, 0.0))
                .add_stop(0.0, Color::WHITE)
                .add_stop(1.0, pure);
        frame.fill_rectangle(Point::ORIGIN, size, saturation);

        let value =
            canvas::gradient::Linear::new(Point::new(0.0, 0.0), Point::new(0.0, size.height))
                .add_stop(0.0, Color::TRANSPARENT)
                .add_stop(1.0, Color::BLACK);
        frame.fill_rectangle(Point::ORIGIN, size, value);

        let cursor_point = Point::new(
            self.saturation.clamp(0.0, 1.0) * size.width,
            (1.0 - self.value.clamp(0.0, 1.0)) * size.height,
        );
        frame.stroke(
            &Path::circle(cursor_point, 6.0),
            canvas::Stroke::default()
                .with_color(Color::WHITE)
                .with_width(2.0),
        );
        frame.stroke(
            &Path::circle(cursor_point, 7.5),
            canvas::Stroke::default()
                .with_color(theme::alpha(Color::BLACK, 0.6))
                .with_width(1.0),
        );

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.is_over(bounds) {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggested_percent_lands_in_the_widest_gap() {
        let spectrum = BatterySpectrum::default_spectrum();
        let percent = suggested_stop_percent(&spectrum);
        assert!(percent > 0 && percent < 100);
        assert!(!spectrum.stops.iter().any(|stop| stop.percent == percent));
    }

    #[test]
    fn state_tracks_selection_after_reorder() {
        let mut state = ConfigureState::new(BatterySpectrum::default_spectrum());
        state.select(1);
        let color = state.spectrum.stops[1].color;
        assert!(state.move_stop(5).is_some());
        assert_eq!(state.spectrum.stops[state.selected].color, color);
    }

    #[test]
    fn removing_below_the_minimum_reports_an_error() {
        let mut state = ConfigureState::new(BatterySpectrum::default_spectrum());
        assert!(state.remove_selected().is_some());
        assert!(state.remove_selected().is_none());
        assert!(state.error.is_some());
    }
}
