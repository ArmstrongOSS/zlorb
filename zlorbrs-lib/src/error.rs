use std::{error::Error, fmt, path::PathBuf};

use log::error;

#[derive(Debug)]
pub enum ZlorbError {
    Io(std::io::Error),
    HomeDirNotFound,
    ConfigNotFound(PathBuf),
    ConfigParseError(String),
    InvalidConfig(String),
    FileNotFOund(PathBuf),
    PermissionDenied(String),
    SerializationError(String),
    Git(git2::Error),
    Other(String),
}

impl ZlorbError {
    pub fn print(&self) {
        error!("{}", self)
    }
}

impl fmt::Display for ZlorbError {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZlorbError::Io(error) => Ok(error!("IO error: {error}")),
            ZlorbError::HomeDirNotFound => Ok(error!("Could not find home directory")),
            ZlorbError::ConfigNotFound(path) => Ok(error!("Config file not found: {path:?}")),
            ZlorbError::ConfigParseError(msg) => Ok(error!("Failed to parse config: {msg}")),
            ZlorbError::InvalidConfig(msg) => Ok(error!("Invalid configuration: {msg}")),
            ZlorbError::FileNotFOund(path) => Ok(error!("File not found: {path:?}")),
            ZlorbError::PermissionDenied(path) => Ok(error!("Permission denied: {path:?}")),
            ZlorbError::SerializationError(msg) => Ok(error!("Serialization error: {msg}")),
            ZlorbError::Other(msg) => Ok(error!("Other error occured: {msg}")),
            ZlorbError::Git(git) => Ok(error!("Recieved git error: {git}")),
        }
    }
}

impl Error for ZlorbError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ZlorbError::Io(e) => Some(e),
            _ => None,
        }
    }
}
