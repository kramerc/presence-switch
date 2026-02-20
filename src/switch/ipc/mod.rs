use std::{error::Error, path::PathBuf};

use tokio::sync::{broadcast, mpsc::{self}};
use tokio_util::sync::CancellationToken;

use crate::{discord::{self, ipc::{Data, OpCode}}, switch::ipc::error::SwitchError};

mod error;

#[cfg(unix)]
mod unix;

#[cfg(windows)]
mod windows;

const PREFERRED_NAME: &str = "discord-ipc-0";

#[derive(Clone)]
pub struct Server {
    name: String,
    token: CancellationToken,
}

impl Server {
    pub fn new(token: CancellationToken) -> Result<Server, Box<dyn Error>> {
        let name = discord::ipc::next_name()?;
        tracing::info!("Creating switch IPC with name {}", name);
        if name != PREFERRED_NAME {
            // Most clients use the first IPC name, so warn the user if we couldn't use it
            tracing::warn!("Warning: Preferred name {} is not available. Using {} instead.", PREFERRED_NAME, name);
            tracing::warn!("Most RPC clients use the first available one, so desired behavior may not be achieved.");
            tracing::warn!("Consider closing all Discord instances and running this program first to ensure the preferred name is used.");
        }

        Ok(Server {
            name,
            token,
        })
    }

    pub async fn start(self) -> Result<(), Box<dyn Error>> {
        #[cfg(unix)]
        unix::start(self).await?;

        #[cfg(windows)]
        windows::start(self).await?;

        Ok(())
    }

    pub fn path(&self) -> PathBuf {
        discord::ipc::path(&self.name)
    }

    /// Gets names of IPCs that excludes our own
    pub fn other_ipc_names(&self) -> Vec<String> {
        let names = discord::ipc::names();
        names.into_iter().filter(|name| *name != self.name).collect::<Vec<_>>()
    }
}

pub struct Client {
    server: Server,
    handshake: Option<discord::api::Handshake>,
    app_data: Option<discord::api::ApplicationRpcData>,
    switch_tx: mpsc::UnboundedSender<Data>,
    discord_ipc_clients: Vec<discord::ipc::Client>,
    discord_channel: (broadcast::Sender<Data>, broadcast::Receiver<Data>),
    closed: bool,
}

impl Client {
    pub fn new(server: Server, tx: mpsc::UnboundedSender<Data>) -> Client {
        let discord_channel = broadcast::channel::<Data>(16);

        Client {
            server,
            handshake: None,
            app_data: None,
            switch_tx: tx,
            discord_ipc_clients: vec![],
            discord_channel,
            closed: false,
        }
    }

    pub fn connected(&self) {
        tracing::info!("{} client connected", self.id());
    }

    pub fn disconnected(&self) {
        tracing::info!("{} client disconnected", self.id());
    }

    pub fn id(&self) -> String {
        let client_id = match self.handshake.clone() {
            Some(handshake) => handshake.client_id,
            None => String::from("Unidentified"),
        };

        match self.app_data.as_ref() {
            Some(data) => data.name.clone(),
            None => client_id,
        }
    }

    pub async fn handle(&mut self, data: Data) -> Result<(), Box<dyn Error>> {
        tracing::trace!("Handling {}", data.msg);

        match data.opcode {
            OpCode::Handshake => self.handshake(data).await?,
            OpCode::Ping => self.ping().await?,
            OpCode::Close => self.close().await?,
            _ => self.relay(data).await?
        }

        Ok(())
    }

    async fn handshake(&mut self, data: Data) -> Result<(), Box<dyn Error>> {
        let handshake: discord::api::Handshake = data.to_json_value()?;
        let client_id = handshake.client_id.clone();
        self.handshake = Some(handshake);

        self.app_data = match crate::discord::api::application_rpc(&client_id).await {
            Ok(data) => Some(data),
            Err(e) => {
                tracing::error!("Unable to retrieve client application metadata from Discord: {}", e);
                None
            },
        };

        self.connected();
        self.setup_discord_ipc_clients().await?;

        Ok(())
    }

    async fn ping(&mut self) -> Result<(), Box<dyn Error>> {
        // Respond with pong
        let pong = Data {
            opcode: OpCode::Pong,
            msg: String::from(format!("\"{}\"", OpCode::Pong))
        };
        self.switch_tx.send(pong)?;

        Ok(())
    }

    async fn close(&mut self) -> Result<(), Box<dyn Error>> {
        self.closed = true;

        Ok(())
    }

    async fn relay(&mut self, data: Data) -> Result<(), Box<dyn Error>> {
        //! Relay message to connected Discord clients
        // Switch Client -> Discord IPC clients
        tracing::trace!("Switch -> Discord: {}", data.msg);
        let discord_tx = self.discord_channel.0.clone();
        discord_tx.send(data)?;

        Ok(())
    }

    pub async fn setup_discord_ipc_clients(&mut self) -> Result<(), Box<dyn Error>> {
        let ipc_names = self.server.other_ipc_names();
        let mut clients = Vec::new();

        for name in ipc_names {
            let client = discord::ipc::Client::new(&name, &self.discord_channel, self.switch_tx.clone());

            if let Err(e) = client.connect().await {
                tracing::error!("[Client: {}] Failed to connect to {}: {}", self.id(), client.name, e);
                continue;
            }

            clients.push(client);
        }

        if clients.is_empty() {
            return Err(Box::new(SwitchError::NoDiscords));
        }

        self.discord_ipc_clients = clients;

        // Send handshake to Discord clients
        if let Some(handshake) = self.handshake.as_ref() {
            let discord_tx = self.discord_channel.0.clone();
            let data = Data {
                opcode: OpCode::Handshake,
                msg: serde_json::to_string(handshake)?
            };
            discord_tx.send(data)?;
        } else {
            tracing::warn!("Missing handshake data");
        }

        Ok(())
    }
}
