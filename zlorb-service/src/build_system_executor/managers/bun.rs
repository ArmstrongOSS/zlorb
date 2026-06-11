use crate::build_system_executor::managers::manager::Manager;
use std::{
    path::PathBuf,
    process::{Output, Stdio},
};
use zlorb_lib::{config::RepositoryConfiguration, error::ZlorbError, log::Logger};

pub struct BunManager;
impl BunManager {
    fn pull_node_modules(&self, repo_path: PathBuf) -> Result<(), ZlorbError> {
        Logger::info("Bun build: Pullig Node Modules".into());
        let p = repo_path.to_string_lossy().into_owned();
        let handle = std::thread::spawn(move || -> Result<Output, ZlorbError> {
            let package_install_handle = std::process::Command::new("bun")
                .arg("install")
                .current_dir(p)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .map_err(ZlorbError::Io)?;
            if !(package_install_handle.stderr.is_empty()) {
                let err = format!(
                    "Npm install failed: {}",
                    String::from_utf8_lossy(&package_install_handle.stderr)
                );
                return Err(ZlorbError::Other(err));
            }
            Ok(package_install_handle)
        });

        let _h = handle.join().map_err(|_| {
            ZlorbError::Other("Failed to successfully run install command".into())
        })??;
        Logger::info(format!("{}", String::from_utf8_lossy(&_h.stdout)));
        Ok(())
    }

    fn build(&self, config: &RepositoryConfiguration) -> Result<(), ZlorbError> {
        Logger::info("Bun build: running build".into());
        let path = config.path.clone();

        let handle = std::thread::spawn(move || -> Result<Output, ZlorbError> {
            let build_handle = std::process::Command::new("bun")
                .args(["run", "build"])
                .current_dir(path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .map_err(ZlorbError::Io)?;
            if build_handle.status.code() == Some(1) {
                return Err(ZlorbError::Other(
                    "Bun build returned status code 1 resulting in failure".to_string(),
                ));
            }
            Ok(build_handle)
        });

        let _h = handle
            .join()
            .map_err(|_| ZlorbError::Other("Faild to join the thread".into()))??;

        Logger::info(format!("{}", String::from_utf8_lossy(&_h.stdout)));
        Ok(())
    }
}

impl Manager for BunManager {
    fn exec(&self, config: &RepositoryConfiguration) -> Result<(), ZlorbError> {
        self.pull_node_modules(PathBuf::from(&config.path))?;
        self.build(config)
    }
}
