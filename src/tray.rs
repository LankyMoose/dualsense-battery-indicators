use crate::battery::{self, ControllerStatus};
use crate::icon;
use std::time::Duration;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};
use winit::application::ApplicationHandler;
use winit::event::StartCause;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const QUIT_ID: &str = "quit";

#[derive(Debug)]
enum UserEvent {
    MenuEvent(MenuEvent),
    Refresh,
}

struct TrayApp {
    tray_icon: Option<TrayIcon>,
    controllers: Vec<ControllerStatus>,
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::MenuEvent(event));
    }));

    // Keep the tray event receiver alive so the crate doesn't drop events unused.
    let _tray_events = TrayIconEvent::receiver();

    let proxy = event_loop.create_proxy();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(POLL_INTERVAL);
            if proxy.send_event(UserEvent::Refresh).is_err() {
                break;
            }
        }
    });

    let mut app = TrayApp {
        tray_icon: None,
        controllers: Vec::new(),
    };

    event_loop.run_app(&mut app)?;
    Ok(())
}

impl TrayApp {
    fn refresh(&mut self) {
        match battery::read_all_controllers() {
            Ok(controllers) => self.controllers = controllers,
            Err(err) => eprintln!("warning: refresh failed: {err}"),
        }
        self.apply_ui();
    }

    fn apply_ui(&mut self) {
        let Some(tray) = self.tray_icon.as_mut() else {
            return;
        };

        let icon = icon::icon_for_controllers(&self.controllers);
        let tooltip = icon::tooltip_for_controllers(&self.controllers);
        let menu = build_menu(&self.controllers);

        let _ = tray.set_icon(Some(icon));
        let _ = tray.set_tooltip(Some(tooltip));
        tray.set_menu(Some(Box::new(menu)));
    }

    fn create_tray(&mut self) {
        self.controllers = battery::read_all_controllers().unwrap_or_default();

        let icon = icon::icon_for_controllers(&self.controllers);
        let tooltip = icon::tooltip_for_controllers(&self.controllers);
        let menu = build_menu(&self.controllers);

        match TrayIconBuilder::new()
            .with_tooltip(tooltip)
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .build()
        {
            Ok(tray) => self.tray_icon = Some(tray),
            Err(err) => eprintln!("error: failed to create tray icon: {err}"),
        }
    }
}

fn build_menu(controllers: &[ControllerStatus]) -> Menu {
    let menu = Menu::new();

    if controllers.is_empty() {
        let empty = MenuItem::new("No DualSense connected", false, None);
        let _ = menu.append(&empty);
    } else {
        for controller in controllers {
            let item = MenuItem::new(controller.menu_label(), true, None);
            let _ = menu.append(&item);
        }
    }

    let _ = menu.append(&PredefinedMenuItem::separator());
    let quit = MenuItem::with_id(QUIT_ID, "Exit", true, None);
    let _ = menu.append(&quit);
    menu
}

impl ApplicationHandler<UserEvent> for TrayApp {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        _event: winit::event::WindowEvent,
    ) {
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        if cause == StartCause::Init {
            self.create_tray();
        }
        event_loop.set_control_flow(ControlFlow::Wait);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Refresh => self.refresh(),
            UserEvent::MenuEvent(event) => {
                if event.id.as_ref() == QUIT_ID {
                    self.tray_icon.take();
                    event_loop.exit();
                }
            }
        }
    }
}
