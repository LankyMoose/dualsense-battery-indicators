use crate::app_log;
#[cfg(windows)]
use crate::autostart;
use crate::battery::{self, ControllerStatus};
use crate::color::{self, BatterySpectrum, color_for_battery_percent};
use crate::configure_ui::{
    ConfigureAction, ConfigureSettings, ConfigureWindow, NotificationSetting,
};
use crate::controller_popup::{ControllerPopup, ControllerRow, PopupAction, TrayAnchor};
#[cfg(feature = "dev-emulate")]
use crate::emulate::{self, Preset};
use crate::icon;
use crate::known::KnownControllers;
use crate::lightbar::{
    self, LOW_BATTERY_ORANGE, LOW_BATTERY_PULSE_GAP_MS, LOW_BATTERY_PULSE_ON_MS,
};
use crate::notify::{NotifyEvent, NotifyTracker};
use crate::prefs::{Prefs, ToastPosition};
use crate::toast::{EscapeHotkey, SLIDE_DURATION, ToastMessage, ToastWindow};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{
    MouseButton as TrayMouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};
use winit::application::ApplicationHandler;
use winit::event::StartCause;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::WindowId;

/// How often to scan for DualSense connect/disconnect.
const PRESENCE_INTERVAL: Duration = Duration::from_secs(3);
/// How often to re-read battery / lightbar when membership is stable and pads are readable.
const BATTERY_INTERVAL: Duration = Duration::from_secs(60);
/// While the tray shows connected pads, re-probe often so a powered-off BT pad
/// (still lingering in the HID list) is dropped quickly.
const LIVENESS_INTERVAL: Duration = Duration::from_secs(5);
/// When HID lists pads but battery reads keep failing, retry sooner than BATTERY_INTERVAL.
const UNREAD_RETRY_INTERVAL: Duration = Duration::from_secs(15);
const QUIT_ID: &str = "quit";
const SETTINGS_ID: &str = "settings";

#[derive(Debug)]
pub(crate) enum UserEvent {
    MenuEvent(MenuEvent),
    TrayIconEvent(TrayIconEvent),
    /// Periodic tick: check presence; full poll when membership changes or battery is due.
    Tick,
    /// Result of a background `poll_controllers` call.
    PollResult(Result<Vec<ControllerStatus>, String>),
    /// Escape was pressed while an overlay toast was visible.
    ToastDismiss(u64),
    /// Advance the active toast's slide animation.
    ToastFrame(u64),
}

struct TrayApp {
    tray_icon: Option<TrayIcon>,
    controllers: Vec<ControllerStatus>,
    /// Last HID presence snapshot (serials), used to detect connect/disconnect.
    last_discovered: Vec<String>,
    /// True while an identify flash sequence is running (pulse should yield).
    identifying: Arc<AtomicBool>,
    /// True while a background battery poll is in flight.
    refreshing: Arc<AtomicBool>,
    /// Controllers currently in the critical low-battery bucket (serial, percent).
    low_battery: Arc<Mutex<Vec<(String, u8)>>>,
    last_battery_poll: Instant,
    proxy: EventLoopProxy<UserEvent>,
    prefs: Prefs,
    known: KnownControllers,
    notify: NotifyTracker,
    configure: Option<ConfigureWindow>,
    controller_popup: Option<ControllerPopup>,
    toast: Option<ToastWindow>,
    toast_queue: VecDeque<ToastMessage>,
    toast_deadline: Option<Instant>,
    toast_hotkey: Option<EscapeHotkey>,
    toast_generation: u64,
    toast_animation_started: Instant,
    toast_dismissing: bool,
    #[cfg(feature = "dev-emulate")]
    dev_mode: bool,
    #[cfg(feature = "dev-emulate")]
    emulating: bool,
}

#[cfg(feature = "dev-emulate")]
pub fn run(dev_mode: bool) -> Result<(), Box<dyn std::error::Error>> {
    run_inner(dev_mode)
}

#[cfg(not(feature = "dev-emulate"))]
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    run_inner()
}

#[cfg(feature = "dev-emulate")]
fn run_inner(dev_mode: bool) -> Result<(), Box<dyn std::error::Error>> {
    run_app(dev_mode)
}

#[cfg(not(feature = "dev-emulate"))]
fn run_inner() -> Result<(), Box<dyn std::error::Error>> {
    run_app()
}

fn run_app(
    #[cfg(feature = "dev-emulate")] dev_mode: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::MenuEvent(event));
    }));

    let proxy = event_loop.create_proxy();
    TrayIconEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::TrayIconEvent(event));
    }));

    let proxy = event_loop.create_proxy();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(PRESENCE_INTERVAL);
            if proxy.send_event(UserEvent::Tick).is_err() {
                break;
            }
        }
    });

    let identifying = Arc::new(AtomicBool::new(false));
    let low_battery = Arc::new(Mutex::new(Vec::new()));
    start_low_battery_pulse_thread(Arc::clone(&identifying), Arc::clone(&low_battery));

    let prefs = Prefs::load();
    let known = KnownControllers::load();
    color::set_active_spectrum(prefs.spectrum);
    #[cfg(windows)]
    autostart::ensure_quiet_entry();

    let mut app = TrayApp {
        tray_icon: None,
        controllers: Vec::new(),
        last_discovered: Vec::new(),
        identifying,
        refreshing: Arc::new(AtomicBool::new(false)),
        low_battery,
        last_battery_poll: Instant::now(),
        proxy: event_loop.create_proxy(),
        prefs,
        known,
        notify: NotifyTracker::new(),
        configure: None,
        controller_popup: None,
        toast: None,
        toast_queue: VecDeque::new(),
        toast_deadline: None,
        toast_hotkey: None,
        toast_generation: 0,
        toast_animation_started: Instant::now(),
        toast_dismissing: false,
        #[cfg(feature = "dev-emulate")]
        dev_mode,
        #[cfg(feature = "dev-emulate")]
        emulating: false,
    };

    event_loop.run_app(&mut app)?;
    Ok(())
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

impl TrayApp {
    fn configure_settings(&self) -> ConfigureSettings {
        ConfigureSettings {
            notify_low: self.prefs.notify_low,
            notify_charged: self.prefs.notify_charged,
            notify_connect: self.prefs.notify_connect,
            notify_disconnect: self.prefs.notify_disconnect,
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

    fn sync_configure_settings(&mut self) {
        let settings = self.configure_settings();
        if let Some(window) = self.configure.as_mut() {
            window.sync_settings(settings);
        }
    }

    fn sync_low_battery(&self) {
        let list: Vec<(String, u8)> = self
            .controllers
            .iter()
            .filter(|c| c.is_low_battery() && !is_emulated_serial(&c.serial))
            .map(|c| (c.serial.clone(), c.percent))
            .collect();
        if let Ok(mut guard) = self.low_battery.lock() {
            *guard = list;
        }
    }

    fn apply_controllers(&mut self, controllers: Vec<ControllerStatus>) {
        let known_changed = self.known.sync_from_live(&controllers);
        let controllers_changed = !controllers_equivalent(&self.controllers, &controllers);
        if !controllers_changed && !known_changed {
            return;
        }
        if controllers_changed {
            let previous = std::mem::replace(&mut self.controllers, controllers);
            let events = self
                .notify
                .evaluate(&previous, &self.controllers, &self.prefs);
            self.queue_notifications(events);
            self.sync_low_battery();
        }
        self.known.save();
        self.apply_tray();
    }

    fn on_tick(&mut self) {
        #[cfg(feature = "dev-emulate")]
        if self.emulating {
            return;
        }

        if self.identifying.load(Ordering::SeqCst) || self.refreshing.load(Ordering::SeqCst) {
            return;
        }

        let discovered = match battery::list_controller_serials() {
            Ok(serials) => serials,
            Err(err) => {
                app_log::warn(format!("presence scan failed: {err}"));
                return;
            }
        };

        let membership_changed = discovered != self.last_discovered;
        self.last_discovered = discovered;

        // HID list went empty — update the tray immediately. A powered-off DualSense
        // often disappears from enumeration well before the next battery poll.
        if membership_changed && self.last_discovered.is_empty() {
            if !self.controllers.is_empty() {
                self.apply_controllers(Vec::new());
            }
            self.last_battery_poll = Instant::now();
            return;
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
            self.request_refresh();
        }
    }

    fn request_refresh(&mut self) {
        #[cfg(feature = "dev-emulate")]
        if self.emulating {
            return;
        }

        if self.identifying.load(Ordering::SeqCst) {
            return;
        }
        if self
            .refreshing
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let result = battery::poll_controllers();
            let _ = proxy.send_event(UserEvent::PollResult(result));
        });
    }

    fn on_poll_result(&mut self, result: Result<Vec<ControllerStatus>, String>) {
        self.refreshing.store(false, Ordering::SeqCst);
        self.last_battery_poll = Instant::now();

        #[cfg(feature = "dev-emulate")]
        if self.emulating {
            return;
        }

        match result {
            Ok(controllers) => self.apply_controllers(controllers),
            Err(err) => app_log::warn(format!("refresh failed: {err}")),
        }
    }

    fn apply_tray(&mut self) {
        let icon = icon::icon_for_controllers(&self.controllers);
        let tooltip = icon::tooltip_for_controllers(&self.controllers);

        let Some(tray) = self.tray_icon.as_mut() else {
            return;
        };

        let _ = tray.set_icon(Some(icon));
        let _ = tray.set_tooltip(Some(tooltip));
        self.sync_controller_popup();
    }

    fn create_tray(&mut self, event_loop: &ActiveEventLoop) {
        // Show the tray immediately, then poll in the background so a stuck HID
        // read cannot delay the icon for tens of seconds.
        self.controllers = Vec::new();
        self.last_discovered = battery::list_controller_serials().unwrap_or_default();
        self.last_battery_poll = Instant::now();
        self.sync_low_battery();

        let icon = icon::icon_for_controllers(&self.controllers);
        let tooltip = icon::tooltip_for_controllers(&self.controllers);
        let menu = self.build_menu();

        match TrayIconBuilder::new()
            .with_tooltip(tooltip)
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
        {
            Ok(tray) => self.tray_icon = Some(tray),
            Err(err) => app_log::error(format!("failed to create tray icon: {err}")),
        }

        match ToastWindow::open(event_loop, event_loop.owned_display_handle()) {
            Ok(window) => self.toast = Some(window),
            Err(err) => app_log::error(format!("failed to create toast window: {err}")),
        }

        match ControllerPopup::open(event_loop, event_loop.owned_display_handle()) {
            Ok(window) => self.controller_popup = Some(window),
            Err(err) => app_log::error(format!("failed to create controller popup: {err}")),
        }
        self.sync_controller_popup();

        self.request_refresh();
    }

    fn popup_rows(&self) -> Vec<ControllerRow> {
        let mut rows = Vec::new();
        for controller in &self.controllers {
            rows.push(ControllerRow::connected(
                controller,
                self.known.is_remembered(&controller.serial),
                KnownControllers::is_storable_serial(&controller.serial)
                    && !is_emulated_serial(&controller.serial),
            ));
        }
        for controller in self.known.remembered_disconnected(&self.controllers) {
            rows.push(ControllerRow::disconnected(controller));
        }
        rows
    }

    fn sync_controller_popup(&mut self) {
        let rows = self.popup_rows();
        if let Some(popup) = self.controller_popup.as_mut() {
            popup.sync(rows, self.prefs.spectrum);
        }
    }

    fn queue_notifications(&mut self, events: Vec<NotifyEvent>) {
        for event in events {
            self.toast_queue
                .push_back(ToastMessage::from_notification(event, self.prefs.spectrum));
        }
        self.show_next_toast();
    }

    fn show_next_toast(&mut self) {
        if self.toast_deadline.is_some() {
            return;
        }
        let Some(message) = self.toast_queue.pop_front() else {
            return;
        };
        let Some(window) = self.toast.as_mut() else {
            app_log::warn("toast dropped because overlay window is unavailable");
            self.toast_queue.clear();
            return;
        };
        window.show(message, self.prefs.toast_position);
        let now = Instant::now();
        self.toast_deadline = Some(now + Duration::from_secs(5));
        self.toast_animation_started = now;
        self.toast_dismissing = false;
        self.toast_generation = self.toast_generation.wrapping_add(1);
        let generation = self.toast_generation;
        self.toast_hotkey = EscapeHotkey::register(self.proxy.clone(), generation);
        self.schedule_animation_frames(generation);

        // A proxy event guarantees delivery even if the platform misses WaitUntil.
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(5));
            let _ = proxy.send_event(UserEvent::ToastDismiss(generation));
        });
    }

    fn dismiss_toast(&mut self) {
        if self.toast_deadline.is_none() || self.toast_dismissing {
            return;
        }
        self.toast_hotkey.take();
        self.toast_dismissing = true;
        self.toast_animation_started = Instant::now();
        if let Some(window) = self.toast.as_ref() {
            window.set_slide_progress(0.0, true);
        }
        self.schedule_animation_frames(self.toast_generation);
    }

    fn finish_toast(&mut self) {
        if self.toast_deadline.take().is_none() {
            return;
        }
        self.toast_hotkey.take();
        self.toast_dismissing = false;
        if let Some(window) = self.toast.as_mut() {
            window.hide();
        }
        self.show_next_toast();
    }

    fn schedule_animation_frames(&self, generation: u64) {
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let started = Instant::now();
            while started.elapsed() < SLIDE_DURATION {
                thread::sleep(Duration::from_millis(16));
                if proxy.send_event(UserEvent::ToastFrame(generation)).is_err() {
                    return;
                }
            }
            let _ = proxy.send_event(UserEvent::ToastFrame(generation));
        });
    }

    fn animate_toast(&mut self) {
        if self.toast_deadline.is_none() {
            return;
        }
        let progress = (self.toast_animation_started.elapsed().as_secs_f32()
            / SLIDE_DURATION.as_secs_f32())
        .min(1.0);
        if let Some(window) = self.toast.as_ref() {
            window.set_slide_progress(progress, self.toast_dismissing);
        }
        if self.toast_dismissing && progress >= 1.0 {
            self.finish_toast();
        }
    }

    fn show_position_preview(&mut self) {
        self.toast_queue.clear();
        if self.toast_deadline.is_some() {
            self.finish_toast();
        }
        self.toast_queue
            .push_back(ToastMessage::preview(self.prefs.spectrum.full));
        self.show_next_toast();
    }

    fn update_control_flow(&self, event_loop: &ActiveEventLoop) {
        let wake_at = if self.toast_dismissing {
            Some(self.toast_animation_started + SLIDE_DURATION)
        } else {
            self.toast_deadline
        };
        match wake_at {
            Some(deadline) => event_loop.set_control_flow(ControlFlow::WaitUntil(deadline)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    fn on_remember_menu(&mut self, serial: &str) {
        if is_emulated_serial(serial) {
            return;
        }

        if self.known.is_remembered(serial) {
            self.known.forget(serial);
        } else if let Some(controller) = self.controllers.iter().find(|c| c.serial == serial) {
            self.known.remember(controller);
        }

        self.known.save();
        self.apply_tray();
    }

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

    fn set_notification(&mut self, setting: NotificationSetting, enabled: bool) {
        match setting {
            NotificationSetting::Connect => self.prefs.notify_connect = enabled,
            NotificationSetting::Disconnect => self.prefs.notify_disconnect = enabled,
            NotificationSetting::Low => self.prefs.notify_low = enabled,
            NotificationSetting::Charged => self.prefs.notify_charged = enabled,
        }
        self.prefs.save();
        self.sync_configure_settings();
    }

    fn set_toast_position(&mut self, position: ToastPosition) {
        self.prefs.toast_position = position;
        self.prefs.save();
        self.sync_configure_settings();
        self.show_position_preview();
    }

    #[cfg(windows)]
    fn set_autostart(&mut self, enabled: bool) {
        if let Err(err) = autostart::set_enabled(enabled) {
            app_log::error(format!("autostart toggle failed: {err}"));
        }
        self.sync_configure_settings();
    }

    fn open_configure(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = self.configure.as_ref() {
            window.focus();
            return;
        }

        match ConfigureWindow::open(
            event_loop,
            event_loop.owned_display_handle(),
            self.prefs.spectrum,
            self.configure_settings(),
        ) {
            Ok(window) => self.configure = Some(window),
            Err(err) => app_log::error(format!("failed to open configure window: {err}")),
        }
    }

    fn apply_spectrum(&mut self, spectrum: BatterySpectrum) {
        self.prefs.spectrum = spectrum;
        self.prefs.save();
        color::set_active_spectrum(spectrum);

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
        self.sync_controller_popup();
    }

    fn on_configure_action(&mut self, action: ConfigureAction) {
        match action {
            ConfigureAction::None => {}
            ConfigureAction::ApplySpectrum(spectrum) => self.apply_spectrum(spectrum),
            ConfigureAction::SetNotification(setting, enabled) => {
                self.set_notification(setting, enabled)
            }
            ConfigureAction::SelectToastPosition(position) => self.set_toast_position(position),
            #[cfg(windows)]
            ConfigureAction::SetAutostart(enabled) => self.set_autostart(enabled),
            #[cfg(feature = "dev-emulate")]
            ConfigureAction::DeveloperPreset(preset) => self.apply_dev_preset(preset),
            ConfigureAction::Closed => {
                self.configure = None;
            }
        }
    }

    fn on_popup_action(&mut self, event_loop: &ActiveEventLoop, action: PopupAction) {
        match action {
            PopupAction::None => {}
            PopupAction::Identify(serial) => self.identify(&serial),
            PopupAction::ToggleRemember(serial) => self.on_remember_menu(&serial),
            PopupAction::OpenSettings => {
                if let Some(popup) = self.controller_popup.as_mut() {
                    popup.hide();
                }
                self.open_configure(event_loop);
            }
            PopupAction::Closed => {
                if let Some(popup) = self.controller_popup.as_mut() {
                    popup.hide();
                }
            }
        }
    }

    fn on_tray_icon_event(&mut self, event: TrayIconEvent) {
        if let TrayIconEvent::Click {
            rect,
            button: TrayMouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            let anchor = TrayAnchor {
                x: rect.position.x.round() as i32,
                y: rect.position.y.round() as i32,
                width: rect.size.width,
                height: rect.size.height,
            };
            if let Some(popup) = self.controller_popup.as_mut() {
                popup.toggle(anchor);
            }
        }
    }

    #[cfg(feature = "dev-emulate")]
    fn apply_dev_preset(&mut self, preset: Preset) {
        if preset == Preset::Clear {
            self.emulating = false;
            self.apply_controllers(Vec::new());
            self.last_battery_poll = Instant::now();
            self.request_refresh();
            return;
        }

        let next = emulate::apply_preset(preset, &self.controllers);
        self.emulating = true;
        self.apply_controllers(next);
    }

    fn build_menu(&self) -> Menu {
        let menu = Menu::new();
        let settings = MenuItem::with_id(SETTINGS_ID, "Settings", true, None);
        let _ = menu.append(&settings);
        let quit = MenuItem::with_id(QUIT_ID, "Exit", true, None);
        let _ = menu.append(&quit);
        menu
    }
}

impl ApplicationHandler<UserEvent> for TrayApp {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: winit::event::WindowEvent,
    ) {
        if self
            .controller_popup
            .as_ref()
            .is_some_and(|window| window.window_id() == window_id)
        {
            let action = self
                .controller_popup
                .as_mut()
                .map(|window| window.handle(&event))
                .unwrap_or(PopupAction::None);
            self.on_popup_action(event_loop, action);
            self.update_control_flow(event_loop);
            return;
        }

        if self
            .toast
            .as_ref()
            .is_some_and(|window| window.window_id() == window_id)
        {
            let dismiss = self
                .toast
                .as_mut()
                .is_some_and(|window| window.handle(&event));
            if dismiss {
                self.dismiss_toast();
            }
            self.update_control_flow(event_loop);
            return;
        }

        let action = {
            let Some(window) = self.configure.as_mut() else {
                return;
            };
            if window.window_id() != window_id {
                return;
            }
            window.handle(&event)
        };
        self.on_configure_action(action);
        self.update_control_flow(event_loop);
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        if cause == StartCause::Init {
            self.create_tray(event_loop);
        }
        let now = Instant::now();
        if self.toast_dismissing && now >= self.toast_animation_started + SLIDE_DURATION {
            self.finish_toast();
        } else if self.toast_deadline.is_some_and(|deadline| now >= deadline) {
            self.dismiss_toast();
        }
        self.update_control_flow(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Tick => self.on_tick(),
            UserEvent::PollResult(result) => self.on_poll_result(result),
            UserEvent::ToastDismiss(generation) if generation == self.toast_generation => {
                self.dismiss_toast()
            }
            UserEvent::ToastDismiss(_) => {}
            UserEvent::ToastFrame(generation) if generation == self.toast_generation => {
                self.animate_toast()
            }
            UserEvent::ToastFrame(_) => {}
            UserEvent::TrayIconEvent(event) => self.on_tray_icon_event(event),
            UserEvent::MenuEvent(event) => {
                let id = event.id.as_ref();
                if id == QUIT_ID {
                    self.tray_icon.take();
                    event_loop.exit();
                } else if id == SETTINGS_ID {
                    self.open_configure(event_loop);
                }
            }
        }
        self.update_control_flow(event_loop);
    }
}
