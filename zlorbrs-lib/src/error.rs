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
        Logger::Error(format!("{}", self));
    }
}

impl fmt::Display for ZlorbError {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZlorbError::Io(error) => {
                Logger::Error("IO error: {error}".into());
                Ok(())
            }
            ZlorbError::HomeDirNotFound => {
                Logger::Error("Could not find home directory".into());
                Ok(())
            }
            ZlorbError::ConfigNotFound(path) => {
                Logger::Error("Config file not found: {path:?}".into());
                Ok(())
            }
            ZlorbError::ConfigParseError(msg) => {
                Logger::Error("Failed to parse config: {msg}".into());
                Ok(())
            },
            ZlorbError::InvalidConfig(msg) => {
                Logger::Error("Invalid configuration: {msg}".into());
                Ok(())
            },
            ZlorbError::FileNotFOund(path) => {
                Logger::Error("File not found: {path:?}".into());
                Ok(())
            },
            ZlorbError::PermissionDenied(path) => {
                Logger::Error("Permission denied: {path:?}".into());
                Ok(())
            },
            ZlorbError::SerializationError(msg) => {
                Logger::Error("Serialization error: {msg}".into());
                Ok(())
            },
            ZlorbError::Other(msg) => {
                Logger::Error("Other error occured: {msg}".into());
                Ok(())
            },
            ZlorbError::Git(git) => {
                Logger::Error("Recieved git error: {git}".into());
                Ok(())
            },
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
