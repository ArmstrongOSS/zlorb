use std::{error::Error, fmt, path::PathBuf};

use crate::log::Logger;

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
        Logger::error(format!("{}", self));
    }
}

impl fmt::Display for ZlorbError {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZlorbError::Io(error) => {
                Logger::error("IO error: {error}".into());
                Ok(())
            }
            ZlorbError::HomeDirNotFound => {
                Logger::error("Could not find home directory".into());
                Ok(())
            }
            ZlorbError::ConfigNotFound(path) => {
                Logger::error("Config file not found: {path:?}".into());
                Ok(())
            }
            ZlorbError::ConfigParseError(msg) => {
                Logger::error("Failed to parse config: {msg}".into());
                Ok(())
            }
            ZlorbError::InvalidConfig(msg) => {
                Logger::error("Invalid configuration: {msg}".into());
                Ok(())
            }
            ZlorbError::FileNotFOund(path) => {
                Logger::error("File not found: {path:?}".into());
                Ok(())
            }
            ZlorbError::PermissionDenied(path) => {
                Logger::error("Permission denied: {path:?}".into());
                Ok(())
            }
            ZlorbError::SerializationError(msg) => {
                Logger::error("Serialization error: {msg}".into());
                Ok(())
            }
            ZlorbError::Other(msg) => {
                Logger::error("Other error occured: {msg}".into());
                Ok(())
            }
            ZlorbError::Git(git) => {
                Logger::error("Recieved git error: {git}".into());
                Ok(())
            }
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
