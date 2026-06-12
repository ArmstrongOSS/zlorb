use std::{error::Error, fmt, path::PathBuf};

use crate::log::Logger;

#[derive(Debug)]
pub enum ZlorbError {
    Io(std::io::Error),
    HomeDirNotFound,
    ConfigNotFound(PathBuf),
    ConfigParseError(String),
    InvalidConfig(String),
    FileNotFound(PathBuf),
    PermissionDenied(String),
    SerializationErrorGeneric(String),
    SerializationError(serde_json::Error),
    TomlSerializationError(toml::ser::Error),
    TomlDeserializationError(toml::de::Error),
    Git(git2::Error),
    Other(String),
}

impl ZlorbError {
    pub fn print(&self) {
        println!("{}", self);
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
            ZlorbError::FileNotFound(path) => {
                Logger::error(format!("File not found: {path:?}"));
                Ok(())
            }
            ZlorbError::PermissionDenied(path) => {
                Logger::error(format!("Permission denied: {path:?}"));
                Ok(())
            }
            ZlorbError::SerializationErrorGeneric(msg) => {
                Logger::error(format!("Generic Serialization error: {msg}"));
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
            ZlorbError::SerializationError(error) => {
                Logger::error(format!("Serialization error: {error}"));
                Ok(())
            }
            ZlorbError::TomlSerializationError(error) => {
                Logger::error(format!("Toml Serialization error: {error}"));
                Ok(())
            }
            ZlorbError::TomlDeserializationError(error) => {
                Logger::error(format!("Toml Deserialization error: {error}"));
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

impl From<std::io::Error> for ZlorbError {
    fn from(value: std::io::Error) -> Self {
        ZlorbError::Io(value)
    }
}
