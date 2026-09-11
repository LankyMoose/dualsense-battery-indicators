//! iced `daemon` shell for the DualSense battery tray app.
//!
//! The daemon boots windowless: it owns the tray icon and only opens windows on
//! demand (controller popup, configure window, overlay toast).

use crate::app_log;
#[cfg(windows)]
use crate::autostart;
use crate::battery::{self, ControllerStatus};
use crate::color::{self, BatterySpectrum, color_for_battery_percent};
use crate::configure_view::{
    self, ConfigureMessage, ConfigureSettings, ConfigureState, NotificationSetting,
};
#[cfg(feature = "dev-emulate")]
use crate::emulate::{self, Preset};
use crate::icon;
use crate::known::KnownControllers;
use crate::lightbar::{
    self, LOW_BATTERY_ORANGE, LOW_BATTERY_PULSE_GAP_MS, LOW_BATTERY_PULSE_ON_MS,
};
use crate::notify::{NotifyEvent, NotifyTracker};
use crate::popup_view::{self, ControllerRow, PopupMessage};
use crate::prefs::{Prefs, ToastPosition, clamp_low_battery_percent};
use crate::theme;
use crate::toast::ToastMessage;
use crate::toast_view;

use iced::futures::Stream;
use iced::futures::channel::{mpsc, oneshot};
use iced::widget::{container, operation, space};
use iced::{Element, Point, Size, Subscription, Task, Theme, stream, window};

use std::collections::VecDeque;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{
    MouseButton as TrayMouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

/// How often to scan for DualSense connect/disconnect.
const PRESENCE_INTERVAL: Duration = Duration::from_secs(3);
/// How often to re-read battery / lightbar when membership is stable and pads are readable.
const BATTERY_INTERVAL: Duration = Duration::from_secs(60);
/// While the tray shows connected pads, re-probe often so a powered-off BT pad
/// (still lingering in the HID list) is dropped quickly.
const LIVENESS_INTERVAL: Duration = Duration::from_secs(5);
/// When HID lists pads but battery reads keep failing, retry sooner than BATTERY_INTERVAL.
const UNREAD_RETRY_INTERVAL: Duration = Duration::from_secs(15);
/// How long an overlay toast stays on screen.
const TOAST_LIFETIME: Duration = Duration::from_secs(5);
/// Slide-in / slide-out duration for overlay toasts.
const TOAST_SLIDE_DURATION: Duration = Duration::from_millis(250);
/// Gap between the tray icon and the popup.
const POPUP_GAP: f32 = 8.0;
/// Minimum distance kept from any screen edge.
const SCREEN_MARGIN: f32 = 8.0;

const QUIT_ID: &str = "quit";
const SETTINGS_ID: &str = "settings";

/// Screen rectangle of the tray icon, in physical pixels.
#[derive(Debug, Clone, Copy, Default)]
pub struct TrayAnchor {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone)]
pub enum Message {
    /// Periodic presence scan.
    Tick,
    /// Result of a background `poll_controllers` call.
    PollResult(Result<Vec<ControllerStatus>, String>),

    /// A tray menu item was activated.
    TrayMenu(String),
    /// The tray icon was left-clicked at the given screen rectangle.
    TrayLeftClick(TrayAnchor),

    WindowClosed(window::Id),
    WindowUnfocused(window::Id),

    PopupOpened(window::Id),
    PlacePopup {
        id: window::Id,
        scale: f32,
        monitor: Option<Size>,
        size: Size,
    },
    Popup(PopupMessage),

    ConfigureOpened(window::Id),
    Configure(ConfigureMessage),

    PlaceToast {
        id: window::Id,
        monitor: Option<Size>,
    },
    /// Advance the active toast slide animation.
    ToastFrame,
    /// Begin dismiss (slide-out) for the toast of the given generation.
    ToastDismiss(u64),

    Exit,
}

pub struct App {
    prefs: Prefs,
    known: KnownControllers,
    notify: NotifyTracker,

    controllers: Vec<ControllerStatus>,
    /// Last HID presence snapshot (serials), used to detect connect/disconnect.
    last_discovered: Vec<String>,
    last_battery_poll: Instant,

    /// True while an identify flash sequence is running (pulse should yield).
    identifying: Arc<AtomicBool>,
    /// True while a background battery poll is in flight.
    refreshing: Arc<AtomicBool>,
    /// Controllers currently in the critical low-battery bucket (serial, percent).
    low_battery: Arc<Mutex<Vec<(String, u8)>>>,

    tray_icon: Option<TrayIcon>,
    tray_anchor: TrayAnchor,

    popup_window: Option<window::Id>,
    popup_state: popup_view::State,
    popup_rows: Vec<ControllerRow>,

    configure_window: Option<window::Id>,
    configure_state: ConfigureState,

    toast_window: Option<window::Id>,
    toast_message: Option<ToastMessage>,
    toast_queue: VecDeque<ToastMessage>,
    toast_generation: u64,
    toast_placement: Option<ToastPlacement>,
    toast_anim_started: Instant,
    toast_dismissing: bool,

    #[cfg(feature = "dev-emulate")]
    dev_mode: bool,
    #[cfg(feature = "dev-emulate")]
    emulating: bool,
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

#[cfg(feature = "dev-emulate")]
pub fn run(dev_mode: bool) -> Result<(), Box<dyn std::error::Error>> {
    run_app(dev_mode)
}

#[cfg(not(feature = "dev-emulate"))]
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    run_app()
}

fn run_app(
    #[cfg(feature = "dev-emulate")] dev_mode: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "dev-emulate")]
    let boot = move || App::boot(dev_mode);
    #[cfg(not(feature = "dev-emulate"))]
    let boot = App::boot;

    iced::daemon(boot, App::update, App::view)
        .subscription(App::subscription)
        .title(App::title)
        .theme(App::theme)
        .antialiasing(true)
        .run()?;

    Ok(())
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

impl App {
    fn boot(#[cfg(feature = "dev-emulate")] dev_mode: bool) -> (Self, Task<Message>) {
        let prefs = Prefs::load();
        let known = KnownControllers::load();
        color::set_active_spectrum(prefs.spectrum.clone());

        #[cfg(windows)]
        autostart::ensure_quiet_entry();

        let identifying = Arc::new(AtomicBool::new(false));
        let low_battery = Arc::new(Mutex::new(Vec::new()));
        start_low_battery_pulse_thread(Arc::clone(&identifying), Arc::clone(&low_battery));

        let configure_state = ConfigureState::new(prefs.spectrum.clone());

        let mut app = Self {
            prefs,
            known,
            notify: NotifyTracker::new(),
            controllers: Vec::new(),
            last_discovered: battery::list_controller_serials().unwrap_or_default(),
            last_battery_poll: Instant::now(),
            identifying,
            refreshing: Arc::new(AtomicBool::new(false)),
            low_battery,
            tray_icon: None,
            tray_anchor: TrayAnchor::default(),
            popup_window: None,
            popup_state: popup_view::State::default(),
            popup_rows: Vec::new(),
            configure_window: None,
            configure_state,
            toast_window: None,
            toast_message: None,
            toast_queue: VecDeque::new(),
            toast_generation: 0,
            toast_placement: None,
            toast_anim_started: Instant::now(),
            toast_dismissing: false,
            #[cfg(feature = "dev-emulate")]
            dev_mode,
            #[cfg(feature = "dev-emulate")]
            emulating: false,
        };

        // Show the tray immediately, then poll in the background so a stuck HID
        // read cannot delay the icon for tens of seconds.
        app.create_tray();
        app.sync_popup_rows();
        app.sync_low_battery();

        let task = app.request_refresh();
        (app, task)
    }

    fn title(&self, window: window::Id) -> String {
        if Some(window) == self.configure_window {
            "Settings".to_string()
        } else {
            crate::app_meta::DISPLAY_NAME.to_string()
        }
    }

    fn theme(&self, window: window::Id) -> Theme {
        if Some(window) == self.toast_window {
            theme::toast_theme()
        } else {
            theme::app_theme()
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            window::close_events().map(Message::WindowClosed),
            iced::event::listen_with(|event, _status, id| match event {
                iced::Event::Window(window::Event::Unfocused) => Some(Message::WindowUnfocused(id)),
                _ => None,
            }),
            iced::time::every(PRESENCE_INTERVAL).map(|_| Message::Tick),
            Subscription::run(tray_events),
        ];

        if self.toast_message.is_some() {
            subscriptions.push(Subscription::run_with(
                self.toast_generation,
                |generation| escape_hotkey(*generation),
            ));
        }

        if self.toast_animating() {
            subscriptions.push(window::frames().map(|_| Message::ToastFrame));
        }

        Subscription::batch(subscriptions)
    }

    fn view(&self, window: window::Id) -> Element<'_, Message> {
        if Some(window) == self.popup_window {
            return popup_view::view(&self.popup_state, &self.popup_rows, &self.prefs.spectrum)
                .map(Message::Popup);
        }

        if Some(window) == self.configure_window {
            return configure_view::view(&self.configure_state, &self.configure_settings())
                .map(Message::Configure);
        }

        if Some(window) == self.toast_window {
            return match self.toast_message.as_ref() {
                Some(message) => {
                    toast_view::view(message, Message::ToastDismiss(self.toast_generation))
                }
                None => toast_view::empty(),
            };
        }

        container(space()).into()
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => self.on_tick(),
            Message::PollResult(result) => self.on_poll_result(result),

            Message::TrayMenu(id) => match id.as_str() {
                SETTINGS_ID => self.open_configure(),
                QUIT_ID => self.update(Message::Exit),
                _ => Task::none(),
            },
            Message::TrayLeftClick(anchor) => {
                self.tray_anchor = anchor;
                self.toggle_popup()
            }

            Message::WindowClosed(id) => {
                if Some(id) == self.popup_window {
                    self.popup_window = None;
                    self.popup_state.cancel();
                } else if Some(id) == self.configure_window {
                    self.configure_window = None;
                } else if Some(id) == self.toast_window {
                    self.toast_window = None;
                }
                Task::none()
            }
            Message::WindowUnfocused(id) => {
                // The popup is a transient tray flyout: dismiss it when focus moves away.
                if Some(id) == self.popup_window && !self.popup_state.is_editing_any() {
                    self.close_popup()
                } else {
                    Task::none()
                }
            }

            Message::PopupOpened(id) => place_popup(id),
            Message::PlacePopup {
                id,
                scale,
                monitor,
                size,
            } => {
                let position = popup_position(self.tray_anchor, scale, monitor, size);
                window::move_to(id, position)
                    .chain(window::set_mode(id, window::Mode::Windowed))
                    .chain(window::gain_focus(id))
            }
            Message::Popup(message) => self.on_popup_message(message),

            Message::ConfigureOpened(id) => window::gain_focus(id),
            Message::Configure(message) => self.on_configure_message(message),

            Message::PlaceToast { id, monitor } => {
                let placement = toast_placement(self.prefs.toast_position, monitor);
                self.toast_placement = Some(placement);
                self.toast_anim_started = Instant::now();
                self.toast_dismissing = false;
                let start = Point::new(placement.x, placement.outside_y);
                window::move_to(id, start).chain(window::set_mode(id, window::Mode::Windowed))
            }
            Message::ToastFrame => self.animate_toast(),
            Message::ToastDismiss(generation) => {
                if generation == self.toast_generation {
                    self.dismiss_toast()
                } else {
                    Task::none()
                }
            }

            Message::Exit => {
                self.known.save();
                self.tray_icon.take();
                iced::exit()
            }
        }
    }

    // -----------------------------------------------------------------------
    // Tray
    // -----------------------------------------------------------------------

    fn create_tray(&mut self) {
        let menu = Menu::new();
        let _ = menu.append(&MenuItem::with_id(SETTINGS_ID, "Settings", true, None));
        let _ = menu.append(&MenuItem::with_id(QUIT_ID, "Exit", true, None));

        match TrayIconBuilder::new()
            .with_tooltip(icon::tooltip_for_controllers(&self.controllers))
            .with_icon(icon::icon_for_controllers(&self.controllers))
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
        {
            Ok(tray) => self.tray_icon = Some(tray),
            Err(err) => app_log::error(format!("failed to create tray icon: {err}")),
        }
    }

    fn apply_tray(&mut self) {
        let icon = icon::icon_for_controllers(&self.controllers);
        let tooltip = icon::tooltip_for_controllers(&self.controllers);
        if let Some(tray) = self.tray_icon.as_mut() {
            let _ = tray.set_icon(Some(icon));
            let _ = tray.set_tooltip(Some(tooltip));
        }
        self.sync_popup_rows();
    }

    // -----------------------------------------------------------------------
    // Controller polling
    // -----------------------------------------------------------------------

    fn on_tick(&mut self) -> Task<Message> {
        #[cfg(feature = "dev-emulate")]
        if self.emulating {
            return Task::none();
        }

        if self.identifying.load(Ordering::SeqCst) || self.refreshing.load(Ordering::SeqCst) {
            return Task::none();
        }

        let discovered = match battery::list_controller_serials() {
            Ok(serials) => serials,
            Err(err) => {
                app_log::warn(format!("presence scan failed: {err}"));
                return Task::none();
            }
        };

        let membership_changed = discovered != self.last_discovered;
        self.last_discovered = discovered;

        // HID list went empty — update the tray immediately. A powered-off DualSense
        // often disappears from enumeration well before the next battery poll.
        if membership_changed && self.last_discovered.is_empty() {
            let task = if self.controllers.is_empty() {
                Task::none()
            } else {
                self.apply_controllers(Vec::new())
            };
            self.last_battery_poll = Instant::now();
            return task;
        }

        let battery_due = self.last_battery_poll.elapsed() >= BATTERY_INTERVAL;
        let liveness_due =
            !self.controllers.is_empty() && self.last_battery_poll.elapsed() >= LIVENESS_INTERVAL;
        // HID can list a pad that we cannot open/read yet (sleeping BT, exclusive access).
        // Retry on a moderate interval — not every presence tick — so the UI stays responsive.
        let unread_retry = !self.last_discovered.is_empty()
            && self.controllers.is_empty()
            && self.last_battery_poll.elapsed() >= UNREAD_RETRY_INTERVAL;

        if membership_changed || battery_due || liveness_due || unread_retry {
            self.request_refresh()
        } else {
            Task::none()
        }
    }

    fn request_refresh(&mut self) -> Task<Message> {
        #[cfg(feature = "dev-emulate")]
        if self.emulating {
            return Task::none();
        }

        if self.identifying.load(Ordering::SeqCst) {
            return Task::none();
        }
        if self
            .refreshing
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Task::none();
        }

        Task::perform(spawn_blocking(battery::poll_controllers), |outcome| {
            Message::PollResult(outcome.unwrap_or_else(Err))
        })
    }

    fn on_poll_result(&mut self, result: Result<Vec<ControllerStatus>, String>) -> Task<Message> {
        self.refreshing.store(false, Ordering::SeqCst);
        self.last_battery_poll = Instant::now();

        #[cfg(feature = "dev-emulate")]
        if self.emulating {
            return Task::none();
        }

        match result {
            Ok(controllers) => self.apply_controllers(controllers),
            Err(err) => {
                app_log::warn(format!("refresh failed: {err}"));
                Task::none()
            }
        }
    }

    fn apply_controllers(&mut self, controllers: Vec<ControllerStatus>) -> Task<Message> {
        let known_changed = self.known.sync_from_live(&controllers);
        let controllers_changed = !controllers_equivalent(&self.controllers, &controllers);
        if !controllers_changed && !known_changed {
            return Task::none();
        }

        let mut events = Vec::new();
        if controllers_changed {
            let previous = std::mem::replace(&mut self.controllers, controllers);
            events = self
                .notify
                .evaluate(&previous, &self.controllers, &self.prefs, |serial| {
                    self.known.nickname(serial).map(str::to_string)
                });
            self.sync_low_battery();
        }

        self.known.save();
        self.apply_tray();
        self.queue_notifications(events)
    }

    fn sync_low_battery(&self) {
        let threshold = self.prefs.low_battery_percent;
        let list: Vec<(String, u8)> = self
            .controllers
            .iter()
            .filter(|c| c.is_low_battery(threshold) && !is_emulated_serial(&c.serial))
            .map(|c| (c.serial.clone(), c.percent))
            .collect();
        if let Ok(mut guard) = self.low_battery.lock() {
            *guard = list;
        }
    }

    // -----------------------------------------------------------------------
    // Popup window
    // -----------------------------------------------------------------------

    fn sync_popup_rows(&mut self) {
        let threshold = self.prefs.low_battery_percent;
        let mut rows = Vec::new();
        for controller in &self.controllers {
            rows.push(ControllerRow::connected(
                controller,
                self.known.is_remembered(&controller.serial),
                KnownControllers::is_storable_serial(&controller.serial)
                    && !is_emulated_serial(&controller.serial),
                self.known.nickname(&controller.serial).map(str::to_string),
                threshold,
            ));
        }
        for controller in self.known.remembered_disconnected(&self.controllers) {
            let nickname = self.known.nickname(&controller.serial).map(str::to_string);
            rows.push(ControllerRow::disconnected(controller, nickname));
        }
        self.popup_rows = rows;
    }

    fn toggle_popup(&mut self) -> Task<Message> {
        if self.popup_window.is_some() {
            self.close_popup()
        } else {
            self.open_popup()
        }
    }

    fn open_popup(&mut self) -> Task<Message> {
        if let Some(id) = self.popup_window {
            return window::gain_focus(id);
        }

        self.popup_state.cancel();
        let height = popup_view::window_height(self.popup_rows.len());
        let (id, open) = window::open(window::Settings {
            size: Size::new(popup_view::WIDTH, height),
            position: window::Position::Default,
            visible: false,
            resizable: false,
            decorations: false,
            level: window::Level::AlwaysOnTop,
            exit_on_close_request: true,
            platform_specific: overlay_platform_specific(),
            ..window::Settings::default()
        });

        self.popup_window = Some(id);
        open.map(Message::PopupOpened)
    }

    fn close_popup(&mut self) -> Task<Message> {
        self.popup_state.cancel();
        match self.popup_window.take() {
            Some(id) => window::close(id),
            None => Task::none(),
        }
    }

    fn on_popup_message(&mut self, message: PopupMessage) -> Task<Message> {
        match message {
            PopupMessage::OpenSettings => {
                let close = self.close_popup();
                close.chain(self.open_configure())
            }
            PopupMessage::Identify(serial) => {
                self.identify(&serial);
                Task::none()
            }
            PopupMessage::PowerOff(serial) => {
                self.power_off(&serial);
                Task::none()
            }
            PopupMessage::ToggleRemember(serial) => {
                self.toggle_remember(&serial);
                Task::none()
            }
            PopupMessage::BeginEdit(serial) => {
                let current = self.known.nickname(&serial).map(str::to_string);
                self.popup_state.begin_edit(&serial, current.as_deref());
                Task::batch([
                    operation::focus(popup_view::nickname_input_id()),
                    operation::select_all(popup_view::nickname_input_id()),
                ])
            }
            PopupMessage::DraftChanged(value) => {
                self.popup_state.draft = value;
                Task::none()
            }
            PopupMessage::CommitNickname => {
                if let Some((serial, nickname)) = self.popup_state.commit() {
                    self.set_nickname(&serial, nickname);
                }
                Task::none()
            }
            PopupMessage::CancelEdit => {
                self.popup_state.cancel();
                Task::none()
            }
        }
    }

    // -----------------------------------------------------------------------
    // Configure window
    // -----------------------------------------------------------------------

    fn configure_settings(&self) -> ConfigureSettings {
        ConfigureSettings {
            notify_low: self.prefs.notify_low,
            notify_charged: self.prefs.notify_charged,
            notify_connect: self.prefs.notify_connect,
            notify_disconnect: self.prefs.notify_disconnect,
            low_battery_percent: self.prefs.low_battery_percent,
            toast_position: self.prefs.toast_position,
            #[cfg(windows)]
            autostart: autostart::is_enabled(),
            show_developer: {
                #[cfg(feature = "dev-emulate")]
                {
                    self.dev_mode
                }
                #[cfg(not(feature = "dev-emulate"))]
                {
                    false
                }
            },
        }
    }

    fn open_configure(&mut self) -> Task<Message> {
        if let Some(id) = self.configure_window {
            return window::gain_focus(id);
        }

        self.configure_state
            .set_spectrum(self.prefs.spectrum.clone());

        let (id, open) = window::open(window::Settings {
            size: Size::new(configure_view::WIDTH, configure_view::HEIGHT),
            position: window::Position::Centered,
            resizable: false,
            decorations: false,
            exit_on_close_request: true,
            platform_specific: window_platform_specific(),
            ..window::Settings::default()
        });

        self.configure_window = Some(id);
        open.map(Message::ConfigureOpened)
    }

    fn on_configure_message(&mut self, message: ConfigureMessage) -> Task<Message> {
        match message {
            ConfigureMessage::Close => match self.configure_window.take() {
                Some(id) => window::close(id),
                None => Task::none(),
            },
            ConfigureMessage::DragWindow => match self.configure_window {
                Some(id) => window::drag(id),
                None => Task::none(),
            },
            ConfigureMessage::ToggleSection(section) => {
                self.configure_state.toggle_section(section);
                Task::none()
            }
            ConfigureMessage::SetNotification(setting, enabled) => {
                match setting {
                    NotificationSetting::Connect => self.prefs.notify_connect = enabled,
                    NotificationSetting::Disconnect => self.prefs.notify_disconnect = enabled,
                    NotificationSetting::Low => self.prefs.notify_low = enabled,
                    NotificationSetting::Charged => self.prefs.notify_charged = enabled,
                }
                self.prefs.save();
                Task::none()
            }
            ConfigureMessage::SetLowBatteryPercent(percent) => {
                let percent = clamp_low_battery_percent(percent);
                if self.prefs.low_battery_percent != percent {
                    self.prefs.low_battery_percent = percent;
                    self.prefs.save();
                    self.sync_low_battery();
                    self.sync_popup_rows();
                }
                Task::none()
            }
            ConfigureMessage::SetToastPosition(position) => {
                self.prefs.toast_position = position;
                self.prefs.save();
                self.show_position_preview()
            }
            #[cfg(windows)]
            ConfigureMessage::SetAutostart(enabled) => {
                if let Err(err) = autostart::set_enabled(enabled) {
                    app_log::error(format!("autostart toggle failed: {err}"));
                }
                Task::none()
            }
            ConfigureMessage::SelectStop(index) => {
                self.configure_state.select(index);
                Task::none()
            }
            ConfigureMessage::MoveStop(percent) => {
                let next = self.configure_state.move_stop(percent);
                self.apply_spectrum_maybe(next)
            }
            ConfigureMessage::AddStopAt(percent) => {
                let next = self.configure_state.add_stop_at(percent);
                self.apply_spectrum_maybe(next)
            }
            ConfigureMessage::RemoveStop => {
                let next = self.configure_state.remove_selected();
                self.apply_spectrum_maybe(next)
            }
            ConfigureMessage::HueChanged(hue) => {
                let next = self.configure_state.set_hue(hue);
                self.apply_spectrum_maybe(next)
            }
            ConfigureMessage::SaturationValueChanged(saturation, value) => {
                let next = self.configure_state.set_saturation_value(saturation, value);
                self.apply_spectrum_maybe(next)
            }
            ConfigureMessage::ResetSpectrum => {
                let next = self.configure_state.reset();
                self.apply_spectrum(next);
                Task::none()
            }
            #[cfg(feature = "dev-emulate")]
            ConfigureMessage::DeveloperPreset(preset) => self.apply_dev_preset(preset),
        }
    }

    fn apply_spectrum_maybe(&mut self, spectrum: Option<BatterySpectrum>) -> Task<Message> {
        if let Some(spectrum) = spectrum {
            self.apply_spectrum(spectrum);
        }
        Task::none()
    }

    fn apply_spectrum(&mut self, spectrum: BatterySpectrum) {
        self.prefs.spectrum = spectrum.clone();
        self.prefs.save();
        color::set_active_spectrum(spectrum.clone());

        for controller in &self.controllers {
            if is_emulated_serial(&controller.serial) {
                continue;
            }
            let color = spectrum.color_at_percent(controller.percent);
            if let Err(err) = lightbar::apply_lightbar_rgb(&controller.serial, color) {
                app_log::warn(format!(
                    "failed to apply spectrum color for {}: {err}",
                    controller.serial
                ));
            }
        }
    }

    // -----------------------------------------------------------------------
    // Controller actions
    // -----------------------------------------------------------------------

    fn identify(&self, serial: &str) {
        if is_emulated_serial(serial) {
            app_log::info(format!("identify skipped for emulated controller {serial}"));
            return;
        }

        let Some(controller) = self.controllers.iter().find(|c| c.serial == serial) else {
            return;
        };

        if self
            .identifying
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let serial = serial.to_string();
        let percent = controller.percent;
        let identifying = Arc::clone(&self.identifying);

        thread::spawn(move || {
            if let Err(err) = lightbar::identify_controller(&serial, percent) {
                app_log::warn(format!("identify failed for {serial}: {err}"));
            }
            identifying.store(false, Ordering::SeqCst);
        });
    }

    fn power_off(&self, serial: &str) {
        if is_emulated_serial(serial) {
            app_log::info(format!(
                "power-off skipped for emulated controller {serial}"
            ));
            return;
        }

        let Some(controller) = self.controllers.iter().find(|c| c.serial == serial) else {
            return;
        };
        if controller.connection != "Bluetooth" {
            app_log::warn(format!(
                "power-off ignored for {serial} ({})",
                controller.connection
            ));
            return;
        }

        let serial = serial.to_string();
        thread::spawn(move || {
            if let Err(err) = battery::power_off_bluetooth(&serial) {
                app_log::warn(format!("power-off failed for {serial}: {err}"));
            } else {
                app_log::info(format!("power-off sent for {serial}"));
            }
        });
    }

    fn toggle_remember(&mut self, serial: &str) {
        if is_emulated_serial(serial) {
            return;
        }

        if self.known.is_remembered(serial) {
            self.known.forget(serial);
        } else if let Some(controller) = self.controllers.iter().find(|c| c.serial == serial) {
            self.known.remember(controller);
        }

        self.known.save();
        self.sync_popup_rows();
    }

    fn set_nickname(&mut self, serial: &str, nickname: Option<String>) {
        if is_emulated_serial(serial) {
            return;
        }
        if self.known.set_nickname(serial, nickname) {
            self.known.save();
            self.sync_popup_rows();
        }
    }

    #[cfg(feature = "dev-emulate")]
    fn apply_dev_preset(&mut self, preset: Preset) -> Task<Message> {
        if preset == Preset::Clear {
            self.emulating = false;
            let task = self.apply_controllers(Vec::new());
            self.last_battery_poll = Instant::now();
            return task.chain(self.request_refresh());
        }

        let next = emulate::apply_preset(preset, &self.controllers);
        self.emulating = true;
        self.apply_controllers(next)
    }

    // -----------------------------------------------------------------------
    // Toasts
    // -----------------------------------------------------------------------

    fn queue_notifications(&mut self, events: Vec<NotifyEvent>) -> Task<Message> {
        for event in events {
            self.toast_queue.push_back(ToastMessage::from_notification(
                event,
                self.prefs.spectrum.clone(),
            ));
        }
        self.show_next_toast()
    }

    fn show_position_preview(&mut self) -> Task<Message> {
        self.toast_queue.clear();
        self.toast_queue
            .push_back(ToastMessage::preview(self.prefs.spectrum.accent()));
        let finish = self.finish_toast();
        finish.chain(self.show_next_toast())
    }

    fn show_next_toast(&mut self) -> Task<Message> {
        if self.toast_message.is_some() {
            return Task::none();
        }
        let Some(message) = self.toast_queue.pop_front() else {
            return Task::none();
        };

        self.toast_message = Some(message);
        self.toast_generation = self.toast_generation.wrapping_add(1);
        let generation = self.toast_generation;

        let expire = Task::perform(delay(TOAST_LIFETIME), move |()| {
            Message::ToastDismiss(generation)
        });

        if let Some(id) = self.toast_window {
            return place_toast(id).chain(expire);
        }

        let (id, open) = window::open(window::Settings {
            size: Size::new(toast_view::WIDTH, toast_view::HEIGHT),
            position: window::Position::Default,
            visible: false,
            resizable: false,
            decorations: false,
            transparent: true,
            level: window::Level::AlwaysOnTop,
            exit_on_close_request: false,
            platform_specific: overlay_platform_specific(),
            ..window::Settings::default()
        });

        self.toast_window = Some(id);
        open.then(place_toast).chain(expire)
    }

    fn dismiss_toast(&mut self) -> Task<Message> {
        if self.toast_message.is_none() || self.toast_dismissing {
            return Task::none();
        }
        self.toast_dismissing = true;
        self.toast_anim_started = Instant::now();
        self.animate_toast()
    }

    fn animate_toast(&mut self) -> Task<Message> {
        let Some(id) = self.toast_window else {
            return Task::none();
        };
        let Some(placement) = self.toast_placement else {
            return Task::none();
        };

        let progress = (self.toast_anim_started.elapsed().as_secs_f32()
            / TOAST_SLIDE_DURATION.as_secs_f32())
        .min(1.0);
        let y = slide_y(placement, progress, self.toast_dismissing);
        let move_task = window::move_to(id, Point::new(placement.x, y));

        if self.toast_dismissing && progress >= 1.0 {
            move_task.chain(self.finish_toast())
        } else {
            move_task
        }
    }

    fn finish_toast(&mut self) -> Task<Message> {
        if self.toast_message.take().is_none() {
            return Task::none();
        }
        // Invalidate any in-flight expiry for this toast.
        self.toast_generation = self.toast_generation.wrapping_add(1);
        self.toast_placement = None;
        self.toast_dismissing = false;

        // Keep the window alive but hidden: recreating a GPU surface for every
        // toast would cost more than the memory it saves.
        let hide = match self.toast_window {
            Some(id) => window::set_mode(id, window::Mode::Hidden),
            None => Task::none(),
        };
        hide.chain(self.show_next_toast())
    }

    fn toast_animating(&self) -> bool {
        self.toast_message.is_some()
            && self.toast_placement.is_some()
            && (self.toast_dismissing || self.toast_anim_started.elapsed() < TOAST_SLIDE_DURATION)
    }
}

// ---------------------------------------------------------------------------
// Window placement
// ---------------------------------------------------------------------------

fn place_popup(id: window::Id) -> Task<Message> {
    window::scale_factor(id).then(move |scale| {
        window::monitor_size(id).then(move |monitor| {
            window::size(id).map(move |size| Message::PlacePopup {
                id,
                scale,
                monitor,
                size,
            })
        })
    })
}

fn place_toast(id: window::Id) -> Task<Message> {
    window::monitor_size(id).map(move |monitor| Message::PlaceToast { id, monitor })
}

fn popup_position(anchor: TrayAnchor, scale: f32, monitor: Option<Size>, size: Size) -> Point {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let anchor_x = anchor.x / scale;
    let anchor_y = anchor.y / scale;
    let anchor_w = anchor.width / scale;
    let anchor_h = anchor.height / scale;

    let mut x = anchor_x + anchor_w / 2.0 - size.width / 2.0;
    let mut y = anchor_y - size.height - POPUP_GAP;

    if let Some(monitor) = monitor {
        // A tray at the top of the screen leaves no room above it.
        if y < SCREEN_MARGIN {
            y = anchor_y + anchor_h + POPUP_GAP;
        }
        x = clamp_axis(x, size.width, monitor.width);
        y = clamp_axis(y, size.height, monitor.height);
    }

    Point::new(x.max(0.0), y.max(0.0))
}

fn toast_placement(position: ToastPosition, monitor: Option<Size>) -> ToastPlacement {
    toast_placement_in(position, resolve_toast_area(monitor))
}

#[derive(Debug, Clone, Copy)]
struct ToastArea {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Debug, Clone, Copy)]
struct ToastPlacement {
    x: f32,
    target_y: f32,
    outside_y: f32,
}

fn resolve_toast_area(monitor: Option<Size>) -> ToastArea {
    #[cfg(windows)]
    if let Some(area) = primary_toast_area() {
        return area;
    }

    match monitor {
        Some(size) => ToastArea {
            x: 0.0,
            y: 0.0,
            width: size.width,
            height: size.height,
        },
        None => ToastArea {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        },
    }
}

fn toast_placement_in(position: ToastPosition, area: ToastArea) -> ToastPlacement {
    let target = toast_position_in(position, area);
    let outside_y = match position {
        ToastPosition::TopLeft | ToastPosition::TopCenter | ToastPosition::TopRight => {
            area.y - toast_view::HEIGHT
        }
        ToastPosition::BottomLeft | ToastPosition::BottomCenter | ToastPosition::BottomRight => {
            area.y + area.height
        }
    };
    ToastPlacement {
        x: target.x,
        target_y: target.y,
        outside_y,
    }
}

fn toast_position_in(position: ToastPosition, area: ToastArea) -> Point {
    let width = toast_view::WIDTH;
    let height = toast_view::HEIGHT;
    let margin = toast_view::MARGIN;

    let x = match position {
        ToastPosition::TopLeft | ToastPosition::BottomLeft => area.x + margin,
        ToastPosition::TopCenter | ToastPosition::BottomCenter => {
            area.x + (area.width - width) / 2.0
        }
        ToastPosition::TopRight | ToastPosition::BottomRight => {
            area.x + area.width - width - margin
        }
    };

    let y = match position {
        ToastPosition::TopLeft | ToastPosition::TopCenter | ToastPosition::TopRight => {
            area.y + margin
        }
        ToastPosition::BottomLeft | ToastPosition::BottomCenter | ToastPosition::BottomRight => {
            area.y + area.height - height - margin
        }
    };

    Point::new(x.max(0.0), y.max(0.0))
}

fn slide_y(placement: ToastPlacement, progress: f32, dismissing: bool) -> f32 {
    let eased = ease_out_cubic(progress.clamp(0.0, 1.0));
    let t = if dismissing { 1.0 - eased } else { eased };
    placement.outside_y + (placement.target_y - placement.outside_y) * t
}

fn ease_out_cubic(progress: f32) -> f32 {
    1.0 - (1.0 - progress).powi(3)
}

/// Primary-monitor toast target in logical pixels.
///
/// Uses the Windows work area so a visible taskbar is cleared, while an
/// auto-hide taskbar (which does not reserve work-area space) lets bottom
/// toasts sit near the screen edge. Fullscreen apps get the full monitor.
#[cfg(windows)]
fn primary_toast_area() -> Option<ToastArea> {
    unsafe {
        let monitor =
            win32::MonitorFromPoint(win32::Point { x: 0, y: 0 }, win32::MONITOR_DEFAULTTOPRIMARY);
        if monitor == 0 {
            return None;
        }

        let mut info = win32::MonitorInfo {
            size: std::mem::size_of::<win32::MonitorInfo>() as u32,
            monitor: win32::Rect::default(),
            work: win32::Rect::default(),
            flags: 0,
        };
        if win32::GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }

        let foreground = win32::GetForegroundWindow();
        let mut foreground_rect = win32::Rect::default();
        let got_foreground =
            foreground != 0 && win32::GetWindowRect(foreground, &mut foreground_rect) != 0;
        let fullscreen = got_foreground
            && foreground_rect.left <= info.monitor.left + 2
            && foreground_rect.top <= info.monitor.top + 2
            && foreground_rect.right >= info.monitor.right - 2
            && foreground_rect.bottom >= info.monitor.bottom - 2;

        let area = if fullscreen { info.monitor } else { info.work };

        let mut dpi_x = 0u32;
        let mut dpi_y = 0u32;
        let dpi_ok =
            win32::GetDpiForMonitor(monitor, win32::MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) == 0
                && dpi_x > 0;
        let scale = if dpi_ok { dpi_x as f32 / 96.0 } else { 1.0 };

        Some(ToastArea {
            x: area.left as f32 / scale,
            y: area.top as f32 / scale,
            width: (area.right - area.left).max(1) as f32 / scale,
            height: (area.bottom - area.top).max(1) as f32 / scale,
        })
    }
}

fn clamp_axis(value: f32, extent: f32, available: f32) -> f32 {
    let max = (available - extent - SCREEN_MARGIN).max(SCREEN_MARGIN);
    value.clamp(SCREEN_MARGIN, max)
}

#[cfg(target_os = "windows")]
fn overlay_platform_specific() -> window::settings::PlatformSpecific {
    window::settings::PlatformSpecific {
        drag_and_drop: false,
        skip_taskbar: true,
        undecorated_shadow: false,
        corner_preference: window::settings::platform::CornerPreference::Round,
    }
}

#[cfg(not(target_os = "windows"))]
fn overlay_platform_specific() -> window::settings::PlatformSpecific {
    window::settings::PlatformSpecific::default()
}

#[cfg(target_os = "windows")]
fn window_platform_specific() -> window::settings::PlatformSpecific {
    window::settings::PlatformSpecific {
        drag_and_drop: false,
        skip_taskbar: false,
        undecorated_shadow: true,
        corner_preference: window::settings::platform::CornerPreference::Round,
    }
}

#[cfg(not(target_os = "windows"))]
fn window_platform_specific() -> window::settings::PlatformSpecific {
    window::settings::PlatformSpecific::default()
}

// ---------------------------------------------------------------------------
// Subscriptions
// ---------------------------------------------------------------------------

/// Bridges the global `tray-icon` / `muda` event handlers into the iced runtime.
///
/// Both handlers are backed by a `OnceLock`, so this stream must be created
/// exactly once for the lifetime of the process.
fn tray_events() -> impl Stream<Item = Message> {
    stream::channel(64, async move |output: mpsc::Sender<Message>| {
        let menu_output = output.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            // Every clone owns a guaranteed slot, so `try_send` cannot be starved.
            let _ = menu_output.clone().try_send(Message::TrayMenu(event.id.0));
        }));

        let icon_output = output.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                rect,
                button: TrayMouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let anchor = TrayAnchor {
                    x: rect.position.x as f32,
                    y: rect.position.y as f32,
                    width: rect.size.width as f32,
                    height: rect.size.height as f32,
                };
                let _ = icon_output.clone().try_send(Message::TrayLeftClick(anchor));
            }
        }));

        std::future::pending::<()>().await;
    })
}

/// Global Escape hotkey while an overlay toast is visible.
///
/// The toast window never takes focus, so a regular keyboard subscription would
/// not see the key press.
fn escape_hotkey(generation: u64) -> impl Stream<Item = Message> {
    stream::channel(1, async move |mut output: mpsc::Sender<Message>| {
        if let Some(()) = wait_for_escape().await {
            let _ = output.try_send(Message::ToastDismiss(generation));
        }
    })
}

#[cfg(windows)]
async fn wait_for_escape() -> Option<()> {
    let (id_sender, id_receiver) = std::sync::mpsc::channel();
    let (sender, receiver) = oneshot::channel();

    thread::spawn(move || unsafe {
        // `RegisterHotKey` binds to the calling thread's message queue, so the
        // whole lifecycle has to live on this worker.
        let _ = id_sender.send(win32::GetCurrentThreadId());
        if win32::RegisterHotKey(0, win32::HOTKEY_ID, win32::MOD_NOREPEAT, win32::VK_ESCAPE) == 0 {
            return;
        }

        let mut message = win32::Message::default();
        while win32::GetMessageW(&mut message, 0, 0, 0) > 0 {
            if message.message == win32::WM_HOTKEY && message.w_param == win32::HOTKEY_ID as usize {
                let _ = sender.send(());
                break;
            }
        }

        win32::UnregisterHotKey(0, win32::HOTKEY_ID);
    });

    // Posting `WM_QUIT` unblocks `GetMessageW` when the toast is dismissed by
    // some other means and this future is dropped.
    let _guard = win32::ThreadQuitGuard(id_receiver.recv().unwrap_or(0));
    receiver.await.ok()
}

#[cfg(not(windows))]
async fn wait_for_escape() -> Option<()> {
    std::future::pending::<()>().await
}

#[cfg(windows)]
mod win32 {
    pub const MOD_NOREPEAT: u32 = 0x4000;
    pub const VK_ESCAPE: u32 = 0x1B;
    pub const HOTKEY_ID: i32 = 0x4454;
    pub const WM_HOTKEY: u32 = 0x0312;
    pub const WM_QUIT: u32 = 0x0012;
    pub const MONITOR_DEFAULTTOPRIMARY: u32 = 1;
    pub const MDT_EFFECTIVE_DPI: u32 = 0;

    #[derive(Clone, Copy, Default)]
    #[repr(C)]
    pub struct Point {
        pub x: i32,
        pub y: i32,
    }

    #[derive(Clone, Copy, Default)]
    #[repr(C)]
    pub struct Rect {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    #[repr(C)]
    pub struct MonitorInfo {
        pub size: u32,
        pub monitor: Rect,
        pub work: Rect,
        pub flags: u32,
    }

    #[derive(Clone, Copy, Default)]
    #[repr(C)]
    pub struct Message {
        pub hwnd: isize,
        pub message: u32,
        pub w_param: usize,
        pub l_param: isize,
        pub time: u32,
        pub pt: Point,
    }

    /// Posts `WM_QUIT` to the hotkey worker when the awaiting future is dropped.
    pub struct ThreadQuitGuard(pub u32);

    impl Drop for ThreadQuitGuard {
        fn drop(&mut self) {
            if self.0 != 0 {
                unsafe {
                    PostThreadMessageW(self.0, WM_QUIT, 0, 0);
                }
            }
        }
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        pub fn RegisterHotKey(hwnd: isize, id: i32, modifiers: u32, key: u32) -> i32;
        pub fn UnregisterHotKey(hwnd: isize, id: i32) -> i32;
        pub fn GetMessageW(message: *mut Message, hwnd: isize, min: u32, max: u32) -> i32;
        pub fn PostThreadMessageW(thread: u32, message: u32, w_param: usize, l_param: isize)
        -> i32;
        pub fn MonitorFromPoint(point: Point, flags: u32) -> isize;
        pub fn GetMonitorInfoW(monitor: isize, info: *mut MonitorInfo) -> i32;
        pub fn GetForegroundWindow() -> isize;
        pub fn GetWindowRect(hwnd: isize, rect: *mut Rect) -> i32;
    }

    #[link(name = "shcore")]
    unsafe extern "system" {
        pub fn GetDpiForMonitor(
            monitor: isize,
            dpi_type: u32,
            dpi_x: *mut u32,
            dpi_y: *mut u32,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GetCurrentThreadId() -> u32;
    }
}

// ---------------------------------------------------------------------------
// Background helpers
// ---------------------------------------------------------------------------

/// Run a blocking closure on a worker thread and await its result.
fn spawn_blocking<T, F>(f: F) -> impl Future<Output = Result<T, String>> + Send
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (sender, receiver) = oneshot::channel();
    thread::spawn(move || {
        let _ = sender.send(f());
    });
    async move {
        receiver
            .await
            .map_err(|_| "background task was cancelled".to_string())
    }
}

/// Timer future backed by a worker thread, so no async runtime is required.
fn delay(duration: Duration) -> impl Future<Output = ()> + Send {
    let (sender, receiver) = oneshot::channel::<()>();
    thread::spawn(move || {
        thread::sleep(duration);
        let _ = sender.send(());
    });
    async move {
        let _ = receiver.await;
    }
}

fn start_low_battery_pulse_thread(
    identifying: Arc<AtomicBool>,
    low_battery: Arc<Mutex<Vec<(String, u8)>>>,
) {
    thread::spawn(move || {
        let on = Duration::from_millis(LOW_BATTERY_PULSE_ON_MS);
        let gap = Duration::from_millis(LOW_BATTERY_PULSE_GAP_MS);

        loop {
            thread::sleep(gap);

            if identifying.load(Ordering::SeqCst) {
                continue;
            }

            let targets = low_battery
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default();

            for (serial, percent) in targets {
                if identifying.load(Ordering::SeqCst) {
                    break;
                }

                if is_emulated_serial(&serial) {
                    continue;
                }

                if let Err(err) = lightbar::apply_lightbar_rgb(&serial, LOW_BATTERY_ORANGE) {
                    app_log::warn(format!("low-battery pulse failed for {serial}: {err}"));
                }
                thread::sleep(on);

                if identifying.load(Ordering::SeqCst) {
                    break;
                }

                let color = color_for_battery_percent(percent);
                if let Err(err) = lightbar::apply_lightbar_rgb(&serial, color) {
                    app_log::warn(format!("low-battery restore failed for {serial}: {err}"));
                }
            }
        }
    });
}

fn is_emulated_serial(serial: &str) -> bool {
    #[cfg(feature = "dev-emulate")]
    {
        emulate::is_emulated(serial)
    }
    #[cfg(not(feature = "dev-emulate"))]
    {
        let _ = serial;
        false
    }
}

fn controllers_equivalent(a: &[ControllerStatus], b: &[ControllerStatus]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).all(|(x, y)| {
        x.serial == y.serial
            && x.percent == y.percent
            && x.state == y.state
            && x.connection == y.connection
            && x.product == y.product
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_is_anchored_above_a_bottom_right_tray() {
        let anchor = TrayAnchor {
            x: 1800.0,
            y: 1040.0,
            width: 24.0,
            height: 24.0,
        };
        let position = popup_position(
            anchor,
            1.0,
            Some(Size::new(1920.0, 1080.0)),
            Size::new(360.0, 200.0),
        );
        assert!(position.y < 1040.0);
        assert!(position.x + 360.0 <= 1920.0);
    }

    #[test]
    fn popup_flips_below_a_top_anchored_tray() {
        let anchor = TrayAnchor {
            x: 100.0,
            y: 0.0,
            width: 24.0,
            height: 24.0,
        };
        let position = popup_position(
            anchor,
            1.0,
            Some(Size::new(1920.0, 1080.0)),
            Size::new(360.0, 200.0),
        );
        assert!(position.y >= 24.0);
    }

    #[test]
    fn toast_positions_respect_the_requested_corner() {
        let area = ToastArea {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        let top_left = toast_position_in(ToastPosition::TopLeft, area);
        let bottom_right = toast_position_in(ToastPosition::BottomRight, area);
        assert!(top_left.x < bottom_right.x);
        assert!(top_left.y < bottom_right.y);
    }

    #[test]
    fn bottom_toasts_use_a_single_edge_margin() {
        let area = ToastArea {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        let bottom = toast_position_in(ToastPosition::BottomCenter, area);
        let expected_y = area.height - toast_view::HEIGHT - toast_view::MARGIN;
        assert!((bottom.y - expected_y).abs() < f32::EPSILON);
    }

    #[test]
    fn toast_slide_eases_from_outside_to_target() {
        let placement = ToastPlacement {
            x: 10.0,
            target_y: 100.0,
            outside_y: 200.0,
        };
        assert!((slide_y(placement, 0.0, false) - 200.0).abs() < f32::EPSILON);
        assert!((slide_y(placement, 1.0, false) - 100.0).abs() < f32::EPSILON);
        assert!((slide_y(placement, 0.0, true) - 100.0).abs() < f32::EPSILON);
        assert!((slide_y(placement, 1.0, true) - 200.0).abs() < f32::EPSILON);
        let mid = slide_y(placement, 0.5, false);
        assert!(mid < 150.0, "ease-out should be past the midpoint by t=0.5");
    }
}
