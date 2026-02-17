#[cfg_attr(windows, path = "windows.rs")]
mod server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    server::start().await?;

    Ok(())
}
