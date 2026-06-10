pub struct BunManager {}
impl Manager for BunManager {
    pub fn build(&self) -> Result<(), ZlorbError> {
        let path = self.processor.config.path.clone();
        let build_command = self.processor.config.build_command.clone();

        let handle = std::thread::spawn(move || -> Result<(), ZlorbError> {
            std::env::set_current_dir(path).map_err(ZlorbError::Io)?;
            let build_handle = std::process::Command::new(build_command)
                .stdout(Stdio::piped())
                .output()
                .map_err(ZlorbError::Io)?;
            if build_handle.status.code() == Some(1) {
                return Err(ZlorbError::Other(
                    "build returned status code 1 resulting in failure".to_string(),
                ));
            }
            Ok(())
        });

        let _h = handle
            .join()
            .map_err(|_| ZlorbError::Other("Faild to join the thread".into()))?;

        Ok(())
    }
}

impl BunManager {
    fn pull_node_modules(&self) -> Result<(), ZlorbError> {
        let p = self.processor.repo_path.to_string_lossy().into_owned();
        let handle = std::thread::spawn(move || -> Result<Output, ZlorbError> {
            let package_install_handle = std::process::Command::new("bun")
                .arg("install")
                .current_dir(p)
                .stdout(Stdio::piped())
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

        let _h = handle
            .join()
            .map_err(|_| ZlorbError::Other("Failed to successfully run install command".into()))?;
        Ok(())
    }

    fn build() {}
}
