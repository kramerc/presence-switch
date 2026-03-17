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
            IpcError::InvalidOpCode => std::io::Error::other(value),
            IpcError::NoNameAvailable => std::io::Error::new(std::io::ErrorKind::NotFound, value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_error_display() {
        assert_eq!(format!("{}", IpcError::InvalidOpCode), "invalid opcode");
        assert_eq!(format!("{}", IpcError::NoNameAvailable), "no name available");
    }

    #[test]
    fn ipc_error_into_io_error() {
        let io_err: std::io::Error = IpcError::InvalidOpCode.into();
        assert_eq!(io_err.kind(), std::io::ErrorKind::Other);

        let io_err: std::io::Error = IpcError::NoNameAvailable.into();
        assert_eq!(io_err.kind(), std::io::ErrorKind::NotFound);
    }
}
