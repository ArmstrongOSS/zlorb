pub mod config;
pub mod error;

use std::path::PathBuf;

use log::error;

use crate::error::ZlorbError;

pub fn get_home_dir() -> PathBuf {
    let home_dir = match std::env::home_dir() {
        Some(x) => x,
        None => {
            error!("Failed to get the home directory");
            panic!("Program exited due to previous error");
        }
    };
    home_dir
}

pub fn create_file_with_content(path: PathBuf, content: &String) -> Result<String, ZlorbError> {
    std::fs::write(path, content).map_err(|e| ZlorbError::Io(e))?;
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
    match std::fs::exists(path) {
        Ok(val) => val,
        Err(_) => false,
    }
}

pub mod shared_test_utils {
    use std::sync::Mutex;
    pub static ENV_MUTEX: Mutex<()> = Mutex::new(());
}
