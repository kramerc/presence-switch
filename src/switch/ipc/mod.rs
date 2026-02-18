use std::{error::Error, fmt, path::PathBuf};

use regex::Regex;
use tokio::sync::broadcast::{Receiver, Sender};
use tokio_util::sync::CancellationToken;

use crate::switch::ipc;

#[cfg(unix)]
mod unix;

#[cfg(windows)]
mod windows;

const BUFFER_SIZE: usize = 8192;
const PREFERRED_NAME: &str = "discord-ipc-0";

#[derive(Debug)]
pub enum ServerError {
    NoNameAvailable,
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ServerError::NoNameAvailable => write!(f, "no name available"),
        }
    }
}

impl Error for ServerError {}

#[derive(Clone)]
pub struct Server {
    name: String,
    token: CancellationToken,
}

impl Server {
    pub fn create(token: CancellationToken) -> Result<Server, Box<dyn Error>> {
        let name = ipc::next_name()?;
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
        return unix::start(self).await;

        #[cfg(windows)]
        return windows::start(self).await;
    }

    pub fn path(&self) -> PathBuf {
        path(&self.name)
    }

    /// Gets names of IPCs that excludes our own
    pub fn other_ipc_names(&self) -> Vec<String> {
        let names = names();
        names.into_iter().filter(|name| *name != self.name).collect::<Vec<_>>()
    }
}

#[derive(Debug)]
pub enum ClientError {
    Closed,
    NoDiscords,
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ClientError::Closed => write!(f, "client is closed"),
            ClientError::NoDiscords => write!(f, "no Discord IPCs found"),
        }
    }
}

impl Error for ClientError {}

pub struct Client<S> {
    server: Server,
    socket: S,
    handshake: Vec<u8>,
    client_id: Option<String>,

    // Broadcast channels
    client_tx: Sender<Vec<u8>>,
    client_rx: Receiver<Vec<u8>>,
    discord_tx: Sender<Vec<u8>>,
    discord_rx: Receiver<Vec<u8>>,
}

trait ClientOps {
    async fn handle(&mut self) -> Result<(), Box<dyn Error>>;
    async fn handshake(&mut self) -> Result<(), Box<dyn Error>>;
    async fn relay(&mut self) -> Result<(), Box<dyn Error>>;
}

impl<S> Client<S> {
    pub fn connected(&self) {
        tracing::info!("[Client {}] Connected", self.client_id());
    }

    pub fn disconnected(&self) {
        tracing::info!("[Client {}] Disconnected", self.client_id());
    }

    /// Configures a client from a handshake
    pub fn configure(&mut self, handshake: Vec<u8>) {
        self.handshake = handshake;

        let handshake_str = String::from_utf8_lossy(&self.handshake);
        tracing::debug!("Read handshake: {}", handshake_str);

        let re = Regex::new(r#""client_id":"(\d+)""#).unwrap();
        if let Some(caps) = re.captures(&handshake_str) {
            self.client_id = Some(caps[1].to_string());
        } else {
            tracing::warn!("Failed to parse client ID from handshake");
        }
    }

    pub fn client_id(&self) -> String {
        self.client_id.clone().unwrap_or(String::from("Unidentified"))
    }
}

#[derive(Clone)]
pub struct Discord<S> {
    name: String,
    socket: S
}

pub fn names() -> Vec<String> {
    let dir = dir();
    let mut pipes = Vec::new();

    for i in 0..10 {
        let mut path = dir.clone();
        let name = format!("discord-ipc-{}", i);
        path.push(&name);
        if path.as_path().exists() {
            pipes.push(name);
        }
    }

    pipes
}

pub fn next_name() -> Result<String, Box<dyn Error>> {
    let dir = dir();

    for i in 0..10 {
        let mut path = dir.clone();
        let name = format!("discord-ipc-{}", i);
        path.push(&name);
        if !path.as_path().exists() {
            return Ok(name);
        }
    }

    Err(Box::new(ServerError::NoNameAvailable))
}

pub fn dir() -> PathBuf {
    #[cfg(unix)]
    {
        let path = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
            .iter()
            .find_map(|var| std::env::var(var).ok())
            .unwrap_or(String::from("/tmp"));

        PathBuf::from(path)
    }

    #[cfg(windows)]
    PathBuf::from(r"\\.\pipe")
}

pub fn path(name: &String) -> PathBuf {
    let mut dir = ipc::dir();
    dir.push(name.clone());
    dir
}
