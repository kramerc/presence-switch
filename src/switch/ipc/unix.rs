use std::error::Error;

use tokio::{
    fs, io::{self, AsyncReadExt, AsyncWriteExt}, net::{UnixListener, UnixStream}, sync::mpsc
};
use tokio_util::bytes::BytesMut;

use crate::{discord::ipc::{Data, OpCode}, switch::ipc::{Client, Server}};

async fn handle(server: Server, stream: UnixStream) -> Result<(), Box<dyn Error>> {
    tracing::info!("Received new client");

    let (mut read_half, mut write_half) = stream.into_split();
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

            write_half.writable().await?;

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

        read_half.readable().await?;

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
    let listener = UnixListener::bind(&path)?;
    tracing::info!("Server listening for clients");

    loop {
        let server = server.clone();

        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        tokio::spawn(async move {
                            if let Err(e) = handle(server, stream).await {
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

    fs::remove_file(&path).await?;

    tracing::info!("Server closed");
    Ok(())
}
