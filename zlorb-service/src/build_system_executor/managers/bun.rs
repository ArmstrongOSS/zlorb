use crate::build_system_executor::managers::manager::Manager;
use std::{path::PathBuf, process::Stdio};
use zlorb_lib::{config::RepositoryConfiguration, error::ZlorbError, log::Logger};

pub struct BunManager;
impl BunManager {
    fn check_bun_installed(&self) -> Result<PathBuf, ZlorbError> {
        let user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
        let bun_exists: bool;
        let user_prefix: PathBuf;
        if user == "root" {
            user_prefix = PathBuf::from("/root/");
        } else {
            user_prefix = PathBuf::from("/home").join(user.clone());
        }
        let bun_path = user_prefix.join(".bun/bin/bun");
        bun_exists = bun_path.try_exists().map_err(ZlorbError::Io)?;

        if !bun_exists {
            let err = format!("Bun is not installed or not in the PATH for user: {}", user);
            Logger::error(err.clone());
            return Err(ZlorbError::Other(err));
        }

        Ok(bun_path)
    }

    fn pull_node_modules(&self, repo_path: PathBuf, bun_path: PathBuf) -> Result<(), ZlorbError> {
        let p = repo_path
            .canonicalize()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| {
                Logger::error(format!("Failed to canonicalize path: {:?}", repo_path));
                repo_path.to_string_lossy().into_owned()
            });

        if !std::path::Path::new(&p).exists() {
            let err = format!("Directory does not exist: {}", p);
            Logger::error(err.clone());
            return Err(ZlorbError::Other(err));
        }

        let package_install_handle = std::process::Command::new(&bun_path)
            .args(["install"])
            .current_dir(p)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| {
                let err = format!("Failed to execute bun install at {:?}: {}", bun_path, e);
                Logger::error(err.clone());
                ZlorbError::Io(e)
            })?;

        if !package_install_handle.stderr.is_empty() {
            let err = format!(
                "Npm install failed: {}",
                String::from_utf8_lossy(&package_install_handle.stderr)
            );
            Logger::error(err.clone());
            return Err(ZlorbError::Other(err));
        }
        Ok(())
    }

    fn build(&self, config: &RepositoryConfiguration, bun_path: PathBuf) -> Result<(), ZlorbError> {
        let path = config.path.clone();

        let package_install_handle = std::process::Command::new(&bun_path)
            .args(["run", "build"])
            .current_dir(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| {
                let err = format!("Failed to execute bun build at {:?}: {}", bun_path, e);
                Logger::error(err.clone());
                ZlorbError::Io(e)
            })?;

        if package_install_handle.status.code() == Some(1)
            || (!package_install_handle.status.success()
                && package_install_handle.status.code() != Some(1))
        {
            let err = format!(
                "Bun build returned status {} resulting in failure: {}",
                package_install_handle.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&package_install_handle.stderr)
            );
            Logger::error(err.clone());
            return Err(ZlorbError::Other(err));
        }
        Ok(())
    }
}

impl Manager for BunManager {
    fn exec(&self, config: &RepositoryConfiguration) -> Result<(), ZlorbError> {
        let bun_path = self.check_bun_installed()?;
        self.pull_node_modules(PathBuf::from(&config.path), bun_path.clone())?;
        self.build(config, bun_path)
    }
}
