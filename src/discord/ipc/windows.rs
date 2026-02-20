use std::error::Error;

use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::ClientOptions,
};
use tokio_util::bytes::BytesMut;

use crate::discord::{self, ipc::{Client, Data, OpCode}};

pub async fn connect(client: &Client) -> Result<(), Box<dyn Error>> {
    let path = discord::ipc::path(&client.name);
    let pipe = ClientOptions::new().open(&path)?;

    let (mut read_half, mut write_half) = tokio::io::split(pipe);
    let mut discord_rx = client.channel.1.resubscribe();

    // Writer: Switch -> Discord IPC
    let name = client.name.clone();
    tokio::spawn(async move {
        loop {
            let data = match discord_rx.recv().await {
                Ok(data) => data,
                Err(e) => {
                    // Channel is closed, closing writer
                    tracing::error!("Error reading from Discord broadcast channel: {}", e);
                    break;
                }
            };

            tracing::trace!("Switch -> Discord IPC: {} {}", name, data.msg);

            let mut buf = data.to_buf();
            write_half.write_all_buf(&mut buf).await?;
        }

        Ok::<_, io::Error>(())
    });

    // Reader: Discord IPC -> Switch
    let name = client.name.clone();
    let switch_tx = client.switch_tx.clone();
    tokio::spawn(async move {
        loop {
            // Expect a u32 indicating opcode
            let opcode = OpCode::from_u32(read_half.read_u32_le().await?)?;

            // Expect a u32 indicating length
            let length = read_half.read_u32_le().await? as usize;

            // Read the message
            let mut buf = BytesMut::with_capacity(length);
            let n = read_half.read_buf(&mut buf).await?;

            if n == 0 {
                // Client disconnected
                tracing::info!("{} disconnected", name);
                break;
            }

            match String::from_utf8(buf.to_vec()) {
                Ok(msg) => {
                    tracing::trace!("Discord IPC Read: {} {} {} {}", name, opcode, length, msg);

                    let data = Data {
                        opcode,
                        msg
                    };

                    if let Err(e) = switch_tx.send(data) {
                        tracing::error!("Could not send to mpsc channel: {}", e);
                    }
                },
                Err(e) => tracing::error!("Could not decode message to UTF-8 string: {}", e),
            }
        }

        Ok::<_, io::Error>(())
    });

    Ok(())
}
