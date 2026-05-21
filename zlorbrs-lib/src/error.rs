use std::{error::Error, fmt, path::PathBuf};

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
    Other(String),
}

impl fmt::Display for ZlorbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZlorbError::Io(error) => write!(f, "IO error: {}", error),
            ZlorbError::HomeDirNotFound => write!(f, "Could not find home directory"),
            ZlorbError::ConfigNotFound(path) => write!(f, "Config file not found: {:?}", path),
            ZlorbError::ConfigParseError(msg) => write!(f, "Failed to parse config: {}", msg),
            ZlorbError::InvalidConfig(msg) => write!(f, "Invalid configuration: {}", msg),
            ZlorbError::FileNotFOund(path) => write!(f, "File not found: {:?}", path),
            ZlorbError::PermissionDenied(path) => write!(f, "Permission denied: {:?}", path),
            ZlorbError::SerializationError(msg) => write!(f, "Serialization error: {}", msg),
            ZlorbError::Other(msg) => write!(f, "Other error occured: {}", msg),
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
