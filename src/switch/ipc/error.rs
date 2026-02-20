use std::fmt;

#[derive(Debug)]
pub enum SwitchError {
    NoDiscords,
}

impl fmt::Display for SwitchError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SwitchError::NoDiscords => write!(f, "no Discord IPCs available"),
        }
    }
}

impl std::error::Error for SwitchError {}
