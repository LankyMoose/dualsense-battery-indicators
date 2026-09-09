//! Custom-painted dark configuration window.

use crate::app_meta::DISPLAY_NAME;
use crate::color::{BatterySpectrum, Rgb, hsv_to_rgb};
#[cfg(feature = "dev-emulate")]
use crate::emulate::Preset;
use crate::prefs::ToastPosition;
use crate::ui::layout::{self, LayoutTree, Rect};
use crate::ui::{self, Framebuffer};
use fontdue::Font;
use softbuffer::{Context, Surface};
use std::num::NonZeroU32;
use std::rc::Rc;
use taffy::prelude::{
    AlignItems, Dimension, Display, FlexDirection, LengthPercentage, NodeId, Size, Style,
};
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
    picker_open: bool,
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

#[derive(Clone)]
struct ConfigureLayout {
    title_drag: Rect,
    title_icon: Rect,
    title_label: Rect,
    minimize: Rect,
    close: Rect,
    notifications_heading: Rect,
    notification_rows: [Rect; 4],
    notification_labels: [Rect; 4],
    notification_switches: [Rect; 4],
    position_heading: Rect,
    position_choices: [Rect; 4],
    system_heading: Rect,
    autostart_row: Rect,
    autostart_label: Rect,
    autostart_switch: Rect,
    editor_heading: Rect,
    preview: Rect,
    spectrum_bar: Rect,
    stops: [StopLayout; 3],
    picker: Rect,
    picker_label: Rect,
    sv: Rect,
    hue: Rect,
    reset: Rect,
    apply: Rect,
    picker_open: bool,
    height: f64,
    separators: Vec<Rect>,
    #[cfg(feature = "dev-emulate")]
    developer_heading: Option<Rect>,
    #[cfg(feature = "dev-emulate")]
    developer_buttons: Vec<Rect>,
}

#[derive(Clone, Copy)]
struct StopLayout {
    bounds: Rect,
    swatch: Rect,
    label: Rect,
    percent: Rect,
    hex: Rect,
}

type StopNodes = (NodeId, NodeId, NodeId, NodeId, NodeId);

impl ConfigureLayout {
    fn compute(font: &Font, picker_open: bool, show_developer: bool) -> Result<Self, String> {
        let mut tree = LayoutTree::new();
        let title_icon = tree.leaf(fixed_style(
            Some(layout::WINDOW_HEADER_ICON_SIZE),
            Some(layout::WINDOW_HEADER_ICON_SIZE),
            0.0,
        ))?;
        let title_label = tree.text(
            DISPLAY_NAME,
            layout::WINDOW_HEADER_TITLE_SIZE,
            Style {
                min_size: Size {
                    width: Dimension::length(0.0),
                    height: Dimension::auto(),
                },
                flex_shrink: 1.0,
                ..fixed_style(None, Some(layout::WINDOW_HEADER_ACTION_SIZE), 0.0)
            },
        )?;
        let title_space = tree.leaf(fixed_style(
            None,
            Some(layout::WINDOW_HEADER_ACTION_SIZE),
            1.0,
        ))?;
        let drag = tree.container(
            Style {
                align_items: Some(AlignItems::CENTER),
                min_size: Size {
                    width: Dimension::length(0.0),
                    height: Dimension::auto(),
                },
                flex_shrink: 1.0,
                padding: taffy::geometry::Rect {
                    left: LengthPercentage::length(layout::SPACE_3 as f32),
                    right: LengthPercentage::length(0.0),
                    top: LengthPercentage::length(layout::SPACE_1 as f32),
                    bottom: LengthPercentage::length(layout::SPACE_1 as f32),
                },
                gap: Size {
                    width: LengthPercentage::length(layout::WINDOW_HEADER_PADDING as f32),
                    height: LengthPercentage::length(0.0),
                },
                ..row_style(None, Some(layout::WINDOW_HEADER_HEIGHT), 0.0, 1.0)
            },
            &[title_icon, title_label, title_space],
        )?;
        let minimize = tree.leaf(fixed_style(
            Some(layout::WINDOW_HEADER_ACTION_SIZE),
            Some(layout::WINDOW_HEADER_ACTION_SIZE),
            0.0,
        ))?;
        let close = tree.leaf(fixed_style(
            Some(layout::WINDOW_HEADER_ACTION_SIZE),
            Some(layout::WINDOW_HEADER_ACTION_SIZE),
            0.0,
        ))?;
        let title_actions = tree.container(
            Style {
                align_items: Some(AlignItems::CENTER),
                gap: Size {
                    width: LengthPercentage::length(layout::SPACE_1 as f32),
                    height: LengthPercentage::length(0.0),
                },
                padding: layout::points(layout::SPACE_1),
                ..row_style(None, Some(layout::WINDOW_HEADER_HEIGHT), 0.0, 0.0)
            },
            &[minimize, close],
        )?;
        let title = tree.container(
            row_style(Some(WIN_W), Some(layout::WINDOW_HEADER_HEIGHT), 0.0, 0.0),
            &[drag, title_actions],
        )?;

        let notifications_heading = compact_heading_node(&mut tree, "Notifications")?;
        let notification_parts = [
            toggle_row(&mut tree, "Controller connected", 29.0)?,
            toggle_row(&mut tree, "Controller disconnected", 29.0)?,
            toggle_row(&mut tree, "Low battery", 29.0)?,
            toggle_row(&mut tree, "Fully charged", 29.0)?,
        ];
        let notification_rows = notification_parts.map(|part| part.0);
        let notifications_card = tree.container(
            column_style(None, Some(116.0), 0.0, 0.0),
            &notification_rows,
        )?;
        let notifications_section = tree.container(
            column_style(None, None, layout::SPACE_1, 0.0),
            &[notifications_heading, notifications_card],
        )?;

        let position_heading = compact_heading_node(&mut tree, "Toast position")?;
        let position_choices = [
            tree.leaf(fixed_style(Some(64.0), Some(36.0), 0.0))?,
            tree.leaf(fixed_style(Some(64.0), Some(36.0), 0.0))?,
            tree.leaf(fixed_style(Some(64.0), Some(36.0), 0.0))?,
            tree.leaf(fixed_style(Some(64.0), Some(36.0), 0.0))?,
        ];
        let position_top = tree.container(
            row_style(Some(132.0), Some(36.0), layout::SPACE_1, 0.0),
            &position_choices[..2],
        )?;
        let position_bottom = tree.container(
            row_style(Some(132.0), Some(36.0), layout::SPACE_1, 0.0),
            &position_choices[2..],
        )?;
        let position_card = tree.container(
            Style {
                align_items: Some(AlignItems::CENTER),
                justify_content: Some(taffy::prelude::JustifyContent::CENTER),
                gap: Size {
                    width: LengthPercentage::length(0.0),
                    height: LengthPercentage::length(layout::SPACE_1 as f32),
                },
                ..column_style(None, Some(76.0), 0.0, 0.0)
            },
            &[position_top, position_bottom],
        )?;
        let position_section = tree.container(
            column_style(None, None, layout::SPACE_1, 0.0),
            &[position_heading, position_card],
        )?;
        let position_row =
            tree.container(row_style(None, Some(98.0), 0.0, 0.0), &[position_section])?;

        let editor_heading = compact_heading_node(&mut tree, "Lightbar colors")?;
        let spectrum_bar = tree.leaf(fixed_style(None, Some(24.0), 1.0))?;
        let preview = tree.container(
            Style {
                align_items: Some(AlignItems::CENTER),
                padding: layout::points(layout::SPACE_3),
                ..row_style(None, Some(48.0), 0.0, 0.0)
            },
            &[spectrum_bar],
        )?;

        let stop_parts = [
            compact_stop_node(&mut tree, "Full", "100%")?,
            compact_stop_node(&mut tree, "Mid", "50%")?,
            compact_stop_node(&mut tree, "Empty", "0%")?,
        ];
        let stops = stop_parts.map(|part| part.0);
        let stop_row = tree.container(row_style(None, Some(54.0), layout::SPACE_2, 0.0), &stops)?;
        let picker_heading = tree.text(
            "Editing battery stop",
            FONT_BODY,
            fixed_style(None, Some(18.0), 0.0),
        )?;
        let sv = tree.leaf(fixed_style(None, Some(100.0), 1.0))?;
        let hue = tree.leaf(fixed_style(Some(18.0), Some(100.0), 0.0))?;
        let picker_controls =
            tree.container(row_style(None, Some(100.0), 10.0, 0.0), &[sv, hue])?;
        let picker = tree.container(
            column_style(None, Some(146.0), layout::SPACE_1, 10.0),
            &[picker_heading, picker_controls],
        )?;
        let reset = tree.leaf(fixed_style(Some(124.0), Some(32.0), 0.0))?;
        let action_space = tree.leaf(fixed_style(None, Some(32.0), 1.0))?;
        let apply = tree.leaf(fixed_style(Some(92.0), Some(32.0), 0.0))?;
        let actions = tree.container(
            row_style(None, Some(32.0), 0.0, 0.0),
            &[reset, action_space, apply],
        )?;
        let editor_summary = tree.container(
            column_style(None, Some(70.0), layout::SPACE_1, 0.0),
            &[editor_heading, preview],
        )?;
        let editor_details = tree.container(
            column_style(None, None, 6.0, 0.0),
            &[stop_row, picker, actions],
        )?;

        let system_heading = compact_heading_node(&mut tree, "System")?;
        let (autostart_row, autostart_label, autostart_switch) =
            toggle_row(&mut tree, "Start with Windows", 40.0)?;
        let system_card =
            tree.container(column_style(None, Some(40.0), 0.0, 0.0), &[autostart_row])?;
        let system_section = tree.container(
            column_style(None, None, layout::SPACE_1, 0.0),
            &[system_heading, system_card],
        )?;

        let separator_one = tree.leaf(fixed_style(None, Some(1.0), 0.0))?;
        let separator_two = tree.leaf(fixed_style(None, Some(1.0), 0.0))?;
        let separator_three = tree.leaf(fixed_style(None, Some(1.0), 0.0))?;
        let separators = vec![separator_one, separator_two, separator_three];
        let mut body_children = vec![
            system_section,
            separator_one,
            notifications_section,
            separator_two,
            position_row,
            separator_three,
            editor_summary,
        ];
        if picker_open {
            body_children.push(editor_details);
        }
        #[cfg(feature = "dev-emulate")]
        let (developer_heading_node, developer_button_nodes, developer_separator_node) =
            if show_developer {
                let heading = compact_heading_node(&mut tree, "Developer")?;
                let mut buttons = Vec::new();
                let mut rows = Vec::new();
                for count in [4, 4] {
                    let mut row_buttons = Vec::new();
                    for _ in 0..count {
                        let button = tree.leaf(fixed_style(None, Some(30.0), 1.0))?;
                        buttons.push(button);
                        row_buttons.push(button);
                    }
                    rows.push(tree.container(
                        row_style(None, Some(30.0), layout::SPACE_2, 0.0),
                        &row_buttons,
                    )?);
                }
                let section = tree.container(
                    column_style(None, Some(88.0), 5.0, 0.0),
                    &[heading, rows[0], rows[1]],
                )?;
                let separator = tree.leaf(fixed_style(None, Some(1.0), 0.0))?;
                body_children.extend([separator, section]);
                (Some(heading), buttons, Some(separator))
            } else {
                (None, Vec::new(), None)
            };
        #[cfg(feature = "dev-emulate")]
        let separators = separators
            .into_iter()
            .chain(developer_separator_node)
            .collect::<Vec<_>>();
        #[cfg(not(feature = "dev-emulate"))]
        let _ = show_developer;

        let body = tree.container(
            Style {
                padding: layout::points(10.0),
                gap: Size {
                    width: LengthPercentage::length(0.0),
                    height: LengthPercentage::length(6.0),
                },
                ..column_style(Some(WIN_W), None, 0.0, 0.0)
            },
            &body_children,
        )?;
        let root = tree.container(column_style(Some(WIN_W), None, 0.0, 0.0), &[title, body])?;
        tree.compute(root, WIN_W, None, font)?;
        let root_rect = tree.rect(root)?;

        Ok(Self {
            title_drag: tree.rect(drag)?,
            title_icon: tree.rect(title_icon)?,
            title_label: tree.rect(title_label)?,
            minimize: tree.rect(minimize)?,
            close: tree.rect(close)?,
            notifications_heading: tree.rect(notifications_heading)?,
            notification_rows: rect_array(&tree, notification_rows)?,
            notification_labels: rect_array(&tree, notification_parts.map(|part| part.1))?,
            notification_switches: rect_array(&tree, notification_parts.map(|part| part.2))?,
            position_heading: tree.rect(position_heading)?,
            position_choices: rect_array(&tree, position_choices)?,
            system_heading: tree.rect(system_heading)?,
            autostart_row: tree.rect(autostart_row)?,
            autostart_label: tree.rect(autostart_label)?,
            autostart_switch: tree.rect(autostart_switch)?,
            editor_heading: tree.rect(editor_heading)?,
            preview: tree.rect(preview)?,
            spectrum_bar: tree.rect(spectrum_bar)?,
            stops: stop_layout_array(&tree, stop_parts)?,
            picker: tree.rect(picker)?,
            picker_label: tree.rect(picker_heading)?,
            sv: tree.rect(sv)?,
            hue: tree.rect(hue)?,
            reset: tree.rect(reset)?,
            apply: tree.rect(apply)?,
            picker_open,
            height: root_rect.h,
            separators: separators
                .into_iter()
                .map(|node| tree.rect(node))
                .collect::<Result<_, _>>()?,
            #[cfg(feature = "dev-emulate")]
            developer_heading: developer_heading_node
                .map(|node| tree.rect(node))
                .transpose()?,
            #[cfg(feature = "dev-emulate")]
            developer_buttons: developer_button_nodes
                .into_iter()
                .map(|node| tree.rect(node))
                .collect::<Result<_, _>>()?,
        })
    }

    fn notification_row(&self, setting: NotificationSetting) -> Rect {
        self.notification_rows[notification_index(setting)]
    }

    fn position_choice(&self, position: ToastPosition) -> Rect {
        self.position_choices[position_index(position)]
    }

    fn stop(&self, stop: Stop) -> Rect {
        self.stops[stop_index(stop)].bounds
    }
}

fn rect_array<const N: usize>(
    tree: &LayoutTree,
    nodes: [taffy::prelude::NodeId; N],
) -> Result<[Rect; N], String> {
    let mut rects = [Rect::default(); N];
    for (index, node) in nodes.into_iter().enumerate() {
        rects[index] = tree.rect(node)?;
    }
    Ok(rects)
}

fn stop_layout_array(tree: &LayoutTree, nodes: [StopNodes; 3]) -> Result<[StopLayout; 3], String> {
    let mut layouts = [StopLayout {
        bounds: Rect::default(),
        swatch: Rect::default(),
        label: Rect::default(),
        percent: Rect::default(),
        hex: Rect::default(),
    }; 3];
    for (index, (bounds, swatch, label, percent, hex)) in nodes.into_iter().enumerate() {
        layouts[index] = StopLayout {
            bounds: tree.rect(bounds)?,
            swatch: tree.rect(swatch)?,
            label: tree.rect(label)?,
            percent: tree.rect(percent)?,
            hex: tree.rect(hex)?,
        };
    }
    Ok(layouts)
}

fn fixed_style(width: Option<f64>, height: Option<f64>, grow: f32) -> Style {
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

fn row_style(width: Option<f64>, height: Option<f64>, gap: f64, grow: f32) -> Style {
    Style {
        display: Display::Flex,
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::STRETCH),
        gap: Size {
            width: LengthPercentage::length(gap as f32),
            height: LengthPercentage::length(0.0),
        },
        ..fixed_style(width, height, grow)
    }
}

fn column_style(width: Option<f64>, height: Option<f64>, gap: f64, padding: f64) -> Style {
    Style {
        display: Display::Flex,
        flex_direction: FlexDirection::Column,
        gap: Size {
            width: LengthPercentage::length(0.0),
            height: LengthPercentage::length(gap as f32),
        },
        padding: layout::points(padding),
        ..fixed_style(width, height, 0.0)
    }
}

fn compact_heading_node(tree: &mut LayoutTree, text: &str) -> Result<NodeId, String> {
    tree.text(text, FONT_HEADING, fixed_style(None, Some(18.0), 0.0))
}

fn toggle_row(
    tree: &mut LayoutTree,
    label: &str,
    height: f64,
) -> Result<(NodeId, NodeId, NodeId), String> {
    let label = tree.text(
        label,
        FONT_BODY,
        Style {
            min_size: Size {
                width: Dimension::length(0.0),
                height: Dimension::auto(),
            },
            flex_basis: Dimension::length(0.0),
            ..fixed_style(None, Some(height), 1.0)
        },
    )?;
    let switch = tree.leaf(fixed_style(Some(38.0), Some(20.0), 0.0))?;
    let row = tree.container(
        Style {
            align_items: Some(AlignItems::CENTER),
            padding: taffy::geometry::Rect {
                left: LengthPercentage::length(layout::SPACE_3 as f32),
                right: LengthPercentage::length(10.0),
                top: LengthPercentage::length(0.0),
                bottom: LengthPercentage::length(0.0),
            },
            gap: Size {
                width: LengthPercentage::length(layout::SPACE_2 as f32),
                height: LengthPercentage::length(0.0),
            },
            ..row_style(None, Some(height), 0.0, 0.0)
        },
        &[label, switch],
    )?;
    Ok((row, label, switch))
}

fn compact_stop_node(
    tree: &mut LayoutTree,
    label: &str,
    percent: &str,
) -> Result<StopNodes, String> {
    let swatch = tree.leaf(fixed_style(Some(26.0), Some(26.0), 0.0))?;
    let label_node = tree.text(label, FONT_BODY, fixed_style(None, Some(13.0), 0.0))?;
    let percent_node = tree.text(percent, FONT_SMALL, fixed_style(None, Some(13.0), 0.0))?;
    let labels = tree.container(
        column_style(None, Some(26.0), 0.0, 0.0),
        &[label_node, percent_node],
    )?;
    let header = tree.container(
        Style {
            gap: Size {
                width: LengthPercentage::length(layout::SPACE_2 as f32),
                height: LengthPercentage::length(0.0),
            },
            ..row_style(None, Some(26.0), 0.0, 0.0)
        },
        &[swatch, labels],
    )?;
    let hex = tree.text("#000000", FONT_SMALL, fixed_style(None, Some(12.0), 0.0))?;
    let bounds = tree.container(
        Style {
            flex_grow: 1.0,
            padding: layout::points(6.0),
            gap: Size {
                width: LengthPercentage::length(0.0),
                height: LengthPercentage::length(layout::SPACE_1 as f32),
            },
            ..column_style(None, Some(54.0), 0.0, 0.0)
        },
        &[header, hex],
    )?;
    Ok((bounds, swatch, label_node, percent_node, hex))
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
        ToastPosition::TopRight => 1,
        ToastPosition::BottomLeft => 2,
        ToastPosition::BottomRight => 3,
    }
}

fn stop_index(stop: Stop) -> usize {
    match stop {
        Stop::Full => 0,
        Stop::Mid => 1,
        Stop::Empty => 2,
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
        let layout = ConfigureLayout::compute(&font, false, settings.show_developer)?;
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
        let (hue, sat, val) = initial.full.to_hsv();
        let editor = Self {
            window,
            surface,
            font,
            settings,
            draft: initial,
            selected: Stop::Full,
            hue,
            sat,
            val,
            drag: None,
            cursor: None,
            dirty: false,
            picker_open: false,
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

    fn resize_to_content(&self) {
        let Ok(layout) =
            ConfigureLayout::compute(&self.font, self.picker_open, self.settings.show_developer)
        else {
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
            WindowEvent::ScaleFactorChanged { .. } => {
                self.resize_to_content();
                self.window.request_redraw();
            }
            _ => {}
        }
        ConfigureAction::None
    }

    fn on_press(&mut self) -> ConfigureAction {
        let Some((x, y)) = self.cursor else {
            return ConfigureAction::None;
        };
        let Ok(layout) =
            ConfigureLayout::compute(&self.font, self.picker_open, self.settings.show_developer)
        else {
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

        for position in positions() {
            if layout.position_choice(position).contains(x, y) {
                self.settings.toast_position = position;
                return ConfigureAction::SelectToastPosition(position);
            }
        }

        if layout.preview.contains(x, y) {
            self.picker_open = !self.picker_open;
            self.resize_to_content();
            return ConfigureAction::None;
        }

        #[cfg(windows)]
        if layout.autostart_row.contains(x, y) {
            self.settings.autostart = !self.settings.autostart;
            return ConfigureAction::SetAutostart(self.settings.autostart);
        }

        #[cfg(feature = "dev-emulate")]
        if self.settings.show_developer {
            for (index, preset) in Preset::ALL.iter().copied().enumerate() {
                if layout.developer_buttons[index].contains(x, y) {
                    return ConfigureAction::DeveloperPreset(preset);
                }
            }
        }

        if self.picker_open {
            for stop in Stop::ALL {
                if layout.stop(stop).contains(x, y) {
                    self.select_stop(stop);
                    return ConfigureAction::None;
                }
            }
            if layout.sv.contains(x, y) {
                self.drag = Some(DragKind::Sv);
                self.apply_drag();
            } else if layout.hue.contains(x, y) {
                self.drag = Some(DragKind::Hue);
                self.apply_drag();
            } else if layout.reset.contains(x, y) {
                self.draft = BatterySpectrum::DEFAULT;
                self.select_stop(self.selected);
                self.dirty = true;
            } else if layout.apply.contains(x, y) && self.dirty {
                self.dirty = false;
                return ConfigureAction::ApplySpectrum(self.draft);
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
        let Ok(layout) =
            ConfigureLayout::compute(&self.font, self.picker_open, self.settings.show_developer)
        else {
            crate::app_log::warn("configure layout failed during drag");
            return;
        };
        match self.drag {
            Some(DragKind::Sv) => {
                let rect = layout.sv;
                self.sat = ((x - rect.x) / rect.w).clamp(0.0, 1.0) as f32;
                self.val = (1.0 - (y - rect.y) / rect.h).clamp(0.0, 1.0) as f32;
            }
            Some(DragKind::Hue) => {
                let rect = layout.hue;
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
        let layout =
            ConfigureLayout::compute(&self.font, self.picker_open, self.settings.show_developer)?;
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
        Self::paint_titlebar(state, &layout, &mut fb);
        paint_separators(&mut fb, &layout);
        Self::paint_settings(state, &layout, &mut fb);
        Self::paint_editor(state, &layout, &mut fb);
        #[cfg(feature = "dev-emulate")]
        if state.settings.show_developer {
            Self::paint_developer(state, &layout, &mut fb);
        }
        buffer.present().map_err(|e| format!("present: {e}"))
    }

    fn paint_titlebar(state: PaintState, layout: &ConfigureLayout, fb: &mut Framebuffer<'_>) {
        let accent = state.draft.full;
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
            "—",
            cursor.is_some_and(|(x, y)| layout.minimize.contains(x, y)),
            false,
        );
        paint_title_button(
            fb,
            layout.close,
            "×",
            cursor.is_some_and(|(x, y)| layout.close.contains(x, y)),
            true,
        );
    }

    fn paint_settings(state: PaintState, layout: &ConfigureLayout, fb: &mut Framebuffer<'_>) {
        section_heading(fb, layout.notifications_heading, "Notifications");
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
                state.draft.full,
            );
        }

        section_heading(fb, layout.position_heading, "Toast position");
        paint_position_diagram(fb, layout, state);

        section_heading(fb, layout.system_heading, "System");
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
                state.draft.full,
            );
        }
        #[cfg(not(windows))]
        fb.text(
            layout.autostart_row.x + 12.0,
            layout.autostart_row.y + 14.0,
            "Managed by your operating system",
            ui::MUTED,
            FONT_SMALL,
        );
    }

    fn paint_editor(state: PaintState, layout: &ConfigureLayout, fb: &mut Framebuffer<'_>) {
        section_heading(fb, layout.editor_heading, "Lightbar colors");
        let preview = layout.preview;
        let preview_hot = state.cursor.is_some_and(|(x, y)| preview.contains(x, y));
        fb.round_rect(
            (preview.x, preview.y, preview.w, preview.h),
            layout::RADIUS_CARD,
            if preview_hot {
                ui::PANEL_HOVER
            } else {
                ui::PANEL
            },
            Some(if layout.picker_open {
                ui::rgb_of(state.draft.full)
            } else {
                ui::LINE
            }),
        );
        draw_spectrum_bar(fb, layout.spectrum_bar, state.draft);
        if !layout.picker_open {
            return;
        }

        for stop in Stop::ALL {
            let stop_layout = layout.stops[stop_index(stop)];
            let rect = stop_layout.bounds;
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
                (
                    stop_layout.swatch.x,
                    stop_layout.swatch.y,
                    stop_layout.swatch.w,
                    stop_layout.swatch.h,
                ),
                6.0,
                ui::rgb_of(state.color_of(stop)),
                None,
            );
            fb.text_in_rect(
                stop_layout.label,
                stop.label(),
                ui::INK,
                FONT_BODY,
                ui::HorizontalAlign::Left,
                ui::VerticalAlign::Center,
            );
            fb.text_in_rect(
                stop_layout.percent,
                stop.percent(),
                ui::MUTED,
                FONT_SMALL,
                ui::HorizontalAlign::Left,
                ui::VerticalAlign::Center,
            );
            fb.text_in_rect(
                stop_layout.hex,
                &state.color_of(stop).to_hex(),
                ui::MUTED,
                FONT_SMALL,
                ui::HorizontalAlign::Left,
                ui::VerticalAlign::Center,
            );
        }

        let picker = layout.picker;
        card(fb, picker);
        fb.text_in_rect(
            layout.picker_label,
            &format!(
                "Editing {} · {}",
                state.selected.label(),
                state.selected.percent()
            ),
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
            state.draft.full,
        );
        paint_button(
            fb,
            layout.apply,
            "Apply",
            state
                .cursor
                .is_some_and(|(x, y)| layout.apply.contains(x, y)),
            true,
            state.dirty,
            state.draft.full,
        );
    }

    #[cfg(feature = "dev-emulate")]
    fn paint_developer(state: PaintState, layout: &ConfigureLayout, fb: &mut Framebuffer<'_>) {
        section_heading(
            fb,
            layout.developer_heading.unwrap_or_default(),
            "Developer",
        );
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
                state.draft.full,
            );
        }
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

fn card(fb: &mut Framebuffer<'_>, rect: Rect) {
    fb.round_rect(
        (rect.x, rect.y, rect.w, rect.h),
        layout::RADIUS_CARD,
        ui::PANEL,
        Some(ui::LINE),
    );
}

fn section_heading(fb: &mut Framebuffer<'_>, rect: Rect, title: &str) {
    fb.text_in_rect(
        rect,
        title,
        ui::INK,
        FONT_HEADING,
        ui::HorizontalAlign::Left,
        ui::VerticalAlign::Center,
    );
}

fn paint_separators(fb: &mut Framebuffer<'_>, layout: &ConfigureLayout) {
    for separator in &layout.separators {
        fb.fill_rect(separator.x, separator.y, separator.w, separator.h, ui::LINE);
    }
}

fn paint_position_diagram(fb: &mut Framebuffer<'_>, layout: &ConfigureLayout, state: PaintState) {
    for position in positions() {
        let hit = layout.position_choice(position);
        let hot = state.cursor.is_some_and(|(x, y)| hit.contains(x, y));
        let selected = position == state.settings.toast_position;
        fb.round_rect(
            (hit.x, hit.y, hit.w, hit.h),
            5.0,
            if selected {
                ui::rgb_of(state.draft.full)
            } else if hot {
                ui::PANEL_HOVER
            } else {
                ui::PANEL
            },
            Some(if selected { ui::INK } else { ui::LINE }),
        );
    }
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
    fb.text_in_rect(
        rect,
        label,
        ui::INK,
        18.0,
        ui::HorizontalAlign::Center,
        ui::VerticalAlign::Center,
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
        let font = ui::load_system_ui_font().unwrap();
        let layout = ConfigureLayout::compute(&font, false, false).unwrap();
        for position in positions() {
            let rect = layout.position_choice(position);
            assert!((rect.w / rect.h - 16.0 / 9.0).abs() < f64::EPSILON);
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
    fn title_controls_do_not_overlap_drag_region() {
        let font = ui::load_system_ui_font().unwrap();
        let layout = ConfigureLayout::compute(&font, false, false).unwrap();
        assert!(layout.title_drag.right() <= layout.minimize.x);
        assert!(layout.minimize.right() <= layout.close.x);
        assert_eq!(layout.title_drag.h, layout::WINDOW_HEADER_HEIGHT);
    }

    #[test]
    fn sections_follow_compact_order_and_picker_expands() {
        let font = ui::load_system_ui_font().unwrap();
        let collapsed = ConfigureLayout::compute(&font, false, false).unwrap();
        let expanded = ConfigureLayout::compute(&font, true, false).unwrap();
        assert!(collapsed.height < expanded.height);
        assert!(collapsed.autostart_row.bottom() < collapsed.notifications_heading.y);
        assert!(collapsed.notification_rows[3].bottom() < collapsed.position_heading.y);
        assert!(collapsed.position_choices[3].bottom() < collapsed.editor_heading.y);
        assert!(collapsed.preview.w > 240.0);
        let center = (
            collapsed.preview.x + collapsed.preview.w / 2.0,
            collapsed.preview.y + collapsed.preview.h / 2.0,
        );
        assert!(collapsed.preview.contains(center.0, center.1));
    }

    #[test]
    fn computed_layout_fits_its_derived_height() {
        let font = ui::load_system_ui_font().unwrap();
        for picker_open in [false, true] {
            let layout = ConfigureLayout::compute(&font, picker_open, false).unwrap();
            let mut rects = vec![
                layout.notifications_heading,
                layout.position_heading,
                layout.system_heading,
                layout.editor_heading,
                layout.preview,
            ];
            rects.extend(layout.notification_rows);
            rects.extend(layout.position_choices);
            if picker_open {
                rects.extend([layout.picker, layout.reset, layout.apply]);
                rects.extend(layout.stops.map(|stop| stop.bounds));
            }
            assert!(rects.iter().all(|rect| {
                rect.x >= 0.0
                    && rect.y >= layout::WINDOW_HEADER_HEIGHT
                    && rect.right() <= WIN_W
                    && rect.bottom() <= layout.height
            }));
        }
    }

    #[cfg(feature = "dev-emulate")]
    #[test]
    fn developer_controls_are_present_and_bounded() {
        let font = ui::load_system_ui_font().unwrap();
        let layout = ConfigureLayout::compute(&font, false, true).unwrap();
        assert!(layout.developer_heading.is_some());
        assert!(layout.developer_buttons.len() >= Preset::ALL.len());
        assert!(
            layout
                .developer_buttons
                .iter()
                .all(|rect| rect.bottom() <= layout.height)
        );
    }
}
