use tokio_util::sync::CancellationToken;
use tray_icon::menu::{Menu, MenuId, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use winit::{application::ApplicationHandler, event_loop::EventLoop};

mod discord;
mod switch;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = CancellationToken::new();

    // Set up logging with tracing
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    // Build the Tokio runtime manually so the main thread is free to drive the
    // winit event loop, which must run on the main thread on most platforms.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    // Handle interrupts
    let interrupt_token = token.clone();
    runtime.spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(_) => tracing::info!("Received Ctrl+C"),
            Err(e) => tracing::error!("Unable to listen for shutdown signal: {}", e),
        }

        interrupt_token.cancel();
    });

    // Start the switch IPC server on the runtime.
    let server = switch::ipc::Server::new(token.clone())?;
    let server_token = token.clone();
    let server_handle = runtime.spawn(async move {
        if let Err(e) = server.start().await {
            tracing::error!("Switch IPC server stopped: {}", e);
        }

        // Make sure the event loop exits if the server stops on its own.
        server_token.cancel();
    });

    // Run the winit event loop on the main thread until shutdown is requested.
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;

    // Wake the event loop when shutdown is requested so it can exit.
    let proxy = event_loop.create_proxy();
    let shutdown_token = token.clone();
    runtime.spawn(async move {
        shutdown_token.cancelled().await;
        let _ = proxy.send_event(UserEvent::Shutdown);
    });

    // Forward tray menu events into the event loop so they're handled on the
    // main thread alongside everything else.
    let menu_proxy = event_loop.create_proxy();
    tray_icon::menu::MenuEvent::set_event_handler(Some(move |event| {
        let _ = menu_proxy.send_event(UserEvent::MenuEvent(event));
    }));

    let mut app = App {
        tray: None,
        quit_id: None,
        token: token.clone(),
    };
    event_loop.run_app(&mut app)?;

    // The event loop has exited; ensure background tasks wind down and wait for
    // the server to finish before tearing the runtime down.
    token.cancel();
    let _ = runtime.block_on(server_handle);

    Ok(())
}

enum UserEvent {
    MenuEvent(tray_icon::menu::MenuEvent),
    Shutdown,
}

struct App {
    tray: Option<TrayIcon>,
    quit_id: Option<MenuId>,
    token: CancellationToken,
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);

        if self.tray.is_none() {
            let icon = Icon::from_rgba(vec![0; 32 * 32 * 4], 32, 32).unwrap();

            let menu = Menu::new();
            let quit_item = MenuItem::new("Quit", true, None);
            menu.append(&quit_item).unwrap();
            self.quit_id = Some(quit_item.id().clone());

            let tray = TrayIconBuilder::new()
                .with_tooltip("presence-switch")
                .with_icon(icon)
                .with_menu(Box::new(menu))
                .build()
                .unwrap();

            self.tray = Some(tray);
        }
    }

    fn user_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::MenuEvent(menu_event) => {
                if self.quit_id.as_ref() == Some(&menu_event.id) {
                    // Trigger graceful shutdown; the runtime task watching the
                    // token will send UserEvent::Shutdown to exit the loop.
                    self.token.cancel();
                }
            }
            UserEvent::Shutdown => event_loop.exit(),
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        _event: winit::event::WindowEvent,
    ) {
        // no-op
    }
}
