use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use zlorb_lib::{config::RepositoryConfiguration, error::ZlorbError, log::Logger};

use crate::build_system_executor::managers::manager::Manager;

pub struct RustManager;

impl RustManager {
    fn build(&self, repo_path: &Path) -> Result<(), ZlorbError> {
        run_cargo_build(Path::new("cargo"), repo_path)
    }
}

impl Manager for RustManager {
    fn exec(&self, config: &RepositoryConfiguration) -> Result<(), ZlorbError> {
        self.build(&PathBuf::from(&config.path))
    }
}

fn run_cargo_build(cargo: &Path, repo_path: &Path) -> Result<(), ZlorbError> {
    let output = Command::new(cargo)
        .args(["build"])
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| {
            let err = format!("Failed to build with {}: {e}", cargo.display());
            Logger::error(err);
            ZlorbError::Io(e)
        })?;
    if !output.status.success() {
        let err = format!(
            "Failed to build: {} exited with status {}: {}",
            cargo.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        Logger::error(err.clone());
        return Err(ZlorbError::Other(err));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "zlorb-rust-manager-test-{}-{}-{}",
            std::process::id(),
            label,
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn build_succeeds_for_successful_command() {
        let dir = temp_dir("ok");
        assert!(run_cargo_build(Path::new("/bin/true"), &dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_errors_for_failing_command() {
        let dir = temp_dir("fail");
        let err = run_cargo_build(Path::new("/bin/false"), &dir).unwrap_err();
        assert!(format!("{err:?}").contains("Failed to build"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn build_error_carries_command_stderr() {
        let dir = temp_dir("stderr");
        let err = run_cargo_build(Path::new("/bin/sh"), &dir).unwrap_err();
        assert!(format!("{err:?}").contains("Failed to build"));
        assert!(format!("{err:?}").contains("cannot open build"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_errors_instead_of_panicking_for_missing_cargo() {
        let dir = temp_dir("missing");
        let missing = dir.join("no-such-cargo");
        assert!(run_cargo_build(&missing, &dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
