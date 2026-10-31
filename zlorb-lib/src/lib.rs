pub mod config;
pub mod error;
pub mod git;
pub mod log;

use std::{
    fs::{self, File, OpenOptions},
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

pub fn explicit_config_dir_from(
    zlorb_config_dir: Option<&str>,
    xdg_config_home: Option<&str>,
) -> Option<PathBuf> {
    if let Some(dir) = zlorb_config_dir {
        let trimmed = dir.trim();
        if !trimmed.is_empty() && std::path::Path::new(trimmed).is_absolute() {
            return Some(PathBuf::from(trimmed));
        }
    }
    if let Some(dir) = xdg_config_home {
        let trimmed = dir.trim();
        if !trimmed.is_empty() && std::path::Path::new(trimmed).is_absolute() {
            return Some(PathBuf::from(trimmed).join("zlorb"));
        }
    }
    None
}

pub fn resolve_config_dir_from(
    zlorb_config_dir: Option<&str>,
    xdg_config_home: Option<&str>,
    home_dir: &std::path::Path,
) -> PathBuf {
    match explicit_config_dir_from(zlorb_config_dir, xdg_config_home) {
        Some(dir) => dir,
        None => home_dir.join(".config").join("zlorb"),
    }
}

pub fn get_zlorb_config_dir() -> PathBuf {
    let zlorb_config_dir = std::env::var("ZLORB_CONFIG_DIR").ok();
    let xdg_config_home = std::env::var("XDG_CONFIG_HOME").ok();
    if let Some(dir) = explicit_config_dir_from(
        zlorb_config_dir.as_deref(),
        xdg_config_home.as_deref(),
    ) {
        return dir;
    }
    resolve_config_dir_from(None, None, &get_home_dir())
}

pub fn effective_uid() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("Uid:"))?;
    line.split_whitespace().nth(2)?.parse().ok()
}

pub fn is_root() -> bool {
    match effective_uid() {
        Some(uid) => uid == 0,
        None => std::env::var("UID").map(|u| u == "0").unwrap_or(false),
    }
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

/// This function creates the config file if needed, and returns toml
///
/// The toml can be empty, or populated depending on the state of the file
pub fn create_config_from_toml(
    truncate: bool,
) -> Result<(RepositoriesConfigurationFile, File), ZlorbError> {
    let config_path_dir = get_zlorb_config_dir();
    let config_file_path = config_path_dir.join("repositories.toml");
    fs::create_dir_all(&config_path_dir)?;
    let file_contents = fs::read_to_string(&config_file_path).unwrap_or_default();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(truncate)
        .open(config_file_path)?;

    let config_loaded = toml::from_str::<RepositoriesConfigurationFile>(&file_contents).unwrap_or(
        RepositoriesConfigurationFile {
            repositories: vec![],
        },
    );

    Ok((config_loaded, file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn explicit_config_dir_wins_over_xdg_and_home() {
        let dir = resolve_config_dir_from(
            Some("/srv/zlorb"),
            Some("/home/u/.config"),
            Path::new("/home/u"),
        );
        assert_eq!(dir, PathBuf::from("/srv/zlorb"));
    }

    #[test]
    fn xdg_config_home_is_used_when_no_explicit_dir() {
        let dir = resolve_config_dir_from(None, Some("/xdg"), Path::new("/home/u"));
        assert_eq!(dir, PathBuf::from("/xdg/zlorb"));
    }

    #[test]
    fn home_config_is_the_fallback() {
        let dir = resolve_config_dir_from(None, None, Path::new("/home/u"));
        assert_eq!(dir, PathBuf::from("/home/u/.config/zlorb"));
    }

    #[test]
    fn relative_explicit_dir_is_ignored() {
        let dir = resolve_config_dir_from(
            Some("relative/zlorb"),
            Some("/xdg"),
            Path::new("/home/u"),
        );
        assert_eq!(dir, PathBuf::from("/xdg/zlorb"));
    }

    #[test]
    fn relative_xdg_is_ignored() {
        let dir = resolve_config_dir_from(None, Some("relative"), Path::new("/home/u"));
        assert_eq!(dir, PathBuf::from("/home/u/.config/zlorb"));
    }

    #[test]
    fn blank_explicit_dir_falls_through_to_xdg() {
        let dir = resolve_config_dir_from(Some("  "), Some("/xdg"), Path::new("/home/u"));
        assert_eq!(dir, PathBuf::from("/xdg/zlorb"));
    }
}
