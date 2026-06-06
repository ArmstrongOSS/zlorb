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
                Logger::error(format!("IO error: {error}"));
                Ok(())
            }
            ZlorbError::HomeDirNotFound => {
                Logger::error("Could not find home directory".into());
                Ok(())
            }
            ZlorbError::ConfigNotFound(path) => {
                Logger::error(format!("Config file not found: {path:?}"));
                Ok(())
            }
            ZlorbError::ConfigParseError(msg) => {
                Logger::error(format!("Failed to parse config: {msg}"));
                Ok(())
            }
            ZlorbError::InvalidConfig(msg) => {
                Logger::error(format!("Invalid configuration: {msg}"));
                Ok(())
            }
            ZlorbError::FileNotFOund(path) => {
                Logger::error(format!("File not found: {path:?}"));
                Ok(())
            }
            ZlorbError::PermissionDenied(path) => {
                Logger::error(format!("Permission denied: {path:?}"));
                Ok(())
            }
            ZlorbError::SerializationError(msg) => {
                Logger::error(format!("Serialization error: {msg}"));
                Ok(())
            }
            ZlorbError::Other(msg) => {
                Logger::error(format!("Other error occured: {msg}"));
                Ok(())
            }
            ZlorbError::Git(git) => {
                Logger::error(format!("Recieved git error: {git}"));
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
