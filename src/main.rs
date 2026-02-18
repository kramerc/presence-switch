use tokio_util::sync::CancellationToken;

mod discord;
mod switch;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = CancellationToken::new();

    // Set up logging with tracing
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    // Handle interrupts
    let interrupt_token = token.clone();
    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(_) => tracing::info!("Received Ctrl+C"),
            Err(e) => tracing::error!("Unable to listen for shutdown signal: {}", e),
        }

        interrupt_token.cancel();
    });

    // Start the switch IPC server
    let server = switch::ipc::Server::create(token.clone())?;
    server.start().await
}
