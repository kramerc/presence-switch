use std::{error::Error, fmt, path::PathBuf};

use serde::Deserialize;
use tokio::sync::{broadcast, mpsc};
use tokio_util::bytes::{BufMut, BytesMut};

use crate::discord::ipc::error::IpcError;

mod error;

#[cfg(unix)]
mod unix;

#[cfg(windows)]
mod windows;

#[derive(Clone, Debug)]
pub struct Data {
    pub opcode: OpCode,
    pub msg: String,
}

impl Data {
    pub fn len(&self) -> usize {
        self.msg.len()
    }

    pub fn to_buf(&self) -> BytesMut {
        let opcode = self.opcode as u32;
        let length = self.len() as u32;
        let mut buf = BytesMut::new();
        buf.put_u32_le(opcode);
        buf.put_u32_le(length);
        buf.put(self.msg.as_bytes());
        buf
    }

    pub fn to_json_value<T: for<'a> Deserialize<'a>>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str::<T>(&self.msg)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum OpCode {
    Handshake = 0,
    Frame = 1,
    Close = 2,
    Ping = 3,
    Pong = 4,
}

impl OpCode {
    pub fn from_u32(n: u32) -> Result<OpCode, IpcError> {
        match n {
            0 => Ok(OpCode::Handshake),
            1 => Ok(OpCode::Frame),
            2 => Ok(OpCode::Close),
            3 => Ok(OpCode::Ping),
            4 => Ok(OpCode::Pong),
            _ => Err(IpcError::InvalidOpCode)
        }
    }
}

impl fmt::Display for OpCode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", *self as u32)
    }
}

pub struct Client {
    pub name: String,
    channel: (broadcast::Sender<Data>, broadcast::Receiver<Data>),
    switch_tx: mpsc::UnboundedSender<Data>
}

impl Client {
    pub fn new(name: &String, channel: &(broadcast::Sender<Data>, broadcast::Receiver<Data>), switch_tx: mpsc::UnboundedSender<Data>) -> Client {
        let (tx, rx) = channel;

        Client {
            name: name.clone(),
            channel: (tx.clone(), rx.resubscribe()),
            switch_tx,
        }
    }

    pub async fn connect(&self) -> Result<(), Box<dyn Error>> {
        tracing::debug!("Connecting to {}", self.name);

        #[cfg(unix)]
        unix::connect(self).await?;

        #[cfg(windows)]
        windows::connect(self).await?;

        tracing::debug!("Connected to {}", self.name);

        Ok(())
    }
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

    Err(Box::new(IpcError::NoNameAvailable))
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
    let mut dir = dir();
    dir.push(name.clone());
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcode_from_u32_valid() {
        assert!(matches!(OpCode::from_u32(0), Ok(OpCode::Handshake)));
        assert!(matches!(OpCode::from_u32(1), Ok(OpCode::Frame)));
        assert!(matches!(OpCode::from_u32(2), Ok(OpCode::Close)));
        assert!(matches!(OpCode::from_u32(3), Ok(OpCode::Ping)));
        assert!(matches!(OpCode::from_u32(4), Ok(OpCode::Pong)));
    }

    #[test]
    fn opcode_from_u32_invalid() {
        assert!(matches!(OpCode::from_u32(5), Err(IpcError::InvalidOpCode)));
        assert!(matches!(OpCode::from_u32(255), Err(IpcError::InvalidOpCode)));
    }

    #[test]
    fn opcode_display() {
        assert_eq!(format!("{}", OpCode::Handshake), "0");
        assert_eq!(format!("{}", OpCode::Frame), "1");
        assert_eq!(format!("{}", OpCode::Close), "2");
        assert_eq!(format!("{}", OpCode::Ping), "3");
        assert_eq!(format!("{}", OpCode::Pong), "4");
    }

    #[test]
    fn data_len() {
        let data = Data {
            opcode: OpCode::Frame,
            msg: String::from("hello"),
        };
        assert_eq!(data.len(), 5);
    }

    #[test]
    fn data_to_buf_format() {
        let data = Data {
            opcode: OpCode::Frame,
            msg: String::from("test"),
        };
        let buf = data.to_buf();
        // 4 bytes opcode + 4 bytes length + 4 bytes message
        assert_eq!(buf.len(), 12);

        // Opcode: Frame = 1, little-endian
        assert_eq!(&buf[0..4], &1u32.to_le_bytes());
        // Length: 4, little-endian
        assert_eq!(&buf[4..8], &4u32.to_le_bytes());
        // Message payload
        assert_eq!(&buf[8..], b"test");
    }

    #[test]
    fn data_to_buf_empty_message() {
        let data = Data {
            opcode: OpCode::Handshake,
            msg: String::new(),
        };
        let buf = data.to_buf();
        assert_eq!(buf.len(), 8);
        assert_eq!(&buf[0..4], &0u32.to_le_bytes());
        assert_eq!(&buf[4..8], &0u32.to_le_bytes());
    }

    #[test]
    fn data_to_json_value() {
        let data = Data {
            opcode: OpCode::Handshake,
            msg: String::from(r#"{"v":1,"client_id":"123456"}"#),
        };
        let handshake: crate::discord::api::Handshake = data.to_json_value().unwrap();
        assert_eq!(handshake.v, 1);
        assert_eq!(handshake.client_id, "123456");
    }

    #[test]
    fn data_to_json_value_invalid() {
        let data = Data {
            opcode: OpCode::Frame,
            msg: String::from("not json"),
        };
        let result: Result<crate::discord::api::Handshake, _> = data.to_json_value();
        assert!(result.is_err());
    }

    #[test]
    fn path_appends_name() {
        let name = String::from("discord-ipc-0");
        let result = path(&name);
        assert!(result.ends_with("discord-ipc-0"));
    }
}
