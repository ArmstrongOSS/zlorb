use crate::build_system_executor::managers::manager::Manager;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use zlorb_lib::{config::RepositoryConfiguration, error::ZlorbError, log::Logger};

pub struct BunManager;
impl BunManager {
    fn check_bun_installed(&self) -> Result<PathBuf, ZlorbError> {
        let user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
        let user_prefix: PathBuf;
        if user == "root" {
            user_prefix = PathBuf::from("/root/");
        } else {
            user_prefix = PathBuf::from("/home").join(user.clone());
        }
        let bun_path = user_prefix.join(".bun/bin/bun");
        ensure_bun_at(&bun_path, &user)?;
        Ok(bun_path)
    }

    fn install_dependencies(&self, bun_path: &Path, repo_path: &Path) -> Result<(), ZlorbError> {
        run_bun_step(bun_path, &["install"], repo_path, "install dependencies")
    }

    fn build(&self, bun_path: &Path, repo_path: &Path) -> Result<(), ZlorbError> {
        run_bun_step(bun_path, &["run", "build"], repo_path, "build")
    }

    pub(crate) fn exec_with_runtime_path(
        &self,
        config: &RepositoryConfiguration,
        bun_path: &Path,
    ) -> Result<(), ZlorbError> {
        let repo_path = Path::new(&config.path);
        self.install_dependencies(bun_path, repo_path)?;
        self.build(bun_path, repo_path)
    }
}

impl Manager for BunManager {
    fn exec(&self, config: &RepositoryConfiguration) -> Result<(), ZlorbError> {
        let bun_path = self.check_bun_installed()?;
        self.exec_with_runtime_path(config, &bun_path)
    }
}

fn ensure_bun_at(bun_path: &Path, user: &str) -> Result<(), ZlorbError> {
    let bun_exists = bun_path.try_exists().map_err(ZlorbError::Io)?;
    if !bun_exists {
        let err = format!("Bun is not installed or not in the PATH for user: {}", user);
        Logger::error(err.clone());
        return Err(ZlorbError::Other(err));
    }
    Ok(())
}

fn run_bun_step(
    bun_path: &Path,
    args: &[&str],
    repo_path: &Path,
    step: &str,
) -> Result<(), ZlorbError> {
    let output = Command::new(bun_path)
        .args(args)
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| {
            let err = format!("Failed to {step} with {}: {e}", bun_path.display());
            Logger::error(err);
            ZlorbError::Io(e)
        })?;
    if !output.status.success() {
        let err = format!(
            "Failed to {step}: {} exited with status {}: {}",
            bun_path.display(),
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
            "zlorb-bun-manager-test-{}-{}-{}",
            std::process::id(),
            label,
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn install_succeeds_for_successful_command() {
        let dir = temp_dir("install-ok");
        let manager = BunManager;
        assert!(manager
            .install_dependencies(Path::new("/bin/true"), &dir)
            .is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_errors_for_failing_command() {
        let dir = temp_dir("install-fail");
        let manager = BunManager;
        let err = manager
            .install_dependencies(Path::new("/bin/false"), &dir)
            .unwrap_err();
        assert!(format!("{err:?}").contains("Failed to install dependencies"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_succeeds_for_successful_command() {
        let dir = temp_dir("build-ok");
        let manager = BunManager;
        assert!(manager.build(Path::new("/bin/true"), &dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_errors_for_failing_command() {
        let dir = temp_dir("build-fail");
        let manager = BunManager;
        let err = manager.build(Path::new("/bin/false"), &dir).unwrap_err();
        assert!(format!("{err:?}").contains("Failed to build"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn step_succeeds_when_command_writes_to_stderr_but_exits_zero() {
        let dir = temp_dir("stderr-ok");
        assert!(
            run_bun_step(
                Path::new("/bin/sh"),
                &["-c", "echo install-warning >&2"],
                &dir,
                "install dependencies",
            )
            .is_ok()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn step_error_carries_the_failing_steps_stderr() {
        let dir = temp_dir("stderr-fail");
        let err = run_bun_step(
            Path::new("/bin/sh"),
            &["-c", "echo install-boom >&2; exit 3"],
            &dir,
            "install dependencies",
        )
        .unwrap_err();
        let debug = format!("{err:?}");
        assert!(debug.contains("Failed to install dependencies"));
        assert!(debug.contains("install-boom"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discovery_errors_for_missing_bun_at_the_checked_path() {
        let dir = temp_dir("ensure-missing");
        let err = ensure_bun_at(&dir.join("bun"), "demo").unwrap_err();
        assert!(format!("{err:?}").contains("Bun is not installed"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discovery_accepts_bun_present_at_the_checked_path() {
        let dir = temp_dir("ensure-present");
        let present = dir.join("bun");
        std::fs::write(&present, "").unwrap();
        assert!(ensure_bun_at(&present, "demo").is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn step_errors_instead_of_panicking_for_missing_bun() {
        let dir = temp_dir("missing");
        let missing = dir.join("no-such-bun");
        assert!(run_bun_step(&missing, &["install"], &dir, "install dependencies").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
