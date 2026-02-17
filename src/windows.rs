use std::{error::Error, io};

use regex::Regex;
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions}, sync::broadcast};
use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;

const PREFERRED_PIPE_NAME: &str = r"\\.\pipe\discord-ipc-0";
const BUFFER_SIZE: usize = 8192;

pub async fn start() -> Result<(), Box<dyn Error>>{
    // Set up logging with tracing
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    // Determine the next available pipe name to use for our server
    let pipe_name = get_next_available_pipe();
    tracing::info!("[Switch] Creating pipe server at {}", pipe_name);
    if pipe_name != PREFERRED_PIPE_NAME {
        // Most clients use the first pipe, so warn the user if we couldn't use it
        tracing::warn!("Warning: Preferred pipe name {} is not available. Using {} instead.", PREFERRED_PIPE_NAME, pipe_name);
        tracing::warn!("Most RPC clients use the first available pipe, so desired behavior may not be achieved.");
        tracing::warn!("Consider closing Discord and running this program first to ensure the preferred pipe name is used.");
    }

    let mut server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&pipe_name)?;

    let server = tokio::spawn(async move {
        loop {
            server.connect().await?;
            let connected_client = server;

            server = ServerOptions::new().create(&pipe_name)?;

            let pipe_name_inner = pipe_name.clone();
            let _client = tokio::spawn(async move {
                tracing::info!("[Switch] Client connected! {:?}", connected_client);

                // Relay data between the connected client and the real Discord IPC pipe
                match relay(connected_client, &pipe_name_inner).await {
                    Ok(_) => {
                        tracing::info!("[Switch] Client disconnected.");
                    }
                    Err(e) => {
                        tracing::error!("[Switch] Error relaying data: {}", e);
                    }
                }
            });
        }

        #[allow(unreachable_code)]
        Ok::<_, io::Error>(())
    });

    server.await??;

    Ok(())
}

async fn relay(mut client_pipe: NamedPipeServer, switch_pipe_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (client_tx, mut client_rx) = broadcast::channel::<Vec<u8>>(BUFFER_SIZE);
    let (discord_tx, discord_rx) = broadcast::channel::<Vec<u8>>(BUFFER_SIZE);

    // Get handshake data from client before setting up the relay
    let mut handshake_buffer = vec![0u8; 8192];
    let n = client_pipe.read(&mut handshake_buffer).await?;
    if n == 0 {
        tracing::error!("[Client] Client pipe closed before handshake was read");
        return Ok(());
    }
    handshake_buffer.truncate(n);
    let handshake_str = String::from_utf8_lossy(&handshake_buffer);
    tracing::debug!("[Client] Read handshake from client: {}", handshake_str);

    // Parse handshake to get client ID
    let mut client_tag = String::from("Client");
    let re = Regex::new(r#""client_id":"(\d+)""#).unwrap();
    if let Some(caps) = re.captures(&handshake_str) {
        let client_id = &caps[1];
        client_tag = format!("Client {}", client_id);
    } else {
        tracing::warn!("[Client] Failed to parse client ID from handshake");
    }
    tracing::info!("[{}] Handshake received", client_tag);

    // Connect to all Discord pipes expect for ourselves
    let pipes = get_discord_pipes().into_iter().filter(|p| p != switch_pipe_name).collect::<Vec<_>>();
    let pipes_clone = pipes.clone();
    let mut real_pipes = Vec::new();
    for pipe in pipes {
        tracing::info!("[Switch] Connecting to real Discord IPC pipe at {}", pipe);
        match ClientOptions::new().open(&pipe) {
            Ok(mut client) => {
                tracing::info!("[Switch] Connected to real pipe: {}", pipe);

                // Pass the handshake to the real pipe
                client.write_all(&handshake_buffer).await?;
                tracing::info!("[Switch] Forwarded handshake to Discord pipe {}", pipe);

                // Read Discord's response to the handshake and forward it back to the client
                let mut response_buffer = vec![0u8; 8192];
                match tokio::time::timeout(std::time::Duration::from_secs(2), client.read(&mut response_buffer)).await {
                    Ok(Ok(n)) if n > 0 => {
                        tracing::debug!("[Switch] Read handshake response: {}", String::from_utf8_lossy(&response_buffer[..n]));
                        client_pipe.write_all(&response_buffer[..n]).await?;
                        tracing::debug!("[Switch] Forwarded Discord handshake response from pipe {} to client", pipe);
                    }
                    Ok(Ok(_)) => {
                        tracing::warn!("[Switch] Discord pipe {} closed connection immediately after handshake", pipe);
                        continue;
                    }
                    Ok(Err(e)) => {
                        tracing::error!("[Switch] Error reading handshake response from Discord pipe {}: {}", pipe, e);
                        continue;
                    }
                    Err(_) => {
                        tracing::warn!("[Switch] Timeout waiting for handshake response from Discord pipe {}", pipe);
                        continue;
                    }
                }

                real_pipes.push(client);
            },
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => (),
            Err(e) => tracing::error!("[Switch] Failed to connect to {}: {}", pipe, e)
        }
    }

    if real_pipes.is_empty() {
        return Err("No Discord IPC pipes available".into());
    }

    // Spawn a task for each real pipe to forward data between the pipe and the broadcast channels
    for (pipe_index, mut pipe) in real_pipes.into_iter().enumerate() {
        let client_tag = client_tag.clone();
        let client_tx = client_tx.clone();
        let mut discord_rx = discord_rx.resubscribe();
        let pipe_name = pipes_clone[pipe_index].clone();

        tokio::spawn(async move {
            let mut buffer = vec![0u8; BUFFER_SIZE];
            loop {
                tokio::select! {
                    // Data from real pipe -> forward to client
                    result = pipe.read(&mut buffer) => {
                        match result {
                            Ok(0) => {
                                tracing::debug!("[{}] Pipe closed", pipe_name);
                                break;
                            },
                            Ok(n) => {
                                let data = buffer[..n].to_vec();
                                tracing::debug!("[{} -> {} TX] {}", pipe_name, client_tag, String::from_utf8_lossy(&data[..data.len()]));
                                if client_tx.send(data).is_err() {
                                    tracing::error!("[{} -> {} TX] Failed to send data to broadcast channel", pipe_name, client_tag);
                                    break;
                                }
                            },
                            Err(e) => {
                                tracing::error!("[{}] Error reading from real pipe: {}", pipe_name, e);
                                break;
                            }
                        }
                    }

                    // Data from Discord broadcast channel -> forward to real pipe
                    result = discord_rx.recv() => {
                        match result {
                            Ok(data) => {
                                tracing::debug!("[Discord RX -> {}] {}", pipe_name, String::from_utf8_lossy(&data[..data.len()]));
                                if let Err(e) = pipe.write_all(&data).await {
                                    tracing::error!("[{}] Error writing to real pipe: {}", pipe_name, e);
                                    break;
                                }
                            }
                            Err(e) => {
                                tracing::error!("[Discord RX -> {}] Error receiving from broadcast channel: {}", pipe_name, e);
                                break;
                            }
                        }
                    }
                }
            }
        });
    }

    // Main relay loop to forward data between the client and the real pipes using the broadcast channels
    let mut buffer = [0u8; BUFFER_SIZE];
    loop {
        let client_tag = client_tag.clone();
        tokio::select! {
            // Data from client -> forward to Discord broadcast channel
            result = client_pipe.read(&mut buffer) => {
                match result {
                    Ok(0) => {
                        tracing::debug!("[{}] Pipe disconnected", client_tag);
                        break;
                    },
                    Ok(n) => {
                        let data = buffer[..n].to_vec();
                        tracing::debug!("[{} -> Discord TX] {}", client_tag, String::from_utf8_lossy(&data[..n]));
                        if discord_tx.send(data).is_err() {
                            tracing::error!("[{} -> Discord TX] Failed to send client data to Discord broadcast channel", client_tag);
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::error!("[{}] Error reading from client pipe: {}", client_tag, e);
                        break;
                    }
                }
            }

            // Data from any real pipe -> forward to client broadcast channel
            result = client_rx.recv() => {
                match result {
                    Ok(data) => {
                        tracing::debug!("[{} RX -> {}] {}", client_tag, client_tag, String::from_utf8_lossy(&data[..data.len()]));
                        if let Err(e) = client_pipe.write_all(&data).await {
                            tracing::error!("[{} RX -> {}] Error writing to client pipe: {}", client_tag, client_tag, e);
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::error!("[{} RX -> {}] Error receiving from client broadcast channel: {}", client_tag, client_tag, e);
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

fn get_discord_pipes() -> Vec<String> {
    let mut pipes = Vec::new();
    for i in 0..10 {
        let pipe_name = format!(r"\\.\pipe\discord-ipc-{}", i);
        if std::path::Path::new(&pipe_name).exists() {
            pipes.push(pipe_name);
        }
    }
    pipes
}

fn get_next_available_pipe() -> String {
    for i in 0..10 {
        let pipe_name = format!(r"\\.\pipe\discord-ipc-{}", i);
        if !std::path::Path::new(&pipe_name).exists() {
            return pipe_name;
        }
    }
    panic!("No available pipe names found!");
}
