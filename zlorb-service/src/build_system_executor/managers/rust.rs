use std::process::{Output, Stdio};

use zlorb_lib::error::ZlorbError;

use crate::build_system_executor::managers::manager::Manager;

pub struct RustManager;
impl RustManager {
    fn build(&self, repo_path: String) -> Result<(), ZlorbError> {
        let handle = std::thread::spawn(move || -> Result<Output, ZlorbError> {
            let p = std::process::Command::new("cargo")
                .args(["build"])
                .current_dir(repo_path)
                .stdout(Stdio::piped())
                .output()
                .map_err(ZlorbError::Io)?;
            Ok(p)
        });
        handle
            .join()
            .map_err(|_| ZlorbError::Other("Failure when joining cargo build handle".into()))??;
        Ok(())
    }
}

impl Manager for RustManager {
    fn exec(
        &self,
        config: &zlorb_lib::config::RepositoryConfiguration,
    ) -> Result<(), zlorb_lib::error::ZlorbError> {
        self.build(config.path.clone())
    }
}
