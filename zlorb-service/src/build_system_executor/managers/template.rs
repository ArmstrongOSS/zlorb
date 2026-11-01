use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use zlorb_lib::{config::RepositoryConfiguration, error::ZlorbError, log::Logger};

use crate::build_system_executor::managers::manager::Manager;

pub struct TemplateManager;

impl TemplateManager {
    fn find_runtime(&self, search_path: Option<OsString>) -> Result<PathBuf, ZlorbError> {
        find_in_path("template-runtime", search_path)
    }

    fn install_dependencies(&self, runtime: &Path, repo_path: &Path) -> Result<(), ZlorbError> {
        run_step(runtime, &["install"], repo_path, "install dependencies")
    }

    fn build(&self, runtime: &Path, repo_path: &Path) -> Result<(), ZlorbError> {
        run_step(runtime, &["run", "build"], repo_path, "build")
    }

    pub(crate) fn exec_with_search_path(
        &self,
        config: &RepositoryConfiguration,
        search_path: Option<OsString>,
    ) -> Result<(), ZlorbError> {
        let runtime = self.find_runtime(search_path)?;
        let repo_path = Path::new(&config.path);
        self.install_dependencies(&runtime, repo_path)?;
        self.build(&runtime, repo_path)
    }
}

impl Manager for TemplateManager {
    fn exec(&self, config: &RepositoryConfiguration) -> Result<(), ZlorbError> {
        self.exec_with_search_path(config, std::env::var_os("PATH"))
    }
}

fn find_in_path(name: &str, search_path: Option<OsString>) -> Result<PathBuf, ZlorbError> {
    let paths = search_path.ok_or_else(|| {
        let err = format!("Cannot discover runtime '{name}': PATH is not set");
        Logger::error(err.clone());
        ZlorbError::Other(err)
    })?;
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(name);
        if candidate.is_file() && is_executable(&candidate) {
            return Ok(candidate);
        }
    }
    let err = format!("Runtime '{name}' is not installed or not on PATH");
    Logger::error(err.clone());
    Err(ZlorbError::Other(err))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(metadata) => metadata.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

fn run_step(
    runtime: &Path,
    args: &[&str],
    repo_path: &Path,
    step: &str,
) -> Result<(), ZlorbError> {
    let output = Command::new(runtime)
        .args(args)
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| {
            let err = format!("Failed to {step} with {}: {e}", runtime.display());
            Logger::error(err);
            ZlorbError::Io(e)
        })?;
    if !output.status.success() {
        let err = format!(
            "Failed to {step}: {} exited with status {}: {}",
            runtime.display(),
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

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zlorb-template-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn find_in_path_errors_when_path_is_unset() {
        assert!(find_in_path("template-runtime", None).is_err());
    }

    #[test]
    fn find_in_path_errors_when_runtime_is_missing() {
        let dir = temp_dir("missing");
        let result = find_in_path("template-runtime", Some(dir.into_os_string()));
        assert!(result.is_err());
    }

    #[test]
    fn find_in_path_finds_runtime_on_path() {
        let dir = temp_dir("found");
        let runtime = dir.join("template-runtime");
        std::fs::write(&runtime, "").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let found = find_in_path("template-runtime", Some(dir.into_os_string())).unwrap();
        assert_eq!(found, runtime);
    }

    #[test]
    fn find_in_path_skips_non_executable_files() {
        let dir = temp_dir("not-executable");
        std::fs::write(dir.join("template-runtime"), "").unwrap();
        let result = find_in_path("template-runtime", Some(dir.into_os_string()));
        assert!(result.is_err());
    }

    #[test]
    fn manager_discovers_runtime_and_runs_steps() {
        let dir = temp_dir("manager");
        let manager = TemplateManager;
        assert!(
            manager
                .find_runtime(Some(dir.clone().into_os_string()))
                .is_err()
        );
        let runtime = dir.join("template-runtime");
        std::fs::write(&runtime, "").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(
            manager
                .find_runtime(Some(dir.clone().into_os_string()))
                .unwrap(),
            runtime
        );
        assert!(manager
            .install_dependencies(Path::new("/bin/true"), &dir)
            .is_ok());
        assert!(manager.build(Path::new("/bin/true"), &dir).is_ok());
        assert!(manager.build(Path::new("/bin/false"), &dir).is_err());
    }

    #[test]
    fn run_step_succeeds_for_successful_command() {
        let dir = temp_dir("step-ok");
        assert!(run_step(Path::new("/bin/true"), &[], &dir, "build").is_ok());
    }

    #[test]
    fn run_step_errors_with_step_name_for_failing_command() {
        let dir = temp_dir("step-fail");
        let err = run_step(Path::new("/bin/false"), &[], &dir, "build").unwrap_err();
        assert!(format!("{err:?}").contains("Failed to build"));
    }

    #[test]
    fn run_step_errors_instead_of_panicking_for_missing_runtime() {
        let dir = temp_dir("step-missing");
        let missing = dir.join("no-such-runtime");
        assert!(run_step(&missing, &[], &dir, "build").is_err());
    }

    fn write_runtime(bin_dir: &Path, repo_dir: &Path, install_body: &str, run_body: &str) {
        std::os::unix::fs::symlink("/bin/sh", bin_dir.join("template-runtime")).unwrap();
        std::fs::write(repo_dir.join("install"), install_body).unwrap();
        std::fs::write(repo_dir.join("run"), run_body).unwrap();
    }

    fn repo_config(repo_dir: &Path) -> RepositoryConfiguration {
        RepositoryConfiguration {
            name: "demo".to_string(),
            path: repo_dir.to_string_lossy().into_owned(),
            remote: "origin".to_string(),
            branch: "master".to_string(),
            build_strategy: None,
            build_command: "".to_string(),
            restart_command: None,
            out_dir: None,
        }
    }

    #[test]
    fn exec_runs_install_then_build_in_the_repo_dir() {
        let bin_dir = temp_dir("exec-bin");
        let repo_dir = temp_dir("exec-repo");
        write_runtime(
            &bin_dir,
            &repo_dir,
            "echo \"call:install\" >> invocations.log\npwd >> invocations.log\n",
            "echo \"call:run $1\" >> invocations.log\npwd >> invocations.log\n",
        );
        let manager = TemplateManager;
        manager
            .exec_with_search_path(
                &repo_config(&repo_dir),
                Some(bin_dir.into_os_string()),
            )
            .unwrap();
        let log = std::fs::read_to_string(repo_dir.join("invocations.log")).unwrap();
        let repo_canon = repo_dir.canonicalize().unwrap();
        let repo_canon = repo_canon.to_string_lossy();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(
            lines,
            vec![
                "call:install",
                repo_canon.as_ref(),
                "call:run build",
                repo_canon.as_ref()
            ]
        );
    }

    #[test]
    fn exec_error_carries_the_failing_steps_stderr() {
        let bin_dir = temp_dir("exec-fail-bin");
        let repo_dir = temp_dir("exec-fail-repo");
        write_runtime(
            &bin_dir,
            &repo_dir,
            "",
            "echo build-boom >&2\nexit 3\n",
        );
        let manager = TemplateManager;
        let err = manager
            .exec_with_search_path(
                &repo_config(&repo_dir),
                Some(bin_dir.into_os_string()),
            )
            .unwrap_err();
        let debug = format!("{err:?}");
        assert!(debug.contains("Failed to build"));
        assert!(debug.contains("build-boom"));
    }
}
