pub mod config;
pub mod error;
pub mod log;

use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::PathBuf,
};

use crate::{config::RepositoriesConfigurationFile, error::ZlorbError, log::Logger};

pub fn get_home_dir() -> PathBuf {
    match std::env::home_dir() {
        Some(x) => x,
        None => {
            Logger::error("Failed to get the home directory".into());
            panic!("Program exited due to previous error");
        }
    }
}

pub fn get_zlorb_config_dir() -> PathBuf {
    get_home_dir().join(".config/zlorb")
}

/// gets the toml file for zlorb repo configurations
/// File: ~/.config/zlorb/repositories.toml
pub fn get_zlorb_repo_config_file() -> PathBuf {
    get_zlorb_config_dir().join("repositories.toml")
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

/// This function creates the config file if needed, and returns toml
///
/// The toml can be empty, or populated depending on the state of the file
pub fn create_config_from_toml() -> Result<(RepositoriesConfigurationFile, File), ZlorbError> {
    let config_path_dir = get_zlorb_config_dir();
    let config_file_path = config_path_dir.join("repositories.toml");
    fs::create_dir_all(&config_path_dir)?;
    let file_contents = fs::read_to_string(&config_file_path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(config_file_path)?;

    let config_loaded = toml::from_str::<RepositoriesConfigurationFile>(&file_contents).unwrap_or(
        RepositoriesConfigurationFile {
            repositories: vec![],
        },
    );

    Ok((config_loaded, file))
}
