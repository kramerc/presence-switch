use std::error::Error;

use tokio::{
    fs, io::{AsyncReadExt, AsyncWriteExt}, net::{UnixListener, UnixStream}, sync::broadcast
};

use crate::switch::ipc::{self, BUFFER_SIZE, Client, ClientError, ClientOps, Discord, Server};

impl ClientOps for Client<UnixStream> {
    async fn handle(&mut self) -> Result<(), Box<dyn Error>> {
        self.handshake().await?;
        self.connected();

        self.relay().await?;

        // Read data from switch client
        let mut buffer = [0u8; BUFFER_SIZE];
        let client_id = self.client_id();
        loop {
            tokio::select! {
                // Data from switch client -> forward to Discord broadcast channel
                result = self.socket.read(&mut buffer) => {
                    match result {
                        Ok(0) => {
                            tracing::debug!("[Client {}] Switch client closed", client_id);
                            break;
                        },
                        Ok(n) => {
                            let data = buffer[..n].to_vec();
                            tracing::debug!("[Client {}] Received data from switch client: {}", client_id, String::from_utf8_lossy(&data[..n]));
                            if let Err(e) = self.discord_tx.send(data) {
                                tracing::error!("[Client {}] Failed to send switch client data to Discord broadcast channel: {}", client_id, e);
                                break;
                            }
                        },
                        Err(e) => {
                            tracing::error!("Client {}] Error reading from switch client: {}", client_id, e);
                            break;
                        }
                    }
                }

                // Data from Discord -> forward to switch client broadcast chanenl
                result = self.client_rx.recv() => {
                    match result {
                        Ok(data) => {
                            if let Err(e) = self.socket.write_all(&data).await {
                                tracing::error!("[Client {}] Error writing to switch client: {}", client_id, e);
                                break;
                            }
                        },
                        Err(e) => {
                            tracing::error!("[Client {}] Error receiving from switch client broadcast channel: {}", client_id, e);
                            break;
                        },
                    }
                }

                _ = self.server.token.cancelled() => {
                    break;
                }
            }
        }

        Ok(())
    }

    async fn handshake(&mut self) -> Result<(), Box<dyn Error>> {
        self.socket.readable().await?;

        let mut handshake = vec![0u8; BUFFER_SIZE];
        let n = self.socket.read(&mut handshake).await?;
        if n == 0 {
            tracing::error!("Could not read handshake");
            return Err(Box::new(ClientError::Closed))
        }
        handshake.truncate(n);

        self.configure(handshake);

        Ok(())
    }

    async fn relay(&mut self) -> Result<(), Box<dyn Error>> {
        // Connect to Discord clients and pass the handshake received from the switch client
        let ipc_names = self.server.other_ipc_names()?;
        let mut discords = Vec::new();
        let client_id = self.client_id().clone();

        for name in ipc_names {
            let handshake = self.handshake.clone();
            let path = ipc::path(&name)?;

            match UnixStream::connect(path).await {
                Ok(mut stream) => {
                    tracing::info!("[Client {}] Connected to Discord IPC {}", client_id, name);

                    stream.writable().await?;
                    stream.write_all(&handshake).await?;

                    stream.readable().await?;
                    let mut response = vec![0u8; BUFFER_SIZE];
                    match stream.try_read(&mut response) {
                        Ok(0) => {
                            tracing::error!("[Client {}] {} closed connection after handshake", client_id, name);
                        },
                        Ok(n) => {
                            // Forward handshake to switch client
                            response.truncate(n);
                            self.socket.write_all(&response).await?;
                        },
                        Err(_) => todo!(),
                    }

                    discords.push(Discord {
                        name,
                        socket: stream,
                    });
                }
                Err(e) => {
                    tracing::error!("[Client {}] Failed to connect to {}: {}", client_id, name, e);
                },
            }
        }

        if discords.is_empty() {
            return Err(Box::new(ClientError::NoDiscords));
        }

        // Set up forwarders
        for discord in discords {
            let client_tx = self.client_tx.clone();
            let mut discord_rx = self.discord_rx.resubscribe();
            let client_id = client_id.clone();

            tokio::spawn(async move {
                let mut buffer = vec![0u8; BUFFER_SIZE];
                let discord_name = discord.name;
                let mut stream = discord.socket;

                loop {
                    tokio::select! {
                        // Data from Discord -> forward to switch client
                        result = stream.read(&mut buffer) => {
                            match result {
                                Ok(0) => {
                                    tracing::debug!("[Client {}] Discord client {} closed", client_id, discord_name);
                                    break;
                                }
                                Ok(n) => {
                                    let data = buffer[..n].to_vec();
                                    tracing::debug!("[Client {}] Received data from {}: {}", client_id, discord_name, String::from_utf8_lossy(&data[..n]));
                                    if let Err(e) = client_tx.send(data) {
                                        tracing::error!("[Client {}] Error sending Discord data to switch client broadcast channel: {}", client_id, e);
                                        break;
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("[Client {}] Error reading from {}: {}", client_id, discord_name, e);
                                    break;
                                }
                            }
                        }

                        // Data from Discord broadcast channel -> forward to Discord
                        result = discord_rx.recv() => {
                            match result {
                                Ok(data) => {
                                    if let Err(e) = stream.write_all(&data).await {
                                        tracing::error!("[Client {}] Error writing to Discord: {}", client_id, e);
                                        break;
                                    }
                                },
                                Err(e) => {
                                    tracing::error!("[Client {}] Error receiving from Discord broadcast channel: {}", client_id, e);
                                    break;
                                },
                            }
                        }
                    }
                }
            });
        }

        Ok(())
    }
}

pub async fn start(switch: Server) -> Result<(), Box<dyn Error>> {
    let socket_path = switch.path()?;
    let listener = UnixListener::bind(&socket_path)?;

    let switch_clone = switch.clone();
    let handler = tokio::spawn(async move {
        tracing::info!("Listening for clients");

        loop {
            let switch = switch_clone.clone();
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let (client_tx, client_rx) = broadcast::channel::<Vec<u8>>(BUFFER_SIZE);
                    let (discord_tx, discord_rx) = broadcast::channel::<Vec<u8>>(BUFFER_SIZE);

                    let mut client = Client {
                        server: switch,
                        socket: stream,
                        handshake: vec![],
                        client_id: None,
                        client_tx,
                        client_rx,
                        discord_tx,
                        discord_rx,
                    };
                    match client.handle().await {
                        Ok(_) => client.disconnected(),
                        Err(e) => tracing::error!("Error handling client: {}", e),
                    }
                }
                Err(e) => {
                    tracing::error!("Error accepting client: {}", e)
                }
            }
        }
    });

    // Gracefully handle interrupt
    switch.token.cancelled().await;
    fs::remove_file(&socket_path).await?;
    handler.abort();

    match handler.await {
        Ok(_) => Ok(()),
        Err(e) => {
            tracing::error!("Handler aborted: {:?}", e);
            tracing::info!("Closing listener");
            Ok(())
        }
    }
}
