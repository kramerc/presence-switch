use std::fmt;

#[derive(Debug)]
pub enum IpcError {
    InvalidOpCode,
    NoNameAvailable,
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            IpcError::InvalidOpCode => write!(f, "invalid opcode"),
            IpcError::NoNameAvailable => write!(f, "no name available"),
        }
    }
}

impl std::error::Error for IpcError {}
impl From<IpcError> for std::io::Error {
    fn from(value: IpcError) -> Self {
        match value {
            IpcError::InvalidOpCode => std::io::Error::new(std::io::ErrorKind::Other, value),
            IpcError::NoNameAvailable => std::io::Error::new(std::io::ErrorKind::NotFound, value),
        }
    }
}
