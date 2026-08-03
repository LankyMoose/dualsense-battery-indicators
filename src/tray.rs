use crate::app_log;
#[cfg(windows)]
use crate::autostart;
use crate::battery::{self, ControllerStatus};
use crate::color::{self, BatterySpectrum, color_for_battery_percent};
use crate::configure_ui::{self, ConfigureAction, ConfigureSettings, ConfigureWindow};
#[cfg(feature = "dev-emulate")]
use crate::emulate::{self, Preset};
use crate::icon;
use crate::lightbar::{
    self, LOW_BATTERY_ORANGE, LOW_BATTERY_PULSE_GAP_MS, LOW_BATTERY_PULSE_ON_MS,
};
use crate::notify::NotifyTracker;
use crate::prefs::Prefs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};
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
const CONFIGURE_ID: &str = "configure";
const IDENTIFY_ID_PREFIX: &str = "identify:";

#[derive(Debug)]
enum UserEvent {
    MenuEvent(MenuEvent),
    /// Periodic tick: check presence; full poll when membership changes or battery is due.
    Tick,
    /// Result of a background `poll_controllers` call.
    PollResult(Result<Vec<ControllerStatus>, String>),
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
    notify: NotifyTracker,
    configure: Option<ConfigureWindow>,
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

    let _tray_events = TrayIconEvent::receiver();

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
    color::set_active_spectrum(prefs.spectrum);

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
        notify: NotifyTracker::new(),
        configure: None,
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
        if controllers_equivalent(&self.controllers, &controllers) {
            return;
        }
        let previous = std::mem::replace(&mut self.controllers, controllers);
        self.notify
            .evaluate(&previous, &self.controllers, &self.prefs);
        self.sync_low_battery();
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
        let menu = self.build_menu();

        let Some(tray) = self.tray_icon.as_mut() else {
            return;
        };

        let _ = tray.set_icon(Some(icon));
        let _ = tray.set_tooltip(Some(tooltip));
        tray.set_menu(Some(Box::new(menu)));
    }

    fn create_tray(&mut self) {
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
            .build()
        {
            Ok(tray) => self.tray_icon = Some(tray),
            Err(err) => app_log::error(format!("failed to create tray icon: {err}")),
        }

        self.request_refresh();
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

    #[cfg(windows)]
    fn on_autostart_menu(&mut self) {
        // muda toggles the checkmark before delivering the event; mirror that state.
        let enable = self
            .configure
            .as_ref()
            .map(|w| w.menu_bar().autostart.is_checked())
            .unwrap_or_else(|| !autostart::is_enabled());
        if let Err(err) = autostart::set_enabled(enable) {
            app_log::error(format!("autostart toggle failed: {err}"));
            self.sync_configure_settings();
            return;
        }
        self.sync_configure_settings();
    }

    fn on_notify_low_menu(&mut self) {
        // muda toggles the checkmark before delivering the event; mirror that state.
        self.prefs.notify_low = self
            .configure
            .as_ref()
            .map(|w| w.menu_bar().notify_low.is_checked())
            .unwrap_or(!self.prefs.notify_low);
        self.prefs.save();
    }

    fn on_notify_charged_menu(&mut self) {
        self.prefs.notify_charged = self
            .configure
            .as_ref()
            .map(|w| w.menu_bar().notify_charged.is_checked())
            .unwrap_or(!self.prefs.notify_charged);
        self.prefs.save();
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
    }

    fn on_configure_action(&mut self, action: ConfigureAction) {
        match action {
            ConfigureAction::None => {}
            ConfigureAction::ApplySpectrum(spectrum) => self.apply_spectrum(spectrum),
            ConfigureAction::Closed => {
                self.configure = None;
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

        if self.controllers.is_empty() {
            let empty = MenuItem::new("No DualSense connected", false, None);
            let _ = menu.append(&empty);
        } else {
            let hint = MenuItem::new("Click a controller to identify", false, None);
            let _ = menu.append(&hint);
            let _ = menu.append(&PredefinedMenuItem::separator());

            for controller in &self.controllers {
                let id = format!("{IDENTIFY_ID_PREFIX}{}", controller.serial);
                let item = MenuItem::with_id(id, controller.menu_label(), true, None);
                let _ = menu.append(&item);
            }
        }

        let _ = menu.append(&PredefinedMenuItem::separator());

        let configure = MenuItem::with_id(CONFIGURE_ID, "Configure", true, None);
        let _ = menu.append(&configure);

        let quit = MenuItem::with_id(QUIT_ID, "Exit", true, None);
        let _ = menu.append(&quit);
        menu
    }
}

fn parse_identify_id(id: &str) -> Option<&str> {
    id.strip_prefix(IDENTIFY_ID_PREFIX)
}

impl ApplicationHandler<UserEvent> for TrayApp {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: winit::event::WindowEvent,
    ) {
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
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        if cause == StartCause::Init {
            self.create_tray();
        }
        event_loop.set_control_flow(ControlFlow::Wait);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Tick => self.on_tick(),
            UserEvent::PollResult(result) => self.on_poll_result(result),
            UserEvent::MenuEvent(event) => {
                let id = event.id.as_ref();
                if id == QUIT_ID {
                    self.tray_icon.take();
                    event_loop.exit();
                } else if id == CONFIGURE_ID {
                    self.open_configure(event_loop);
                } else if id == configure_ui::NOTIFY_LOW_ID {
                    self.on_notify_low_menu();
                } else if id == configure_ui::NOTIFY_CHARGED_ID {
                    self.on_notify_charged_menu();
                } else if let Some(serial) = parse_identify_id(id) {
                    self.identify(serial);
                }
                #[cfg(windows)]
                if id == configure_ui::AUTOSTART_ID {
                    self.on_autostart_menu();
                }
                #[cfg(feature = "dev-emulate")]
                if self.dev_mode {
                    if let Some(preset) = Preset::from_menu_id(id) {
                        self.apply_dev_preset(preset);
                    }
                }
            }
        }
    }
}
