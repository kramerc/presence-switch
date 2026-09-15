use std::path::{Path, PathBuf};

use tokio_util::sync::CancellationToken;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tray_icon::menu::{Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use winit::{application::ApplicationHandler, event_loop::EventLoop};

use crate::switch;

const TRAY_ICON: &[u8] = include_bytes!("../assets/icons/tray.png");

fn load_tray_icon() -> Result<Icon, Box<dyn std::error::Error>> {
    let image = image::load_from_memory(TRAY_ICON)?.into_rgba8();
    let (width, height) = image.dimensions();
    Ok(Icon::from_rgba(image.into_raw(), width, height)?)
}

/// Path to the log file the tray "Open Log" item opens.
fn log_path() -> PathBuf {
    std::env::temp_dir().join("presence-switch.log")
}

/// Open the given path in the platform's default viewer.
fn open_path(path: &Path) {
    let result = {
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("notepad").arg(path).spawn()
        }
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open").arg(path).spawn()
        }
    };

    if let Err(e) = result {
        tracing::error!("Failed to open {}: {}", path.display(), e);
    }
}

async fn run_server(
    token: CancellationToken,
    server: impl std::future::Future<Output = Result<(), Box<dyn std::error::Error>>>,
) -> std::io::Result<()> {
    // Wake the UI on success, failure, or panic while unwinding the task.
    let _shutdown = token.drop_guard();
    server.await.map_err(|error| {
        tracing::error!("Switch IPC server stopped: {error}");
        // The server's boxed error is not Send, so carry its message across
        // the runtime task boundary in an owned, Send error.
        std::io::Error::other(error.to_string())
    })
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let token = CancellationToken::new();

    // Set up logging with tracing. Write to a log file (viewable from the tray)
    // and also to stdout, which is only visible in debug builds.
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())?;
    let make_file_writer = move || log_file.try_clone().expect("clone log file handle");
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(make_file_writer.and(std::io::stdout))
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

    // Create the event loop before starting the server so a display setup
    // failure cannot skip cleanup of a running IPC server.
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;

    // Start the switch IPC server on the runtime.
    let server = switch::ipc::Server::new(token.clone())?;
    let server_token = token.clone();
    let server_handle =
        runtime.spawn(async move { run_server(server_token, server.start()).await });

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
        open_log_id: None,
        quit_id: None,
        startup_error: None,
        token: token.clone(),
    };
    let event_loop_result = event_loop.run_app(&mut app);

    // The event loop has exited; ensure background tasks wind down and wait for
    // the server to finish before tearing the runtime down.
    token.cancel();
    let server_result = runtime.block_on(server_handle);

    if let Some(error) = app.startup_error {
        return Err(error);
    }
    event_loop_result?;
    server_result??;

    Ok(())
}

enum UserEvent {
    MenuEvent(tray_icon::menu::MenuEvent),
    Shutdown,
}

struct App {
    tray: Option<TrayIcon>,
    open_log_id: Option<MenuId>,
    quit_id: Option<MenuId>,
    startup_error: Option<Box<dyn std::error::Error>>,
    token: CancellationToken,
}

impl App {
    fn create_tray(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let icon = load_tray_icon()?;
        let menu = Menu::new();
        let open_log_item = MenuItem::new("Open Log", true, None);
        let quit_item = MenuItem::new("Quit", true, None);
        menu.append(&open_log_item)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&quit_item)?;

        let tray = TrayIconBuilder::new()
            .with_tooltip("presence-switch")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .build()?;

        // Only publish the tray state once every setup step has succeeded.
        self.open_log_id = Some(open_log_item.id().clone());
        self.quit_id = Some(quit_item.id().clone());
        self.tray = Some(tray);
        Ok(())
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);

        if self.tray.is_none()
            && self.startup_error.is_none()
            && let Err(error) = self.create_tray()
        {
            tracing::error!("Failed to initialize tray: {error}");
            self.startup_error = Some(error);
            self.token.cancel();
            event_loop.exit();
        }
    }

    fn user_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::MenuEvent(menu_event) => {
                if self.open_log_id.as_ref() == Some(&menu_event.id) {
                    open_path(&log_path());
                } else if self.quit_id.as_ref() == Some(&menu_event.id) {
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

#[cfg(test)]
mod tests {
    use super::run_server;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn server_error_is_returned_and_requests_shutdown() {
        let token = CancellationToken::new();
        let result = tokio::spawn(run_server(token.clone(), async {
            Err(std::io::Error::other("bind failed").into())
        }))
        .await
        .unwrap();

        assert_eq!(result.unwrap_err().to_string(), "bind failed");
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn normal_server_exit_requests_shutdown() {
        let token = CancellationToken::new();
        run_server(token.clone(), async { Ok(()) }).await.unwrap();
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn server_panic_requests_shutdown() {
        let token = CancellationToken::new();
        let result = tokio::spawn(run_server(token.clone(), async {
            panic!("server panicked");
        }))
        .await;

        assert!(result.unwrap_err().is_panic());
        assert!(token.is_cancelled());
    }
}
