// Keep the Windows release application in the system tray without a console.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod discord;
mod switch;

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod gui;

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    gui::run()
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use tokio_util::sync::CancellationToken;

    // systemd captures stdout in the journal; no display or tray is required.
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let token = CancellationToken::new();
    let interrupt_token = token.clone();
    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(_) => tracing::info!("Received Ctrl+C"),
            Err(error) => tracing::error!("Unable to listen for shutdown signal: {error}"),
        }
        interrupt_token.cancel();
    });

    let server = switch::ipc::Server::new(token)?;
    server.start().await
}
