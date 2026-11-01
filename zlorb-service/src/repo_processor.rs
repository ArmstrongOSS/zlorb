use git2::{
    AnnotatedCommit, Branch, BranchType, Cred, FetchOptions, MergeAnalysis, MergePreference, Oid,
    Reference, Remote, RemoteCallbacks,
};
use std::{
    collections::HashMap,
    fmt::Debug,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};
use zlorb_lib::{config::RepositoryConfiguration, error::ZlorbError, log::Logger};

use crate::build_system_executor::BuildSystemExecutor;

pub type FailedBuilds = Arc<Mutex<HashMap<String, Oid>>>;

pub fn new_failed_builds() -> FailedBuilds {
    Arc::new(Mutex::new(HashMap::new()))
}

pub type FailedRestarts = Arc<Mutex<HashMap<String, Oid>>>;

pub fn new_failed_restarts() -> FailedRestarts {
    Arc::new(Mutex::new(HashMap::new()))
}

pub fn tracker_key(raw_path: &str) -> String {
    match std::fs::canonicalize(raw_path) {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(_) => normalize_path_lexically(raw_path),
    }
}

fn normalize_path_lexically(raw_path: &str) -> String {
    let path = Path::new(raw_path);
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !normalized.pop() && !path.is_absolute() {
                    normalized.push(component);
                }
            }
            other => normalized.push(other),
        }
    }
    if normalized.as_os_str().is_empty() {
        return ".".to_string();
    }
    normalized.to_string_lossy().into_owned()
}

fn tracker_failed_at(
    tracker: &FailedBuilds,
    key: &str,
    oid: Oid,
    label: &str,
    repo_name: &str,
) -> bool {
    match tracker.lock() {
        Ok(failed) => failed.get(key) == Some(&oid),
        Err(e) => {
            Logger::error(format!(
                "The {} tracker lock is poisoned for {}; treating the commit as not failed and skipping retries until the daemon restarts: {}",
                label, repo_name, e
            ));
            false
        }
    }
}

fn tracker_record(tracker: &FailedBuilds, key: &str, oid: Oid, label: &str, repo_name: &str) {
    match tracker.lock() {
        Ok(mut failed) => {
            failed.insert(key.to_string(), oid);
        }
        Err(e) => Logger::error(format!(
            "The {} tracker lock is poisoned for {}; the failure at {} was not recorded and will not be retried: {}",
            label, repo_name, oid, e
        )),
    }
}

fn tracker_clear(tracker: &FailedBuilds, key: &str, label: &str, repo_name: &str) {
    match tracker.lock() {
        Ok(mut failed) => {
            failed.remove(key);
        }
        Err(e) => Logger::error(format!(
            "The {} tracker lock is poisoned for {}; the stored failure was not cleared: {}",
            label, repo_name, e
        )),
    }
}

pub struct RepoProcessor {
    pub(crate) repo_path: PathBuf,
    pub repo: git2::Repository,
    pub config: RepositoryConfiguration,
    pub failed_builds: FailedBuilds,
    pub failed_restarts: FailedRestarts,
}

impl Debug for RepoProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepoProcessor")
            .field("repo_path", &self.repo_path)
            .field("repo", &"Repository")
            .field("config", &self.config)
            .finish()
    }
}

fn run_restart_command(
    command: &str,
    repo_path: &Path,
    repo_name: &str,
) -> Result<(), ZlorbError> {
    Logger::info(format!(
        "Running restart command for {}: {}",
        repo_name, command
    ));
    let capture_path = std::env::temp_dir().join(format!(
        "zlorb-restart-output-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    let capture = std::fs::File::create(&capture_path).map_err(|e| {
        let err = format!(
            "Failed to run restart command for {}: could not create the output capture file: {}",
            repo_name, e
        );
        Logger::error(err.clone());
        ZlorbError::Other(err)
    })?;
    let capture_err = capture.try_clone().map_err(|e| {
        let err = format!(
            "Failed to run restart command for {}: could not clone the output capture file: {}",
            repo_name, e
        );
        Logger::error(err.clone());
        ZlorbError::Other(err)
    })?;
    let mut shell = shell_command(command);
    let mut child = shell
        .current_dir(repo_path)
        .stdout(Stdio::from(capture))
        .stderr(Stdio::from(capture_err))
        .spawn()
        .map_err(|e| {
            let err = format!(
                "Failed to run restart command for {}: {}",
                repo_name, e
            );
            Logger::error(err.clone());
            ZlorbError::Other(err)
        })?;
    let status = child.wait().map_err(|e| {
        let err = format!(
            "Failed to run restart command for {}: {}",
            repo_name, e
        );
        Logger::error(err.clone());
        ZlorbError::Other(err)
    })?;
    let output = std::fs::read_to_string(&capture_path).unwrap_or_default();
    let _ = std::fs::remove_file(&capture_path);
    if !output.trim().is_empty() {
        Logger::info(format!(
            "Restart command for {} output: {}",
            repo_name,
            output.trim()
        ));
    }
    if !status.success() {
        let err = format!(
            "Restart command for {} exited with status {}: {}",
            repo_name,
            status,
            output.trim()
        );
        Logger::error(err.clone());
        return Err(ZlorbError::Other(err));
    }
    Ok(())
}

#[cfg(windows)]
fn shell_command(command: &str) -> Command {
    let mut shell = Command::new("cmd");
    shell.args(["/C", command]);
    shell
}

#[cfg(not(windows))]
fn shell_command(command: &str) -> Command {
    let mut shell = Command::new("/bin/sh");
    shell.args(["-c", command]);
    shell
}

impl RepoProcessor {
    pub fn new(config: RepositoryConfiguration) -> Result<Self, ZlorbError> {
        Logger::info(format!("Initializing RepoProcessor for {}", config.name));
        let repo = git2::Repository::open(&config.path).map_err(|e| {
            ZlorbError::Other(format!(
                "Failed to acquire repo {} at {}: {}",
                config.name, config.path, e
            ))
        })?;
        Ok(Self {
            repo_path: PathBuf::from(&config.path),
            repo,
            config,
            failed_builds: new_failed_builds(),
            failed_restarts: new_failed_restarts(),
        })
    }

    pub fn update_from_remote(&self) -> Result<(), ZlorbError> {
        Logger::info(format!("Updating from remote for {}", self.config.name));
        self.config.resolved_build_strategy()?;
        let head_before = self.head_oid("before fetch");
        let has_updates = self.fetch_remote_updates()?;
        Logger::info(format!("Remote updates found: {}", has_updates));

        let out_dir_missing = self.config.out_dir.is_some() && !self.check_if_out_dir_exists()?;
        Logger::info(format!("Out directory missing (expected): {}", out_dir_missing));

        let head_oid = self.head_oid("after fetch");
        let retrying_failed_build = head_oid
            .map(|oid| self.build_failed_at(oid))
            .unwrap_or(false);
        if retrying_failed_build {
            Logger::error(format!(
                "Retrying the failed build for {} at {}: the previous build for this commit did not succeed",
                self.config.name,
                head_oid.map(|oid| oid.to_string()).unwrap_or_default()
            ));
        }

        let should_run_build = has_updates || out_dir_missing || retrying_failed_build;
        Logger::info(format!("Should run build: {}", should_run_build));

        if should_run_build && !has_updates {
            let tracked_changes = self._tracked_local_changes()?;
            if !tracked_changes.is_empty() {
                return Err(ZlorbError::Other(format!(
                    "Refusing to build {}: local changes to tracked files ({}) are not part of the commit being built. Stash the work, move it to another branch, or discard it — do not commit it on this branch, the daemon can only fast-forward this branch",
                    self.config.name,
                    tracked_changes.join(", ")
                )));
            }
        }

        let build_outcome = match should_run_build {
            true => {
                Logger::info(format!(
                    "Found updates or missing output dir for {}. Pulling changes...",
                    self.config.name
                ));
                let exec = BuildSystemExecutor { processor: self };
                match exec.run_build() {
                    Ok(()) => {
                        self.clear_failed_build();
                        Ok(())
                    }
                    Err(e) => {
                        match self.head_oid("after a failed build") {
                            Some(oid) if Some(oid) == head_oid => {
                                self.record_failed_build(oid);
                            }
                            Some(oid) => Logger::error(format!(
                                "Not recording the failed build for {}: HEAD moved to {} during the build, so the failure cannot be attributed to the commit that was current before it",
                                self.config.name, oid
                            )),
                            None => Logger::error(format!(
                                "Not recording the failed build for {}: HEAD could not be read after the build failed, so the failure will not be retried",
                                self.config.name
                            )),
                        }
                        Err(e)
                    }
                }
            }
            false => {
                Logger::info(format!("No build needed for {}", self.config.name));
                Ok(())
            }
        };
        build_outcome.and_then(|_| {
            self.run_restart_if_needed(head_before, retrying_failed_build, should_run_build)
        })
    }

    fn run_restart_if_needed(
        &self,
        head_before: Option<Oid>,
        retrying_failed_build: bool,
        build_ran_this_cycle: bool,
    ) -> Result<(), ZlorbError> {
        let Some(command) = self.config.resolved_restart_command() else {
            return Ok(());
        };
        let current_oid = self.head_oid("before the restart check");
        let head_changed = current_oid.is_some() && current_oid != head_before;
        let retrying_failed_restart = current_oid
            .map(|oid| self.restart_failed_at(oid))
            .unwrap_or(false);
        if retrying_failed_restart && !head_changed {
            Logger::error(format!(
                "Retrying the failed restart command for {} at {}: the previous restart for this commit did not succeed",
                self.config.name,
                current_oid.map(|oid| oid.to_string()).unwrap_or_default()
            ));
        }
        let needed = head_changed || retrying_failed_restart || retrying_failed_build;
        if !needed {
            return Ok(());
        }
        if !build_ran_this_cycle {
            let tracked_changes = self._tracked_local_changes()?;
            if !tracked_changes.is_empty() {
                return Err(ZlorbError::Other(format!(
                    "Refusing to run the restart command for {}: local changes to tracked files ({}) are not part of the commit being restarted on. Stash the work, move it to another branch, or discard it — do not commit it on this branch, the daemon can only fast-forward this branch",
                    self.config.name,
                    tracked_changes.join(", ")
                )));
            }
        }
        match run_restart_command(command, &self.repo_path, &self.config.name) {
            Ok(()) => {
                self.clear_failed_restart();
                Ok(())
            }
            Err(e) => {
                match current_oid {
                    Some(oid) => self.record_failed_restart(oid),
                    None => Logger::error(format!(
                        "Not recording the failed restart for {}: HEAD could not be read after the restart failed, so the failure will not be retried",
                        self.config.name
                    )),
                }
                Err(e)
            }
        }
    }

    fn head_oid(&self, context: &str) -> Option<Oid> {
        match self.repo.head() {
            Ok(head) => match head.target() {
                Some(oid) => Some(oid),
                None => {
                    Logger::error(format!(
                        "Could not read HEAD for {} ({}): HEAD has no target, so failed-build and failed-restart tracking is skipped for this read",
                        self.config.name, context
                    ));
                    None
                }
            },
            Err(e) => {
                Logger::error(format!(
                    "Could not read HEAD for {} ({}): {}, so failed-build and failed-restart tracking is skipped for this read",
                    self.config.name, context, e
                ));
                None
            }
        }
    }

    fn restart_failed_at(&self, oid: Oid) -> bool {
        tracker_failed_at(
            &self.failed_restarts,
            &tracker_key(&self.config.path),
            oid,
            "failed-restart",
            &self.config.name,
        )
    }

    fn record_failed_restart(&self, oid: Oid) {
        tracker_record(
            &self.failed_restarts,
            &tracker_key(&self.config.path),
            oid,
            "failed-restart",
            &self.config.name,
        );
    }

    fn clear_failed_restart(&self) {
        tracker_clear(
            &self.failed_restarts,
            &tracker_key(&self.config.path),
            "failed-restart",
            &self.config.name,
        );
    }

    fn build_failed_at(&self, oid: Oid) -> bool {
        tracker_failed_at(
            &self.failed_builds,
            &tracker_key(&self.config.path),
            oid,
            "failed-build",
            &self.config.name,
        )
    }

    fn record_failed_build(&self, oid: Oid) {
        tracker_record(
            &self.failed_builds,
            &tracker_key(&self.config.path),
            oid,
            "failed-build",
            &self.config.name,
        );
    }

    fn clear_failed_build(&self) {
        tracker_clear(
            &self.failed_builds,
            &tracker_key(&self.config.path),
            "failed-build",
            &self.config.name,
        );
    }

    fn check_if_out_dir_exists(&self) -> Result<bool, ZlorbError> {
        Logger::info(format!("Checking out dir existence for {}", self.config.name));
        match &self.config.out_dir {
            Some(path_str) => {
                let path = PathBuf::from(path_str);
                let full_path = if path.is_absolute() {
                    path
                } else {
                    self.repo_path.join(path)
                };
                let exists = full_path.exists();
                Logger::info(format!("Out dir {} exists: {}", full_path.display(), exists));
                Ok(exists)
            }
            None => {
                Logger::info("No out_dir specified in config.".to_string());
                Ok(true)
            }
        }
    }

    fn fetch_remote_updates(&self) -> Result<bool, ZlorbError> {
        let local_branch = self._get_local_branch()?;
        let local_oid = self._get_local_oid_from_branch(local_branch)?;
        let mut remote = self._get_remote()?;
        self._download_new_data(&mut remote);
        let analysis = self._get_analysis()?;
        let (merge_analysis, _, fetch_commit) = analysis;
        if merge_analysis.is_up_to_date() {
            return Ok(false);
        }
        if !merge_analysis.is_fast_forward() {
            return Err(ZlorbError::Git(git2::Error::new(
                git2::ErrorCode::NotFastForward,
                git2::ErrorClass::Invalid,
                "Fast-forward only!",
            )));
        }
        self._refuse_if_update_would_overwrite(&fetch_commit)?;
        let fetch_oid = fetch_commit.id();
        let remote_ref = match self._apply_fast_forward(fetch_commit) {
            Ok(remote_ref) => remote_ref,
            Err(e) => {
                self._rollback_fast_forward(local_oid, fetch_oid);
                return Err(e);
            }
        };

        self._should_trigger_update(remote_ref, local_oid)
    }

    fn _refuse_if_update_would_overwrite(
        &self,
        fetch_commit: &AnnotatedCommit,
    ) -> Result<(), ZlorbError> {
        let tracked_changes = self._tracked_local_changes()?;
        let untracked_collisions = self._untracked_collisions(fetch_commit)?;
        if tracked_changes.is_empty() && untracked_collisions.is_empty() {
            return Ok(());
        }
        let mut reasons = Vec::new();
        if !tracked_changes.is_empty() {
            reasons.push(format!(
                "local changes to tracked files ({})",
                tracked_changes.join(", ")
            ));
        }
        if !untracked_collisions.is_empty() {
            reasons.push(format!(
                "untracked files in the way ({})",
                untracked_collisions.join(", ")
            ));
        }
        Err(ZlorbError::Other(format!(
            "Refusing to fast-forward {}: {} would be overwritten. Stash the work, move it to another branch, or discard it — do not commit it on this branch, the daemon can only fast-forward this branch",
            self.config.name,
            reasons.join(" and ")
        )))
    }

    fn _untracked_collisions(
        &self,
        fetch_commit: &AnnotatedCommit,
    ) -> Result<Vec<String>, ZlorbError> {
        let Some(workdir) = self.repo.workdir().map(|path| path.to_path_buf()) else {
            return Ok(Vec::new());
        };
        let index = self.repo.index().map_err(ZlorbError::Git)?;
        let commit = self
            .repo
            .find_commit(fetch_commit.id())
            .map_err(ZlorbError::Git)?;
        let tree = commit.tree().map_err(ZlorbError::Git)?;
        let mut collisions: Vec<String> = Vec::new();
        tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
            if matches!(
                entry.kind(),
                Some(git2::ObjectType::Blob) | Some(git2::ObjectType::Commit)
            ) {
                let path = PathBuf::from(root).join(entry.name().unwrap_or_default());
                let components: Vec<_> = path.components().collect();
                let mut prefix = PathBuf::new();
                for (position, component) in components.iter().enumerate() {
                    prefix.push(component);
                    let Ok(metadata) = std::fs::symlink_metadata(workdir.join(&prefix)) else {
                        break;
                    };
                    let is_full_path = position + 1 == components.len();
                    let blocks_descent = !is_full_path && !metadata.is_dir();
                    if (is_full_path || blocks_descent)
                        && index.get_path(&prefix, 0).is_none()
                    {
                        let label = prefix.to_string_lossy().into_owned();
                        if !collisions.contains(&label) {
                            collisions.push(label);
                        }
                        break;
                    }
                }
            }
            git2::TreeWalkResult::Ok
        })
        .map_err(ZlorbError::Git)?;
        Ok(collisions)
    }

    fn _tracked_local_changes(&self) -> Result<Vec<String>, ZlorbError> {
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(false);
        opts.include_ignored(false);
        let statuses = self
            .repo
            .statuses(Some(&mut opts))
            .map_err(ZlorbError::Git)?;
        Ok(statuses
            .iter()
            .map(|entry| String::from_utf8_lossy(entry.path_bytes()).into_owned())
            .collect())
    }

    fn _apply_fast_forward(
        &self,
        fetch_commit: AnnotatedCommit,
    ) -> Result<Reference<'_>, ZlorbError> {
        self._setup_repo_for_checkout(fetch_commit)?;
        self._checkout_and_get_ref()
    }

    fn _rollback_fast_forward(&self, local_oid: Oid, fetch_oid: Oid) {
        let refname = format!("refs/heads/{}", self.config.branch);
        if let Ok(mut reference) = self.repo.find_reference(&refname) {
            if let Err(e) = reference.set_target(local_oid, "Rolling back failed fast-forward") {
                Logger::error(format!(
                    "Failed to roll back {} to {}: {}",
                    refname, local_oid, e
                ));
            }
        }
        self._remove_partial_update_files(local_oid, fetch_oid);
        let mut checkout = git2::build::CheckoutBuilder::default();
        checkout.force();
        if let Err(e) = self.repo.checkout_head(Some(&mut checkout)) {
            Logger::error(format!(
                "Failed to restore the working tree after a failed fast-forward: {}",
                e
            ));
        }
    }

    fn _remove_partial_update_files(&self, local_oid: Oid, fetch_oid: Oid) {
        let (Ok(fetch_commit), Ok(local_commit)) = (
            self.repo.find_commit(fetch_oid),
            self.repo.find_commit(local_oid),
        ) else {
            return;
        };
        let (Ok(fetch_tree), Ok(local_tree)) = (fetch_commit.tree(), local_commit.tree())
        else {
            return;
        };
        let Some(workdir) = self.repo.workdir().map(|path| path.to_path_buf()) else {
            return;
        };
        let Ok(index) = self.repo.index() else {
            return;
        };
        let mut added: Vec<(PathBuf, Oid)> = Vec::new();
        let _ = fetch_tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
            if matches!(
                entry.kind(),
                Some(git2::ObjectType::Blob) | Some(git2::ObjectType::Commit)
            ) {
                let path = PathBuf::from(root).join(entry.name().unwrap_or_default());
                if local_tree.get_path(&path).is_err() {
                    added.push((path, entry.id()));
                }
            }
            git2::TreeWalkResult::Ok
        });
        for (path, entry_id) in added {
            let full_path = workdir.join(&path);
            if index.get_path(&path, 0).is_some() {
                continue;
            }
            let is_file = std::fs::symlink_metadata(&full_path)
                .map(|metadata| !metadata.is_dir())
                .unwrap_or(false);
            if !is_file {
                continue;
            }
            let Ok(contents) = std::fs::read(&full_path) else {
                continue;
            };
            let Ok(disk_id) = Oid::hash_object(git2::ObjectType::Blob, &contents) else {
                continue;
            };
            if disk_id != entry_id {
                continue;
            }
            if let Err(e) = std::fs::remove_file(&full_path) {
                Logger::error(format!(
                    "Failed to remove {} left by a failed fast-forward: {}",
                    full_path.display(),
                    e
                ));
            }
        }
    }

    fn _setup_fetchoptions_with_creds(&self) -> FetchOptions<'_> {
        let mut callbacks = RemoteCallbacks::new();
        //TODO: Complete the credential stuff
        callbacks.credentials(|_, _, _| Cred::userpass_plaintext("", ""));
        let mut fetch_options = FetchOptions::new();
        fetch_options.remote_callbacks(callbacks);
        fetch_options
    }

    fn _get_local_branch(&self) -> Result<Branch<'_>, ZlorbError> {
        let branch = self
            .repo
            .find_branch(&self.config.branch, BranchType::Local)
            .map_err(|e| ZlorbError::Other(e.to_string()))?;
        Ok(branch)
    }

    fn _get_local_oid_from_branch(&self, branch: Branch) -> Result<Oid, ZlorbError> {
        let id = branch.get().target();
        if id.is_none() {
            return Err(ZlorbError::Git(git2::Error::new(
                git2::ErrorCode::NotFound,
                git2::ErrorClass::None,
                "Failed to get Oid from branch",
            )));
        };
        Ok(id.unwrap())
    }

    fn _get_remote(&self) -> Result<Remote<'_>, ZlorbError> {
        let remote = self
            .repo
            .find_remote(&self.config.remote)
            .map_err(ZlorbError::Git)?;
        Ok(remote)
    }

    fn _download_new_data(&self, remote: &mut Remote) {
        let _ = remote.fetch(
            std::slice::from_ref(&self.config.branch),
            Some(&mut self._setup_fetchoptions_with_creds()),
            None,
        );
    }

    fn _get_analysis(
        &self,
    ) -> Result<(MergeAnalysis, MergePreference, AnnotatedCommit<'_>), ZlorbError> {
        let r = &self.repo;
        let fetch_head = r.find_reference("FETCH_HEAD").map_err(ZlorbError::Git)?;
        let fetch_commit = r
            .reference_to_annotated_commit(&fetch_head)
            .map_err(ZlorbError::Git)?;
        let analysis = r
            .merge_analysis(&[&fetch_commit])
            .map_err(ZlorbError::Git)?;
        let (m, n) = analysis;
        Ok((m, n, fetch_commit))
    }

    fn _setup_repo_for_checkout(&self, fetch_commit: AnnotatedCommit) -> Result<(), ZlorbError> {
        let refname = format!("refs/heads/{}", self.config.branch);
        self.repo
            .find_reference(&refname)
            .map_err(ZlorbError::Git)?
            .set_target(fetch_commit.id(), "Fast-Forward")
            .map_err(ZlorbError::Git)?;
        self.repo.set_head(&refname).map_err(ZlorbError::Git)?;
        Ok(())
    }

    fn _checkout_and_get_ref(&self) -> Result<Reference<'_>, ZlorbError> {
        let mut checkout = git2::build::CheckoutBuilder::default();
        checkout.force();
        self.repo
            .checkout_head(Some(&mut checkout))
            .map_err(ZlorbError::Git)?;

        let remote_ref = self
            .repo
            .resolve_reference_from_short_name(&format!(
                "{}/{}",
                self.config.remote, self.config.branch
            ))
            .map_err(ZlorbError::Git)?;

        Ok(remote_ref)
    }

    fn _should_trigger_update(
        &self,
        remote_ref: Reference,
        local_oid: Oid,
    ) -> Result<bool, ZlorbError> {
        let remote_iod: Oid = remote_ref.target().ok_or_else(|| {
            ZlorbError::Git(git2::Error::new(
                git2::ErrorCode::NotFound,
                git2::ErrorClass::None,
                "Remote ref has no target",
            ))
        })?;
        let dist_dir_path = PathBuf::from(&self.config.path).join("dist");
        let dist_dir_exists = std::fs::exists(dist_dir_path).map_err(ZlorbError::Io)?;
        let local_iod_matches_remote = local_oid == remote_iod;
        if !dist_dir_exists || !local_iod_matches_remote {
            return Ok(true);
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{IndexAddOption, RepositoryInitOptions, Signature};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "zlorb-repo-processor-test-{}-{}-{}",
            std::process::id(),
            label,
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_fixture_crate(repo_path: &Path, marker: &str) {
        std::fs::create_dir_all(repo_path.join("src")).unwrap();
        std::fs::write(
            repo_path.join("Cargo.toml"),
            "[package]\nname = \"fixture-app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(
            repo_path.join("src/main.rs"),
            format!("fn main() {{ println!(\"{}\"); }}\n", marker),
        )
        .unwrap();
    }

    fn commit_all(repo: &git2::Repository, message: &str) -> Oid {
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .unwrap();
        let workdir = repo.workdir().unwrap().to_path_buf();
        let tracked: Vec<String> = index
            .iter()
            .map(|entry| String::from_utf8_lossy(&entry.path).into_owned())
            .collect();
        for path in tracked {
            if !workdir.join(&path).exists() {
                index.remove_path(Path::new(&path)).unwrap();
            }
        }
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let sig = Signature::now("zlorb-test", "zlorb-test@example.com").unwrap();
        let parent = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .map(|oid| repo.find_commit(oid).unwrap());
        match parent {
            Some(parent) => repo
                .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
                .unwrap(),
            None => repo
                .commit(Some("HEAD"), &sig, &sig, message, &tree, &[])
                .unwrap(),
        }
    }

    fn head_oid(repo: &git2::Repository) -> Oid {
        repo.head().unwrap().target().unwrap()
    }

    fn fixture_artifact(watched: &Path) -> PathBuf {
        let name = if cfg!(windows) {
            "fixture-app.exe"
        } else {
            "fixture-app"
        };
        match std::env::var_os("CARGO_TARGET_DIR") {
            Some(dir) if Path::new(&dir).is_absolute() => {
                PathBuf::from(dir).join("debug").join(name)
            }
            _ => watched.join("target").join("debug").join(name),
        }
    }

    fn processor_for(watched: &Path) -> RepoProcessor {
        RepoProcessor::new(RepositoryConfiguration {
            name: "watched".to_string(),
            path: watched.to_string_lossy().into_owned(),
            remote: "origin".to_string(),
            branch: "master".to_string(),
            build_strategy: Some("rust".to_string()),
            build_command: String::new(),
            restart_command: None,
            out_dir: None,
        })
        .unwrap()
    }

    #[test]
    fn fast_forward_updates_working_tree_and_build_output() {
        let root = temp_dir("fast-forward");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        std::fs::write(upstream_path.join("old.txt"), "old file\n").unwrap();
        let v1 = commit_all(&upstream, "v1");
        drop(upstream);
        let watched = git2::Repository::clone(
            upstream_path.to_string_lossy().as_ref(),
            &watched_path,
        )
        .unwrap();
        assert_eq!(head_oid(&watched), v1);
        drop(watched);

        processor_for(&watched_path).update_from_remote().unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        assert!(!fixture_artifact(&watched_path).exists());
        drop(watched);

        let upstream = git2::Repository::open(&upstream_path).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-two");
        std::fs::remove_file(upstream_path.join("old.txt")).unwrap();
        std::fs::write(upstream_path.join("new.txt"), "new file\n").unwrap();
        let v2 = commit_all(&upstream, "v2");
        drop(upstream);

        processor_for(&watched_path).update_from_remote().unwrap();

        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        let source = std::fs::read_to_string(watched_path.join("src/main.rs")).unwrap();
        assert!(source.contains("fixture-version-two"));
        assert_eq!(
            std::fs::read_to_string(watched_path.join("new.txt")).unwrap(),
            "new file\n"
        );
        assert!(!watched_path.join("old.txt").exists());
        let statuses = watched.statuses(None).unwrap();
        assert!(
            statuses
                .iter()
                .all(|entry| entry.status() == git2::Status::WT_NEW),
            "no tracked file may be dirty after the fast-forward, got: {:?}",
            statuses
                .iter()
                .map(|entry| (entry.path().map(str::to_string), entry.status()))
                .collect::<Vec<_>>()
        );
        let artifact = fixture_artifact(&watched_path);
        assert!(artifact.exists());
        let output = std::process::Command::new(&artifact).output().unwrap();
        assert!(output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("fixture-version-two")
        );
        drop(statuses);
        drop(watched);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn up_to_date_repo_is_left_untouched() {
        let root = temp_dir("up-to-date");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        let v1 = commit_all(&upstream, "v1");
        drop(upstream);
        git2::Repository::clone(upstream_path.to_string_lossy().as_ref(), &watched_path)
            .unwrap();
        std::fs::write(watched_path.join("src/main.rs"), "local edit\n").unwrap();
        processor_for(&watched_path).update_from_remote().unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        assert_eq!(
            std::fs::read_to_string(watched_path.join("src/main.rs")).unwrap(),
            "local edit\n"
        );
        drop(watched);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dirty_repo_refuses_fast_forward_until_cleaned() {
        let root = temp_dir("dirty-refuses");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        let v1 = commit_all(&upstream, "v1");
        write_fixture_crate(&upstream_path, "fixture-version-two");
        let v2 = commit_all(&upstream, "v2");
        drop(upstream);
        git2::Repository::clone(upstream_path.to_string_lossy().as_ref(), &watched_path)
            .unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        let checkout = watched.find_commit(v1).unwrap();
        watched
            .reset(checkout.as_object(), git2::ResetType::Hard, None)
            .unwrap();
        drop(checkout);
        drop(watched);
        std::fs::write(watched_path.join("src/main.rs"), "local edit\n").unwrap();

        let err = processor_for(&watched_path)
            .update_from_remote()
            .unwrap_err();
        assert!(format!("{:?}", err).contains("local changes to tracked files"));
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        assert_eq!(
            std::fs::read_to_string(watched_path.join("src/main.rs")).unwrap(),
            "local edit\n"
        );
        drop(watched);

        write_fixture_crate(&watched_path, "fixture-version-one");
        processor_for(&watched_path).update_from_remote().unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        assert!(
            std::fs::read_to_string(watched_path.join("src/main.rs"))
                .unwrap()
                .contains("fixture-version-two")
        );
        drop(watched);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn untracked_file_in_the_way_refuses_fast_forward_until_removed() {
        let root = temp_dir("untracked-in-the-way");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        let v1 = commit_all(&upstream, "v1");
        std::fs::write(upstream_path.join("blocked.txt"), "from remote\n").unwrap();
        let v2 = commit_all(&upstream, "v2");
        drop(upstream);
        git2::Repository::clone(upstream_path.to_string_lossy().as_ref(), &watched_path)
            .unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        let checkout = watched.find_commit(v1).unwrap();
        watched
            .reset(checkout.as_object(), git2::ResetType::Hard, None)
            .unwrap();
        drop(checkout);
        drop(watched);
        std::fs::create_dir_all(watched_path.join("blocked.txt")).unwrap();
        std::fs::write(watched_path.join("blocked.txt/inner.txt"), "local\n").unwrap();

        let err = processor_for(&watched_path)
            .update_from_remote()
            .unwrap_err();
        assert!(format!("{:?}", err).contains("untracked files in the way"));
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        assert_eq!(
            std::fs::read_to_string(watched_path.join("blocked.txt/inner.txt")).unwrap(),
            "local\n"
        );
        drop(watched);

        std::fs::remove_dir_all(watched_path.join("blocked.txt")).unwrap();
        processor_for(&watched_path).update_from_remote().unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        assert_eq!(
            std::fs::read_to_string(watched_path.join("blocked.txt")).unwrap(),
            "from remote\n"
        );
        drop(watched);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn processor_with_tracker(watched: &Path, failed_builds: FailedBuilds) -> RepoProcessor {
        let mut processor = processor_for(watched);
        processor.failed_builds = failed_builds;
        processor
    }

    fn write_sentinel_fixture_crate(repo_path: &Path, marker: &str) {
        write_fixture_crate(repo_path, marker);
        std::fs::write(
            repo_path.join("build.rs"),
            "fn main() {\n    let sentinel = std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(\"fail-build\");\n    println!(\"cargo:rerun-if-changed=fail-build\");\n    if sentinel.exists() {\n        panic!(\"simulated transient build failure\");\n    }\n}\n",
        )
        .unwrap();
    }

    fn processor_with_strategy(watched: &Path, strategy: Option<&str>) -> RepoProcessor {
        RepoProcessor::new(RepositoryConfiguration {
            name: "watched".to_string(),
            path: watched.to_string_lossy().into_owned(),
            remote: "origin".to_string(),
            branch: "master".to_string(),
            build_strategy: strategy.map(|s| s.to_string()),
            build_command: String::new(),
            restart_command: None,
            out_dir: None,
        })
        .unwrap()
    }

    #[test]
    fn invalid_strategy_refuses_update_until_config_is_fixed() {
        let root = temp_dir("invalid-strategy");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        let v1 = commit_all(&upstream, "v1");
        write_fixture_crate(&upstream_path, "fixture-version-two");
        let v2 = commit_all(&upstream, "v2");
        drop(upstream);
        git2::Repository::clone(upstream_path.to_string_lossy().as_ref(), &watched_path)
            .unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        let checkout = watched.find_commit(v1).unwrap();
        watched
            .reset(checkout.as_object(), git2::ResetType::Hard, None)
            .unwrap();
        drop(checkout);
        drop(watched);

        let err = processor_with_strategy(&watched_path, None)
            .update_from_remote()
            .unwrap_err();
        assert!(format!("{:?}", err).contains("no build_strategy"));
        let err = processor_with_strategy(&watched_path, Some("webpack"))
            .update_from_remote()
            .unwrap_err();
        assert!(format!("{:?}", err).contains("webpack"));
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        drop(watched);
        assert!(
            std::fs::read_to_string(watched_path.join("src/main.rs"))
                .unwrap()
                .contains("fixture-version-one")
        );
        assert!(!fixture_artifact(&watched_path).exists());

        processor_with_strategy(&watched_path, Some("rust"))
            .update_from_remote()
            .unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        drop(watched);
        let artifact = fixture_artifact(&watched_path);
        let output = std::process::Command::new(&artifact).output().unwrap();
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("fixture-version-two")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_checkout_rolls_branch_back_and_next_cycle_retries() {
        let root = temp_dir("failed-checkout");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        let v1 = commit_all(&upstream, "v1");
        write_fixture_crate(&upstream_path, "fixture-version-two");
        let v2 = commit_all(&upstream, "v2");
        drop(upstream);
        git2::Repository::clone(upstream_path.to_string_lossy().as_ref(), &watched_path)
            .unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        let checkout = watched.find_commit(v1).unwrap();
        watched
            .reset(checkout.as_object(), git2::ResetType::Hard, None)
            .unwrap();
        drop(checkout);
        drop(watched);
        std::fs::write(watched_path.join(".git/index.lock"), "").unwrap();

        assert!(processor_for(&watched_path).update_from_remote().is_err());
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        drop(watched);
        assert!(
            std::fs::read_to_string(watched_path.join("src/main.rs"))
                .unwrap()
                .contains("fixture-version-one")
        );

        std::fs::remove_file(watched_path.join(".git/index.lock")).unwrap();
        processor_for(&watched_path).update_from_remote().unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        assert!(
            std::fs::read_to_string(watched_path.join("src/main.rs"))
                .unwrap()
                .contains("fixture-version-two")
        );
        drop(watched);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_build_is_retried_on_later_cycles_until_it_succeeds() {
        let root = temp_dir("failed-build-retry");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        let v1 = commit_all(&upstream, "v1");
        write_sentinel_fixture_crate(&upstream_path, "fixture-version-two");
        let v2 = commit_all(&upstream, "v2");
        drop(upstream);
        git2::Repository::clone(upstream_path.to_string_lossy().as_ref(), &watched_path)
            .unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        let checkout = watched.find_commit(v1).unwrap();
        watched
            .reset(checkout.as_object(), git2::ResetType::Hard, None)
            .unwrap();
        drop(checkout);
        drop(watched);

        let failed_builds = new_failed_builds();
        std::fs::write(watched_path.join("fail-build"), "").unwrap();

        let err = processor_with_tracker(&watched_path, failed_builds.clone())
            .update_from_remote()
            .unwrap_err();
        assert!(format!("{:?}", err).contains("Failed to build"));
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        drop(watched);
        assert!(!fixture_artifact(&watched_path).exists());

        let err = processor_with_tracker(&watched_path, failed_builds.clone())
            .update_from_remote()
            .unwrap_err();
        assert!(format!("{:?}", err).contains("Failed to build"));
        assert!(!fixture_artifact(&watched_path).exists());

        std::fs::write(watched_path.join("src/main.rs"), "local edit\n").unwrap();
        let err = processor_with_tracker(&watched_path, failed_builds.clone())
            .update_from_remote()
            .unwrap_err();
        assert!(format!("{:?}", err).contains("local changes to tracked files"));
        assert!(!fixture_artifact(&watched_path).exists());
        write_sentinel_fixture_crate(&watched_path, "fixture-version-two");

        std::fs::remove_file(watched_path.join("fail-build")).unwrap();
        processor_with_tracker(&watched_path, failed_builds.clone())
            .update_from_remote()
            .unwrap();
        let artifact = fixture_artifact(&watched_path);
        assert!(artifact.exists());
        let output = std::process::Command::new(&artifact).output().unwrap();
        assert!(output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("fixture-version-two")
        );

        std::fs::write(watched_path.join("fail-build"), "").unwrap();
        processor_with_tracker(&watched_path, failed_builds.clone())
            .update_from_remote()
            .unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn seeded_failure_for_a_bun_repo_is_retried_and_kept_when_bun_is_missing() {
        let user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
        let bun_path = if user == "root" {
            PathBuf::from("/root/.bun/bin/bun")
        } else {
            PathBuf::from("/home").join(&user).join(".bun/bin/bun")
        };
        if bun_path.exists() {
            return;
        }
        let root = temp_dir("bun-retry");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        let v1 = commit_all(&upstream, "v1");
        drop(upstream);
        git2::Repository::clone(upstream_path.to_string_lossy().as_ref(), &watched_path)
            .unwrap();

        let failed_builds = new_failed_builds();
        failed_builds
            .lock()
            .unwrap()
            .insert(tracker_key(&watched_path.to_string_lossy()), v1);
        let mut processor = processor_with_strategy(&watched_path, Some("bun"));
        processor.failed_builds = failed_builds.clone();
        let err = processor.update_from_remote().unwrap_err();
        assert!(format!("{:?}", err).contains("Bun is not installed"));
        assert_eq!(
            failed_builds
                .lock()
                .unwrap()
                .get(&tracker_key(&watched_path.to_string_lossy())),
            Some(&v1)
        );
        let mut processor = processor_with_strategy(&watched_path, Some("bun"));
        processor.failed_builds = failed_builds.clone();
        assert!(processor.update_from_remote().is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_build_record_is_cleared_when_a_newer_commit_builds() {
        let root = temp_dir("failed-build-cleared-by-newer");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        let v1 = commit_all(&upstream, "v1");
        write_sentinel_fixture_crate(&upstream_path, "fixture-version-two");
        let v2 = commit_all(&upstream, "v2");
        drop(upstream);
        git2::Repository::clone(upstream_path.to_string_lossy().as_ref(), &watched_path)
            .unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        let checkout = watched.find_commit(v1).unwrap();
        watched
            .reset(checkout.as_object(), git2::ResetType::Hard, None)
            .unwrap();
        drop(checkout);
        drop(watched);

        let failed_builds = new_failed_builds();
        std::fs::write(watched_path.join("fail-build"), "").unwrap();
        assert!(
            processor_with_tracker(&watched_path, failed_builds.clone())
                .update_from_remote()
                .is_err()
        );
        assert_eq!(
            failed_builds
                .lock()
                .unwrap()
                .get(&tracker_key(&watched_path.to_string_lossy())),
            Some(&v2)
        );

        let upstream = git2::Repository::open(&upstream_path).unwrap();
        write_sentinel_fixture_crate(&upstream_path, "fixture-version-three");
        let v3 = commit_all(&upstream, "v3");
        drop(upstream);
        std::fs::remove_file(watched_path.join("fail-build")).unwrap();
        processor_with_tracker(&watched_path, failed_builds.clone())
            .update_from_remote()
            .unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v3);
        drop(watched);
        assert!(
            !failed_builds
                .lock()
                .unwrap()
                .contains_key(&tracker_key(&watched_path.to_string_lossy())),
            "a successful build at a newer commit must clear the failure record stored for the older commit"
        );
        let artifact = fixture_artifact(&watched_path);
        let output = std::process::Command::new(&artifact).output().unwrap();
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("fixture-version-three")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    fn processor_with_path_str(
        path: String,
        failed_builds: FailedBuilds,
        failed_restarts: FailedRestarts,
    ) -> RepoProcessor {
        let mut processor = RepoProcessor::new(RepositoryConfiguration {
            name: "watched".to_string(),
            path,
            remote: "origin".to_string(),
            branch: "master".to_string(),
            build_strategy: Some("rust".to_string()),
            build_command: String::new(),
            restart_command: None,
            out_dir: None,
        })
        .unwrap();
        processor.failed_builds = failed_builds;
        processor.failed_restarts = failed_restarts;
        processor
    }

    #[test]
    fn tracker_key_collapses_spelling_variants_of_the_same_repo() {
        let root = temp_dir("tracker-key");
        let repo_path = root.join("repo");
        std::fs::create_dir_all(repo_path.join("sub")).unwrap();
        let real = repo_path.to_string_lossy().into_owned();
        let key = tracker_key(&real);
        assert_eq!(tracker_key(&format!("{}/", real)), key);
        assert_eq!(tracker_key(&format!("{}/./", real)), key);
        assert_eq!(
            tracker_key(&repo_path.join("sub").join("..").to_string_lossy()),
            key
        );
        assert_eq!(tracker_key("/nope/a/./b/../c"), "/nope/a/c");
        assert_eq!(tracker_key("a/./b/../c"), "a/c");
        assert_eq!(tracker_key("/../a"), "/a");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn tracker_key_resolves_symlinks_to_the_same_repo() {
        let root = temp_dir("tracker-key-symlink");
        let repo_path = root.join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();
        let link_path = root.join("link");
        std::os::unix::fs::symlink(&repo_path, &link_path).unwrap();
        assert_eq!(
            tracker_key(&link_path.to_string_lossy()),
            tracker_key(&repo_path.to_string_lossy())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_trackers_survive_a_path_spelling_edit() {
        let root = temp_dir("tracker-spelling");
        let repo_path = root.join("repo");
        git2::Repository::init(&repo_path).unwrap();
        let real = repo_path.to_string_lossy().into_owned();
        let failed_builds = new_failed_builds();
        let failed_restarts = new_failed_restarts();
        let oid = Oid::from_str("1111111111111111111111111111111111111111").unwrap();

        let plain = processor_with_path_str(
            real.clone(),
            failed_builds.clone(),
            failed_restarts.clone(),
        );
        plain.record_failed_build(oid);
        plain.record_failed_restart(oid);

        let respelled = processor_with_path_str(
            format!("{}/./", real),
            failed_builds.clone(),
            failed_restarts.clone(),
        );
        assert!(respelled.build_failed_at(oid));
        assert!(respelled.restart_failed_at(oid));

        let trailing = processor_with_path_str(
            format!("{}/", real),
            failed_builds.clone(),
            failed_restarts.clone(),
        );
        trailing.clear_failed_build();
        trailing.clear_failed_restart();
        assert!(!plain.build_failed_at(oid));
        assert!(!plain.restart_failed_at(oid));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn poisoned_trackers_degrade_without_panicking() {
        let root = temp_dir("tracker-poisoned");
        let repo_path = root.join("repo");
        git2::Repository::init(&repo_path).unwrap();
        let failed_builds = new_failed_builds();
        let failed_restarts = new_failed_restarts();
        for tracker in [&failed_builds, &failed_restarts] {
            let cloned = tracker.clone();
            let handle = std::thread::spawn(move || {
                let _guard = cloned.lock().unwrap();
                panic!("poison the tracker lock");
            });
            assert!(handle.join().is_err());
            assert!(tracker.lock().is_err());
        }
        let processor = processor_with_path_str(
            repo_path.to_string_lossy().into_owned(),
            failed_builds.clone(),
            failed_restarts.clone(),
        );
        let oid = Oid::from_str("2222222222222222222222222222222222222222").unwrap();
        assert!(!processor.build_failed_at(oid));
        assert!(!processor.restart_failed_at(oid));
        processor.record_failed_build(oid);
        processor.record_failed_restart(oid);
        processor.clear_failed_build();
        processor.clear_failed_restart();
        assert!(
            failed_builds
                .lock()
                .unwrap_err()
                .into_inner()
                .get(&tracker_key(&repo_path.to_string_lossy()))
                .is_none()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    fn processor_with_restart(
        watched: &Path,
        restart_command: Option<String>,
        failed_restarts: FailedRestarts,
    ) -> RepoProcessor {
        let mut processor = processor_for(watched);
        processor.config.restart_command = restart_command;
        processor.failed_restarts = failed_restarts;
        processor
    }

    fn serve_command(watched: &Path) -> String {
        format!(
            "\"{}\" > served.txt",
            fixture_artifact(watched).display()
        )
    }

    #[cfg(unix)]
    #[test]
    fn restart_command_succeeds_and_fails_with_its_stderr() {
        let dir = temp_dir("restart-unit");
        assert!(run_restart_command("true", &dir, "demo").is_ok());
        std::fs::write(dir.join("marker.txt"), "x").unwrap();
        assert!(run_restart_command("test -f marker.txt", &dir, "demo").is_ok());
        let err = run_restart_command("echo restart-boom >&2; exit 3", &dir, "demo")
            .unwrap_err();
        assert!(format!("{:?}", err).contains("Restart command for demo"));
        assert!(format!("{:?}", err).contains("restart-boom"));
        run_restart_command("echo ran > restart-ran.txt", &dir, "demo").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("restart-ran.txt")).unwrap(),
            "ran\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn restart_command_is_not_blocked_by_a_backgrounded_process() {
        let dir = temp_dir("restart-background");
        let started = std::time::Instant::now();
        run_restart_command("sleep 20 & echo started", &dir, "demo").unwrap();
        assert!(
            started.elapsed() < std::time::Duration::from_secs(15),
            "restart command blocked on a backgrounded process for {:?}",
            started.elapsed()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restart_runs_after_an_update_and_not_when_nothing_changes() {
        let root = temp_dir("restart-after-update");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        let v1 = commit_all(&upstream, "v1");
        drop(upstream);
        git2::Repository::clone(upstream_path.to_string_lossy().as_ref(), &watched_path)
            .unwrap();

        let failed_restarts = new_failed_restarts();
        let command = serve_command(&watched_path);
        processor_with_restart(
            &watched_path,
            Some(command.clone()),
            failed_restarts.clone(),
        )
        .update_from_remote()
        .unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        drop(watched);
        assert!(!watched_path.join("served.txt").exists());

        let upstream = git2::Repository::open(&upstream_path).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-two");
        let v2 = commit_all(&upstream, "v2");
        drop(upstream);
        processor_with_restart(
            &watched_path,
            Some(command.clone()),
            failed_restarts.clone(),
        )
        .update_from_remote()
        .unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        drop(watched);
        assert_eq!(
            std::fs::read_to_string(watched_path.join("served.txt")).unwrap(),
            "fixture-version-two\n"
        );

        std::fs::remove_file(watched_path.join("served.txt")).unwrap();
        processor_with_restart(
            &watched_path,
            Some(command),
            failed_restarts.clone(),
        )
        .update_from_remote()
        .unwrap();
        assert!(!watched_path.join("served.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_restart_is_reported_and_retried_until_it_succeeds() {
        let root = temp_dir("restart-retry");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        commit_all(&upstream, "v1");
        write_fixture_crate(&upstream_path, "fixture-version-two");
        let v2 = commit_all(&upstream, "v2");
        drop(upstream);
        git2::Repository::clone(upstream_path.to_string_lossy().as_ref(), &watched_path)
            .unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        let checkout = watched.find_commit(
            watched
                .revparse_single("HEAD~1")
                .unwrap()
                .id(),
        )
        .unwrap();
        watched
            .reset(checkout.as_object(), git2::ResetType::Hard, None)
            .unwrap();
        drop(checkout);
        drop(watched);

        let failed_restarts = new_failed_restarts();
        let err = processor_with_restart(
            &watched_path,
            Some("false".to_string()),
            failed_restarts.clone(),
        )
        .update_from_remote()
        .unwrap_err();
        assert!(format!("{:?}", err).contains("Restart command"));
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        drop(watched);
        assert_eq!(
            failed_restarts
                .lock()
                .unwrap()
                .get(&tracker_key(&watched_path.to_string_lossy())),
            Some(&v2)
        );

        let err = processor_with_restart(
            &watched_path,
            Some("false".to_string()),
            failed_restarts.clone(),
        )
        .update_from_remote()
        .unwrap_err();
        assert!(format!("{:?}", err).contains("Restart command"));

        std::fs::write(watched_path.join("src/main.rs"), "local edit\n").unwrap();
        let err = processor_with_restart(
            &watched_path,
            Some(serve_command(&watched_path)),
            failed_restarts.clone(),
        )
        .update_from_remote()
        .unwrap_err();
        assert!(
            format!("{:?}", err).contains("Refusing to run the restart command")
        );
        assert!(!watched_path.join("served.txt").exists());
        write_fixture_crate(&watched_path, "fixture-version-two");

        processor_with_restart(
            &watched_path,
            Some(serve_command(&watched_path)),
            failed_restarts.clone(),
        )
        .update_from_remote()
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(watched_path.join("served.txt")).unwrap(),
            "fixture-version-two\n"
        );
        assert!(
            !failed_restarts
                .lock()
                .unwrap()
                .contains_key(&tracker_key(&watched_path.to_string_lossy()))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn restart_does_not_run_for_an_out_dir_triggered_build() {
        let root = temp_dir("restart-skipped-for-out-dir-build");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        commit_all(&upstream, "v1");
        drop(upstream);
        git2::Repository::clone(upstream_path.to_string_lossy().as_ref(), &watched_path)
            .unwrap();

        let failed_restarts = new_failed_restarts();
        let mut processor = processor_with_restart(
            &watched_path,
            Some(serve_command(&watched_path)),
            failed_restarts.clone(),
        );
        processor.config.out_dir = Some("dist-out".to_string());
        processor.update_from_remote().unwrap();
        assert!(fixture_artifact(&watched_path).exists());
        assert!(!watched_path.join("served.txt").exists());

        let mut processor = processor_with_restart(
            &watched_path,
            Some(serve_command(&watched_path)),
            failed_restarts.clone(),
        );
        processor.config.out_dir = Some("dist-out".to_string());
        processor.update_from_remote().unwrap();
        assert!(!watched_path.join("served.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn restart_does_not_run_when_the_build_fails() {
        let root = temp_dir("restart-skipped-on-build-failure");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        let v1 = commit_all(&upstream, "v1");
        write_sentinel_fixture_crate(&upstream_path, "fixture-version-two");
        commit_all(&upstream, "v2");
        drop(upstream);
        git2::Repository::clone(upstream_path.to_string_lossy().as_ref(), &watched_path)
            .unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        let checkout = watched.find_commit(v1).unwrap();
        watched
            .reset(checkout.as_object(), git2::ResetType::Hard, None)
            .unwrap();
        drop(checkout);
        drop(watched);

        let failed_builds = new_failed_builds();
        let failed_restarts = new_failed_restarts();
        std::fs::write(watched_path.join("fail-build"), "").unwrap();
        let mut processor = processor_with_restart(
            &watched_path,
            Some(serve_command(&watched_path)),
            failed_restarts.clone(),
        );
        processor.failed_builds = failed_builds.clone();
        let err = processor.update_from_remote().unwrap_err();
        assert!(format!("{:?}", err).contains("Failed to build"));
        assert!(!watched_path.join("served.txt").exists());

        std::fs::remove_file(watched_path.join("fail-build")).unwrap();
        let mut processor = processor_with_restart(
            &watched_path,
            Some(serve_command(&watched_path)),
            failed_restarts.clone(),
        );
        processor.failed_builds = failed_builds.clone();
        processor.update_from_remote().unwrap();
        assert_eq!(
            std::fs::read_to_string(watched_path.join("served.txt")).unwrap(),
            "fixture-version-two\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
