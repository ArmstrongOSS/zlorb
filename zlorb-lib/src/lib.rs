pub mod config;
pub mod error;
pub mod log;

use std::path::PathBuf;

use crate::{error::ZlorbError, log::Logger};

pub fn get_home_dir() -> PathBuf {
    match std::env::home_dir() {
        Some(x) => x,
        None => {
            Logger::error("Failed to get the home directory".into());
            panic!("Program exited due to previous error");
        }
    }
}

pub fn create_file_with_content(path: PathBuf, content: &String) -> Result<String, ZlorbError> {
    std::fs::write(path, content).map_err(ZlorbError::Io)?;
    Ok(content.clone())
}

pub fn read_file_from_filesystem(path: PathBuf) -> Option<String> {
    let f = std::fs::read_to_string(path);
    if f.is_err() {
        return None;
    }
    Some(f.unwrap())
}

pub fn check_file_exist(path: PathBuf) -> bool {
    std::fs::exists(path).unwrap_or_default()
}

pub mod shared_test_utils {
    use std::sync::Mutex;
    pub static ENV_MUTEX: Mutex<()> = Mutex::new(());
}
