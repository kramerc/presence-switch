use std::error::Error;

use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::ServerOptions,
    sync::mpsc,
};
use tokio_util::bytes::BytesMut;

use crate::{discord::ipc::{Data, OpCode}, switch::ipc::{Client, Server}};

async fn handle(server: Server, pipe: tokio::net::windows::named_pipe::NamedPipeServer) -> Result<(), Box<dyn Error>> {
    tracing::info!("Received new client");

    let (mut read_half, mut write_half) = tokio::io::split(pipe);
    let (tx, mut rx) = mpsc::unbounded_channel::<Data>();
    let mut client = Client::new(server, tx);

    // Writer
    tokio::spawn(async move {
        loop {
            let data = match rx.recv().await {
                Some(msg) => msg,
                None => {
                    // Channel is closed, closing writer
                    break;
                }
            };

            tracing::trace!("Switch Writer: {}", data.msg);

            let mut buf = data.to_buf();
            write_half.write_all_buf(&mut buf).await?;
        }

        Ok::<_, io::Error>(())
    });

    // Reader
    loop {
        if client.closed {
            break;
        }

        // Expect a u32 indicating opcode
        let opcode = OpCode::from_u32(read_half.read_u32_le().await?)?;

        // Expect a u32 indicating length
        let length = read_half.read_u32_le().await? as usize;

        // Read the message
        let mut buf = BytesMut::with_capacity(length);
        let n = read_half.read_buf(&mut buf).await?;

        if n == 0 {
            break;
        }

        let msg = String::from_utf8(buf.to_vec())?;
        tracing::trace!("Switch Reader: {}", msg);

        let data = Data {
            opcode,
            msg
        };

        client.handle(data).await?;
    }

    client.disconnected();

    Ok(())
}

pub async fn start(server: Server) -> Result<(), Box<dyn Error>> {
    let path = server.path();

    let mut pipe_server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&path)?;

    tracing::info!("Server listening for clients");

    loop {
        let server = server.clone();

        tokio::select! {
            result = pipe_server.connect() => {
                match result {
                    Ok(()) => {
                        let connected_pipe = pipe_server;

                        // Create a new pipe instance for the next client
                        pipe_server = ServerOptions::new().create(&path)?;

                        tokio::spawn(async move {
                            if let Err(e) = handle(server, connected_pipe).await {
                                tracing::error!("Error handling client: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        tracing::error!("Error accepting client: {}", e);
                    }
                }
            }

            _ = server.token.cancelled() => {
                tracing::info!("Cancellation token triggered, stopping server");
                break;
            }
        }
    }

    tracing::info!("Server closed");
    Ok(())
}
