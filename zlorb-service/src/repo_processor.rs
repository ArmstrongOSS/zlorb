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

fn stored_tracker_key(raw_path: &str, repo_name: &str) -> String {
    match std::fs::canonicalize(raw_path) {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(e) => {
            Logger::error(format!(
                "Could not canonicalize the path for {} ({}): {}; failed-build and failed-restart tracking will use the normalized path spelling, which may not match other spellings of the same repo",
                repo_name, raw_path, e
            ));
            normalize_path_lexically(raw_path)
        }
    }
}

fn normalize_path_lexically(raw_path: &str) -> String {
    let path = Path::new(raw_path);
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                match normalized.components().last() {
                    Some(std::path::Component::Normal(_)) => {
                        normalized.pop();
                    }
                    Some(std::path::Component::ParentDir) => normalized.push(component),
                    None => {
                        if !path.is_absolute() {
                            normalized.push(component);
                        }
                    }
                    _ => {}
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

fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec()))
}

fn stat_matches_index_entry(entry: &git2::IndexEntry, metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    entry.ino != 0
        && metadata.ino() as u32 == entry.ino
        && metadata.len() == u64::from(entry.file_size)
        && metadata.mtime() == i64::from(entry.mtime.seconds())
        && metadata.mtime_nsec() == i64::from(entry.mtime.nanoseconds())
        && metadata.ctime() == i64::from(entry.ctime.seconds())
        && metadata.ctime_nsec() == i64::from(entry.ctime.nanoseconds())
}

fn display_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(", ")
}

fn directory_is_tracked_typechange(
    head_paths: &std::collections::HashSet<Vec<u8>>,
    workdir: &Path,
    prefix: &Path,
) -> bool {
    let mut dir_prefix = {
        use std::os::unix::ffi::OsStrExt;
        prefix.as_os_str().as_bytes().to_vec()
    };
    dir_prefix.push(b'/');
    if !head_paths.iter().any(|path| path.starts_with(&dir_prefix)) {
        return false;
    }
    let mut stack = vec![workdir.join(prefix)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return false;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                return false;
            };
            let Ok(file_type) = entry.file_type() else {
                return false;
            };
            if file_type.is_dir() {
                stack.push(entry.path());
                continue;
            }
            let entry_path = entry.path();
            let Ok(relative) = entry_path.strip_prefix(workdir) else {
                return false;
            };
            let relative_bytes = {
                use std::os::unix::ffi::OsStrExt;
                relative.as_os_str().as_bytes()
            };
            if !head_paths.contains(relative_bytes) {
                return false;
            }
        }
    }
    true
}

pub struct RepoProcessor {
    pub(crate) repo_path: PathBuf,
    pub repo: git2::Repository,
    pub config: RepositoryConfiguration,
    pub failed_builds: FailedBuilds,
    pub failed_restarts: FailedRestarts,
    tracker_key: String,
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
        let tracker_key = stored_tracker_key(&config.path, &config.name);
        Ok(Self {
            repo_path: PathBuf::from(&config.path),
            repo,
            config,
            failed_builds: new_failed_builds(),
            failed_restarts: new_failed_restarts(),
            tracker_key,
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
                    display_paths(&tracked_changes)
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
                    display_paths(&tracked_changes)
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
            &self.tracker_key,
            oid,
            "failed-restart",
            &self.config.name,
        )
    }

    fn record_failed_restart(&self, oid: Oid) {
        tracker_record(
            &self.failed_restarts,
            &self.tracker_key,
            oid,
            "failed-restart",
            &self.config.name,
        );
    }

    fn clear_failed_restart(&self) {
        tracker_clear(
            &self.failed_restarts,
            &self.tracker_key,
            "failed-restart",
            &self.config.name,
        );
    }

    fn build_failed_at(&self, oid: Oid) -> bool {
        tracker_failed_at(
            &self.failed_builds,
            &self.tracker_key,
            oid,
            "failed-build",
            &self.config.name,
        )
    }

    fn record_failed_build(&self, oid: Oid) {
        tracker_record(
            &self.failed_builds,
            &self.tracker_key,
            oid,
            "failed-build",
            &self.config.name,
        );
    }

    fn clear_failed_build(&self) {
        tracker_clear(
            &self.failed_builds,
            &self.tracker_key,
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
        let changed_paths = match self._changed_paths(local_oid, fetch_commit.id()) {
            Ok(paths) => Some(paths),
            Err(e) => {
                Logger::error(format!(
                    "Could not compute the paths changed by the update for {}; falling back to refusing on any local change and checking out the whole tree: {}",
                    self.config.name, e
                ));
                None
            }
        };
        self._refuse_if_update_would_overwrite(&fetch_commit, changed_paths.as_deref())?;
        let fetch_oid = fetch_commit.id();
        let remote_ref = match self._apply_fast_forward(fetch_commit, changed_paths.as_deref()) {
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
        changed_paths: Option<&[PathBuf]>,
    ) -> Result<(), ZlorbError> {
        let tracked_changes = self._tracked_local_changes()?;
        let tracked_changes: Vec<PathBuf> = match changed_paths {
            Some(paths) => tracked_changes
                .into_iter()
                .filter(|change| paths.contains(change))
                .collect(),
            None => tracked_changes,
        };
        let untracked_collisions = self._untracked_collisions(fetch_commit)?;
        if tracked_changes.is_empty() && untracked_collisions.is_empty() {
            return Ok(());
        }
        let mut reasons = Vec::new();
        if !tracked_changes.is_empty() {
            reasons.push(format!(
                "local changes to tracked files ({})",
                display_paths(&tracked_changes)
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
        let mut head_paths: std::collections::HashSet<Vec<u8>> =
            std::collections::HashSet::new();
        if let Ok(head) = self.repo.head() {
            if let Ok(head_commit) = head.peel_to_commit() {
                if let Ok(head_tree) = head_commit.tree() {
                    let _ = head_tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
                        if matches!(
                            entry.kind(),
                            Some(git2::ObjectType::Blob) | Some(git2::ObjectType::Commit)
                        ) {
                            let path =
                                PathBuf::from(root).join(entry.name().unwrap_or_default());
                            {
                                use std::os::unix::ffi::OsStrExt;
                                head_paths
                                    .insert(path.as_os_str().as_bytes().to_vec());
                            }
                        }
                        git2::TreeWalkResult::Ok
                    });
                }
            }
        }
        let mut typechange_cache: HashMap<PathBuf, bool> = HashMap::new();
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
                        let tracked_typechange = metadata.is_dir()
                            && *typechange_cache.entry(prefix.clone()).or_insert_with(|| {
                                directory_is_tracked_typechange(
                                    &head_paths, &workdir, &prefix,
                                )
                            });
                        if tracked_typechange {
                            break;
                        }
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

    fn _tracked_local_changes(&self) -> Result<Vec<PathBuf>, ZlorbError> {
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(false);
        opts.include_ignored(false);
        let statuses = self
            .repo
            .statuses(Some(&mut opts))
            .map_err(ZlorbError::Git)?;
        let mut changes: Vec<PathBuf> = statuses
            .iter()
            .map(|entry| path_from_bytes(entry.path_bytes()))
            .collect();
        changes.extend(self._hidden_local_changes(&changes));
        Ok(changes)
    }

    fn _hidden_local_changes(&self, known: &[PathBuf]) -> Vec<PathBuf> {
        let Some(workdir) = self.repo.workdir().map(|path| path.to_path_buf()) else {
            return Vec::new();
        };
        let index = match self.repo.index() {
            Ok(index) => index,
            Err(e) => {
                Logger::error(format!(
                    "Could not read the index for {}; local changes under skip-worktree or assume-unchanged cannot be checked this cycle and would not be reported: {}",
                    self.config.name, e
                ));
                return Vec::new();
            }
        };
        let file_mode = self
            .repo
            .config()
            .and_then(|config| config.get_bool("core.fileMode"))
            .unwrap_or(true);
        let mut hidden = Vec::new();
        for entry in index.iter() {
            if entry.flags & 0x3000 != 0 {
                continue;
            }
            let skip_worktree =
                git2::IndexEntryExtendedFlag::from_bits_truncate(entry.flags_extended)
                    .is_skip_worktree();
            let assume_unchanged =
                git2::IndexEntryFlag::from_bits_truncate(entry.flags).is_valid();
            if !skip_worktree && !assume_unchanged {
                continue;
            }
            let path = path_from_bytes(&entry.path);
            if known.contains(&path) || hidden.contains(&path) {
                continue;
            }
            let full_path = workdir.join(&path);
            let metadata = match std::fs::symlink_metadata(&full_path) {
                Ok(metadata) => metadata,
                Err(_) => {
                    if assume_unchanged {
                        hidden.push(path);
                    }
                    continue;
                }
            };
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                let Ok(target) = std::fs::read_link(&full_path) else {
                    continue;
                };
                let target_bytes = {
                    use std::os::unix::ffi::OsStrExt;
                    target.as_os_str().as_bytes().to_vec()
                };
                let Ok(disk_id) = Oid::hash_object(git2::ObjectType::Blob, &target_bytes)
                else {
                    continue;
                };
                if disk_id != entry.id {
                    hidden.push(path);
                }
                continue;
            }
            if !file_type.is_file() {
                if matches!(entry.mode, 0o100644 | 0o100755 | 0o120000) {
                    hidden.push(path);
                }
                continue;
            }
            if file_mode && matches!(entry.mode, 0o100644 | 0o100755) {
                use std::os::unix::fs::PermissionsExt;
                let workdir_mode = if metadata.permissions().mode() & 0o111 != 0 {
                    0o100755
                } else {
                    0o100644
                };
                if workdir_mode != entry.mode {
                    hidden.push(path);
                    continue;
                }
            }
            if stat_matches_index_entry(&entry, &metadata) {
                continue;
            }
            let mut writer = match self.repo.blob_writer(Some(&path)) {
                Ok(writer) => writer,
                Err(e) => {
                    Logger::error(format!(
                        "Could not check {} in {} for local changes under skip-worktree or assume-unchanged ({}); treating it as changed so the update is refused rather than risk discarding it",
                        path.display(),
                        self.config.name,
                        e
                    ));
                    hidden.push(path);
                    continue;
                }
            };
            let mut file = match std::fs::File::open(&full_path) {
                Ok(file) => file,
                Err(e) => {
                    Logger::error(format!(
                        "Could not read {} in {} to check it for local changes under skip-worktree or assume-unchanged ({}); treating it as changed so the update is refused rather than risk discarding it",
                        path.display(),
                        self.config.name,
                        e
                    ));
                    hidden.push(path);
                    continue;
                }
            };
            if let Err(e) = std::io::copy(&mut file, &mut writer) {
                Logger::error(format!(
                    "Could not read {} in {} to check it for local changes under skip-worktree or assume-unchanged ({}); treating it as changed so the update is refused rather than risk discarding it",
                    path.display(),
                    self.config.name,
                    e
                ));
                hidden.push(path);
                continue;
            }
            let disk_id = match writer.commit() {
                Ok(disk_id) => disk_id,
                Err(e) => {
                    Logger::error(format!(
                        "Could not hash {} in {} to check it for local changes under skip-worktree or assume-unchanged ({}); treating it as changed so the update is refused rather than risk discarding it",
                        path.display(),
                        self.config.name,
                        e
                    ));
                    hidden.push(path);
                    continue;
                }
            };
            if disk_id != entry.id {
                if !self._matches_after_external_clean_filter(
                    &workdir,
                    &path,
                    &full_path,
                    entry.id,
                ) {
                    hidden.push(path);
                }
            }
        }
        hidden
    }

    fn _matches_after_external_clean_filter(
        &self,
        workdir: &Path,
        path: &Path,
        full_path: &Path,
        expected: Oid,
    ) -> bool {
        self._matches_after_external_clean_filter_with_timeout(
            workdir,
            path,
            full_path,
            expected,
            std::time::Duration::from_secs(30),
        )
    }

    fn _matches_after_external_clean_filter_with_timeout(
        &self,
        workdir: &Path,
        path: &Path,
        full_path: &Path,
        expected: Oid,
        timeout: std::time::Duration,
    ) -> bool {
        use std::io::Read;
        let Some(filter_name) = self._external_clean_filter_name(path) else {
            return false;
        };
        Logger::info(format!(
            "Re-checking {} in {} through git hash-object so the external clean filter {} is applied",
            path.display(),
            self.config.name,
            filter_name
        ));
        let mut child = match Command::new("git")
            .arg("-C")
            .arg(workdir)
            .arg("hash-object")
            .arg("--")
            .arg(full_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                Logger::error(format!(
                    "Could not run git hash-object for {} in {} to apply the external clean filter {} ({}); treating it as changed so the update is refused rather than risk discarding it",
                    path.display(),
                    self.config.name,
                    filter_name,
                    e
                ));
                return false;
            }
        };
        let stdout_reader = child.stdout.take().map(|mut pipe| {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = pipe.read_to_end(&mut buf);
                buf
            })
        });
        let stderr_reader = child.stderr.take().map(|mut pipe| {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = pipe.read_to_end(&mut buf);
                buf
            })
        });
        let deadline = std::time::Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        Logger::error(format!(
                            "git hash-object for {} in {} did not finish within {} seconds while applying the external clean filter {}; treating it as changed so the update is refused rather than risk discarding it",
                            path.display(),
                            self.config.name,
                            timeout.as_secs(),
                            filter_name
                        ));
                        return false;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => {
                    Logger::error(format!(
                        "Could not wait for git hash-object for {} in {} to apply the external clean filter {} ({}); treating it as changed so the update is refused rather than risk discarding it",
                        path.display(),
                        self.config.name,
                        filter_name,
                        e
                    ));
                    return false;
                }
            }
        };
        let stdout = stdout_reader
            .and_then(|reader| reader.join().ok())
            .unwrap_or_default();
        let stderr = stderr_reader
            .and_then(|reader| reader.join().ok())
            .unwrap_or_default();
        if !status.success() {
            Logger::error(format!(
                "git hash-object for {} in {} failed ({}) while applying the external clean filter {}; treating it as changed so the update is refused rather than risk discarding it: {}",
                path.display(),
                self.config.name,
                status,
                filter_name,
                String::from_utf8_lossy(&stderr).trim()
            ));
            return false;
        }
        let text = String::from_utf8_lossy(&stdout);
        match Oid::from_str(text.trim()) {
            Ok(external_id) => {
                if external_id != expected {
                    Logger::warn(format!(
                        "Re-checking {} in {} through the external clean filter {} still produced a different hash than the index; treating it as changed",
                        path.display(),
                        self.config.name,
                        filter_name
                    ));
                }
                external_id == expected
            }
            Err(e) => {
                Logger::error(format!(
                    "Could not parse the git hash-object result for {} in {} ({}); treating it as changed so the update is refused rather than risk discarding it",
                    path.display(),
                    self.config.name,
                    e
                ));
                false
            }
        }
    }

    fn _external_clean_filter_name(&self, path: &Path) -> Option<String> {
        let value = self
            .repo
            .get_attr(path, "filter", git2::AttrCheckFlags::FILE_THEN_INDEX)
            .ok()?;
        let name = match git2::AttrValue::from_string(value) {
            git2::AttrValue::String(name) => name.to_string(),
            _ => return None,
        };
        let config = self.repo.config().ok()?;
        let configured = ["clean", "process"].iter().any(|kind| {
            config
                .get_string(&format!("filter.{}.{}", name, kind))
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false)
        });
        if configured {
            Some(name)
        } else {
            None
        }
    }

    fn _apply_fast_forward(
        &self,
        fetch_commit: AnnotatedCommit,
        changed_paths: Option<&[PathBuf]>,
    ) -> Result<Reference<'_>, ZlorbError> {
        self._setup_repo_for_checkout(fetch_commit)?;
        self._checkout_and_get_ref(changed_paths)
    }

    fn _changed_paths(&self, from: Oid, to: Oid) -> Result<Vec<PathBuf>, ZlorbError> {
        let from_tree = self
            .repo
            .find_commit(from)
            .map_err(ZlorbError::Git)?
            .tree()
            .map_err(ZlorbError::Git)?;
        let to_tree = self
            .repo
            .find_commit(to)
            .map_err(ZlorbError::Git)?
            .tree()
            .map_err(ZlorbError::Git)?;
        let diff = self
            .repo
            .diff_tree_to_tree(Some(&from_tree), Some(&to_tree), None)
            .map_err(ZlorbError::Git)?;
        let mut seen = std::collections::HashSet::new();
        let mut paths: Vec<PathBuf> = Vec::new();
        for delta in diff.deltas() {
            for file in [delta.old_file(), delta.new_file()] {
                if let Some(path) = file.path() {
                    let path = path.to_path_buf();
                    if seen.insert(path.clone()) {
                        paths.push(path);
                    }
                }
            }
        }
        Ok(paths)
    }

    fn _rollback_fast_forward(&self, local_oid: Oid, fetch_oid: Oid) -> bool {
        let refname = format!("refs/heads/{}", self.config.branch);
        let mut branch_restored = false;
        match self.repo.find_reference(&refname) {
            Ok(mut reference) => {
                match reference.set_target(local_oid, "Rolling back failed fast-forward") {
                    Ok(_) => branch_restored = true,
                    Err(e) => {
                        Logger::error(format!(
                            "Failed to roll back {} to {}: {}",
                            refname, local_oid, e
                        ));
                    }
                }
            }
            Err(e) => {
                Logger::error(format!(
                    "Failed to find {} to roll back {} after a failed fast-forward: {}",
                    refname, self.config.name, e
                ));
            }
        }
        self._remove_partial_update_files(local_oid, fetch_oid);
        let changed_paths = match self._changed_paths(local_oid, fetch_oid) {
            Ok(paths) => Some(paths),
            Err(e) => {
                Logger::error(format!(
                    "Could not compute the paths changed by the update for {}; rolling back with a whole-tree restore, which may discard local changes outside the update: {}",
                    self.config.name, e
                ));
                None
            }
        };
        let mut tree_restored = false;
        let restore_result = match self.repo.find_commit(local_oid) {
            Ok(commit) => {
                if matches!(&changed_paths, Some(paths) if paths.is_empty()) {
                    Ok(())
                } else {
                    let mut checkout = git2::build::CheckoutBuilder::default();
                    checkout.force();
                    checkout.disable_pathspec_match(true);
                    if let Some(paths) = &changed_paths {
                        for path in paths {
                            checkout.path(path);
                        }
                    }
                    self.repo
                        .checkout_tree(commit.as_object(), Some(&mut checkout))
                }
            }
            Err(e) => Err(e),
        };
        match restore_result {
            Ok(_) => tree_restored = true,
            Err(e) => {
                Logger::error(format!(
                    "Failed to restore the working tree after a failed fast-forward: {}",
                    e
                ));
            }
        }
        let mut head_restored = !branch_restored;
        if branch_restored {
            match self.repo.set_head(&refname) {
                Ok(_) => head_restored = true,
                Err(e) => {
                    Logger::error(format!(
                        "Failed to point HEAD at {} after rolling back a failed fast-forward: {}",
                        refname, e
                    ));
                }
            }
        }
        let verified = self._verify_rollback(local_oid, fetch_oid);
        if verified && !(tree_restored && head_restored) {
            Logger::error(format!(
                "Rollback verification failed for {}: {} is back at {}, but the working tree or HEAD could not be restored to match, so the rolled-back state does not hold and this update will not be built from it without manual intervention",
                self.config.name, refname, local_oid
            ));
            return false;
        }
        verified
    }

    fn _verify_rollback(&self, local_oid: Oid, fetch_oid: Oid) -> bool {
        let refname = format!("refs/heads/{}", self.config.branch);
        let reference = match self.repo.find_reference(&refname) {
            Ok(reference) => reference,
            Err(e) => {
                Logger::error(format!(
                    "Rollback verification failed for {}: {} could not be read after rolling back a failed fast-forward ({}), expected {}; the repository state is unknown and this update will not be retried or built without manual intervention",
                    self.config.name, refname, e, local_oid
                ));
                return false;
            }
        };
        match reference.target() {
            Some(oid) if oid == local_oid => true,
            Some(oid) if oid == fetch_oid => {
                let head_detached_at_local = self.repo.head_detached().unwrap_or(false)
                    && self
                        .repo
                        .head()
                        .ok()
                        .and_then(|head| head.target())
                        == Some(local_oid);
                if head_detached_at_local {
                    Logger::error(format!(
                        "Rollback verification failed for {}: {} is {} after rolling back a failed fast-forward, expected {}; the branch is left at the fetched commit {}, but HEAD is detached at {}, so later cycles compare against the detached HEAD and may retry the update while the branch itself will not move without manual intervention",
                        self.config.name, refname, oid, local_oid, fetch_oid, local_oid
                    ));
                } else {
                    Logger::error(format!(
                        "Rollback verification failed for {}: {} is {} after rolling back a failed fast-forward, expected {}; the branch and HEAD are left at the fetched commit {} and later cycles will read it as up to date, so this update will not be retried or built without manual intervention",
                        self.config.name, refname, oid, local_oid, fetch_oid
                    ));
                }
                false
            }
            Some(oid) => {
                Logger::error(format!(
                    "Rollback verification failed for {}: {} is {} after rolling back a failed fast-forward, expected {}; it is not the fetched commit {}, so the rolled-back state does not hold and the next cycle will re-read the branch against the remote",
                    self.config.name, refname, oid, local_oid, fetch_oid
                ));
                false
            }
            None => {
                Logger::error(format!(
                    "Rollback verification failed for {}: {} has no target after rolling back a failed fast-forward, expected {}; the repository state is unknown and this update will not be retried or built without manual intervention",
                    self.config.name, refname, local_oid
                ));
                false
            }
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
            let Ok(metadata) = std::fs::symlink_metadata(&full_path) else {
                continue;
            };
            let file_type = metadata.file_type();
            let contents = if file_type.is_symlink() {
                let Ok(target) = std::fs::read_link(&full_path) else {
                    continue;
                };
                {
                    use std::os::unix::ffi::OsStrExt;
                    target.as_os_str().as_bytes().to_vec()
                }
            } else if file_type.is_file() {
                let Ok(contents) = std::fs::read(&full_path) else {
                    continue;
                };
                contents
            } else {
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

    fn _checkout_and_get_ref(
        &self,
        changed_paths: Option<&[PathBuf]>,
    ) -> Result<Reference<'_>, ZlorbError> {
        let skip_checkout = matches!(changed_paths, Some(paths) if paths.is_empty());
        if !skip_checkout {
            let mut checkout = git2::build::CheckoutBuilder::default();
            checkout.force();
            checkout.disable_pathspec_match(true);
            if let Some(paths) = changed_paths {
                for path in paths {
                    checkout.path(path);
                }
            }
            self.repo
                .checkout_head(Some(&mut checkout))
                .map_err(ZlorbError::Git)?;
        }

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
        let name = "fixture-app";
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
    fn wip_on_a_file_the_update_does_not_touch_survives_fast_forward() {
        let root = temp_dir("wip-untouched-survives");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        std::fs::write(upstream_path.join("keep.txt"), "committed base\n").unwrap();
        std::fs::write(upstream_path.join("old.txt"), "old file\n").unwrap();
        let v1 = commit_all(&upstream, "v1");
        write_fixture_crate(&upstream_path, "fixture-version-two");
        std::fs::remove_file(upstream_path.join("old.txt")).unwrap();
        std::fs::write(upstream_path.join("new.txt"), "new file\n").unwrap();
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
        std::fs::write(watched_path.join("keep.txt"), "standing wip\n").unwrap();

        processor_for(&watched_path).update_from_remote().unwrap();

        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        drop(watched);
        assert!(
            std::fs::read_to_string(watched_path.join("src/main.rs"))
                .unwrap()
                .contains("fixture-version-two")
        );
        assert_eq!(
            std::fs::read_to_string(watched_path.join("new.txt")).unwrap(),
            "new file\n"
        );
        assert!(!watched_path.join("old.txt").exists());
        assert_eq!(
            std::fs::read_to_string(watched_path.join("keep.txt")).unwrap(),
            "standing wip\n"
        );
        let artifact = fixture_artifact(&watched_path);
        assert!(artifact.exists());
        let watched = git2::Repository::open(&watched_path).unwrap();
        let statuses = watched.statuses(None).unwrap();
        let tracked_dirty: Vec<(String, git2::Status)> = statuses
            .iter()
            .filter(|entry| entry.status() != git2::Status::WT_NEW)
            .map(|entry| {
                (
                    String::from_utf8_lossy(entry.path_bytes()).into_owned(),
                    entry.status(),
                )
            })
            .collect();
        assert_eq!(
            tracked_dirty,
            vec![("keep.txt".to_string(), git2::Status::WT_MODIFIED)]
        );
        drop(statuses);
        drop(watched);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn set_repo_config(watched: &Path, key: &str, value: &str) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(watched)
            .args(["config", key, value])
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn set_index_flag(watched: &Path, flag: &str, path: &str) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(watched)
            .args(["update-index", flag, path])
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn repo_reset_to_v1_with_upstream_v2(
        label: &str,
    ) -> (PathBuf, PathBuf, Oid, Oid) {
        let root = temp_dir(label);
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        std::fs::write(upstream_path.join("keep.txt"), "committed base\n").unwrap();
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
        (root, watched_path, v1, v2)
    }

    #[test]
    fn skip_worktree_edit_on_a_touched_file_refuses_fast_forward() {
        let (root, watched_path, v1, _v2) =
            repo_reset_to_v1_with_upstream_v2("skip-worktree-touched");
        std::fs::write(watched_path.join("src/main.rs"), "local edit\n").unwrap();
        set_index_flag(&watched_path, "--skip-worktree", "src/main.rs");

        let processor = processor_for(&watched_path);
        assert_eq!(
            processor._tracked_local_changes().unwrap(),
            vec![PathBuf::from("src/main.rs")]
        );
        let err = processor.update_from_remote().unwrap_err();
        assert!(format!("{:?}", err).contains("local changes to tracked files"));
        drop(processor);
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        drop(watched);
        assert_eq!(
            std::fs::read_to_string(watched_path.join("src/main.rs")).unwrap(),
            "local edit\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn assume_unchanged_edit_on_a_touched_file_refuses_fast_forward() {
        let (root, watched_path, v1, _v2) =
            repo_reset_to_v1_with_upstream_v2("assume-unchanged-touched");
        std::fs::write(watched_path.join("src/main.rs"), "local edit\n").unwrap();
        set_index_flag(&watched_path, "--assume-unchanged", "src/main.rs");

        let processor = processor_for(&watched_path);
        assert_eq!(
            processor._tracked_local_changes().unwrap(),
            vec![PathBuf::from("src/main.rs")]
        );
        let err = processor.update_from_remote().unwrap_err();
        assert!(format!("{:?}", err).contains("local changes to tracked files"));
        drop(processor);
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        drop(watched);
        assert_eq!(
            std::fs::read_to_string(watched_path.join("src/main.rs")).unwrap(),
            "local edit\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skip_worktree_edit_on_an_untouched_file_survives_fast_forward() {
        let (root, watched_path, _v1, v2) =
            repo_reset_to_v1_with_upstream_v2("skip-worktree-untouched");
        std::fs::write(watched_path.join("keep.txt"), "standing wip\n").unwrap();
        set_index_flag(&watched_path, "--skip-worktree", "keep.txt");

        processor_for(&watched_path).update_from_remote().unwrap();

        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        drop(watched);
        assert!(
            std::fs::read_to_string(watched_path.join("src/main.rs"))
                .unwrap()
                .contains("fixture-version-two")
        );
        assert_eq!(
            std::fs::read_to_string(watched_path.join("keep.txt")).unwrap(),
            "standing wip\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skip_worktree_without_an_edit_does_not_block_fast_forward() {
        let (root, watched_path, _v1, v2) =
            repo_reset_to_v1_with_upstream_v2("skip-worktree-clean");
        set_index_flag(&watched_path, "--skip-worktree", "src/main.rs");

        let processor = processor_for(&watched_path);
        assert!(processor._tracked_local_changes().unwrap().is_empty());
        processor.update_from_remote().unwrap();
        drop(processor);

        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        drop(watched);
        assert!(
            std::fs::read_to_string(watched_path.join("src/main.rs"))
                .unwrap()
                .contains("fixture-version-two")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skip_worktree_file_with_matching_stat_skips_the_hash_check() {
        let (root, watched_path, _v1, _v2) =
            repo_reset_to_v1_with_upstream_v2("skip-worktree-stat-shortcut");
        std::fs::write(
            watched_path.join("src/main.rs"),
            "fn main() { println!(\"local-edit-versionx\"); }\n",
        )
        .unwrap();
        {
            use std::os::unix::fs::MetadataExt;
            let watched = git2::Repository::open(&watched_path).unwrap();
            let mut index = watched.index().unwrap();
            let mut entry = index.get_path(Path::new("src/main.rs"), 0).unwrap();
            let metadata = std::fs::metadata(watched_path.join("src/main.rs")).unwrap();
            entry.mtime =
                git2::IndexTime::new(metadata.mtime() as i32, metadata.mtime_nsec() as u32);
            entry.ctime =
                git2::IndexTime::new(metadata.ctime() as i32, metadata.ctime_nsec() as u32);
            entry.ino = metadata.ino() as u32;
            entry.file_size = metadata.len() as u32;
            entry.flags_extended |= 0x4000;
            index.add(&entry).unwrap();
            index.write().unwrap();
        }

        let processor = processor_for(&watched_path);
        assert!(processor._tracked_local_changes().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skip_worktree_file_with_stale_stat_falls_back_to_the_hash_check() {
        let (root, watched_path, _v1, _v2) =
            repo_reset_to_v1_with_upstream_v2("skip-worktree-stat-stale");
        set_index_flag(&watched_path, "--skip-worktree", "src/main.rs");
        std::fs::write(
            watched_path.join("src/main.rs"),
            "fn main() { println!(\"local-edit-versionx\"); }\n",
        )
        .unwrap();

        let processor = processor_for(&watched_path);
        assert_eq!(
            processor._tracked_local_changes().unwrap(),
            vec![PathBuf::from("src/main.rs")]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skip_worktree_file_with_only_mtime_restored_still_falls_back_to_the_hash_check() {
        let (root, watched_path, _v1, _v2) =
            repo_reset_to_v1_with_upstream_v2("skip-worktree-stat-mtime-restored");
        set_index_flag(&watched_path, "--skip-worktree", "src/main.rs");
        let recorded_mtime = std::fs::metadata(watched_path.join("src/main.rs"))
            .unwrap()
            .modified()
            .unwrap();
        std::fs::write(
            watched_path.join("src/main.rs"),
            "fn main() { println!(\"local-edit-versionx\"); }\n",
        )
        .unwrap();
        let file = std::fs::File::options()
            .write(true)
            .open(watched_path.join("src/main.rs"))
            .unwrap();
        file.set_modified(recorded_mtime).unwrap();
        drop(file);

        let processor = processor_for(&watched_path);
        assert_eq!(
            processor._tracked_local_changes().unwrap(),
            vec![PathBuf::from("src/main.rs")]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skip_worktree_retargeted_symlink_on_a_touched_path_refuses_fast_forward() {
        let root = temp_dir("skip-worktree-symlink");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        std::os::unix::fs::symlink("src/main.rs", upstream_path.join("link.txt")).unwrap();
        let v1 = commit_all(&upstream, "v1");
        std::fs::remove_file(upstream_path.join("link.txt")).unwrap();
        std::os::unix::fs::symlink("Cargo.toml", upstream_path.join("link.txt")).unwrap();
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
        std::fs::remove_file(watched_path.join("link.txt")).unwrap();
        std::os::unix::fs::symlink("local-target", watched_path.join("link.txt")).unwrap();
        set_index_flag(&watched_path, "--skip-worktree", "link.txt");

        let processor = processor_for(&watched_path);
        assert_eq!(
            processor._tracked_local_changes().unwrap(),
            vec![PathBuf::from("link.txt")]
        );
        let err = processor.update_from_remote().unwrap_err();
        assert!(format!("{:?}", err).contains("local changes to tracked files"));
        drop(processor);
        assert_eq!(
            std::fs::read_link(watched_path.join("link.txt")).unwrap(),
            PathBuf::from("local-target")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skip_worktree_file_matching_its_blob_after_clean_filters_is_not_a_change() {
        let root = temp_dir("skip-worktree-crlf");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        std::fs::write(upstream_path.join(".gitattributes"), "*.txt text eol=crlf\n").unwrap();
        std::fs::write(upstream_path.join("data.txt"), "line one\nline two\n").unwrap();
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
        std::fs::write(watched_path.join("data.txt"), "line one\r\nline two\r\n").unwrap();
        set_index_flag(&watched_path, "--skip-worktree", "data.txt");

        let processor = processor_for(&watched_path);
        assert!(processor._tracked_local_changes().unwrap().is_empty());
        processor.update_from_remote().unwrap();
        drop(processor);

        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        drop(watched);
        assert_eq!(
            std::fs::read(watched_path.join("data.txt")).unwrap(),
            b"line one\r\nline two\r\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skip_worktree_file_matching_its_blob_after_an_external_clean_filter_is_not_a_change() {
        let root = temp_dir("skip-worktree-external-filter");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        std::fs::write(upstream_path.join(".gitattributes"), "*.txt filter=strip\n").unwrap();
        std::fs::write(upstream_path.join("data.txt"), "hello\n").unwrap();
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
        set_repo_config(&watched_path, "filter.strip.clean", "sed 's/^X//'");
        std::fs::write(watched_path.join("data.txt"), "Xhello\n").unwrap();
        set_index_flag(&watched_path, "--skip-worktree", "data.txt");

        let processor = processor_for(&watched_path);
        assert!(processor._tracked_local_changes().unwrap().is_empty());
        processor.update_from_remote().unwrap();
        drop(processor);

        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        drop(watched);
        assert_eq!(
            std::fs::read(watched_path.join("data.txt")).unwrap(),
            b"Xhello\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skip_worktree_file_changed_after_an_external_clean_filter_still_refuses_fast_forward() {
        let root = temp_dir("skip-worktree-external-filter-changed");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        std::fs::write(upstream_path.join(".gitattributes"), "*.txt filter=strip\n").unwrap();
        std::fs::write(upstream_path.join("data.txt"), "hello\n").unwrap();
        let v1 = commit_all(&upstream, "v1");
        std::fs::write(upstream_path.join("data.txt"), "version two\n").unwrap();
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
        set_repo_config(&watched_path, "filter.strip.clean", "sed 's/^X//'");
        std::fs::write(watched_path.join("data.txt"), "Xgoodbye\n").unwrap();
        set_index_flag(&watched_path, "--skip-worktree", "data.txt");

        let processor = processor_for(&watched_path);
        assert_eq!(
            processor._tracked_local_changes().unwrap(),
            vec![PathBuf::from("data.txt")]
        );
        let err = processor.update_from_remote().unwrap_err();
        assert!(format!("{:?}", err).contains("local changes to tracked files"));
        drop(processor);
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        drop(watched);
        assert_eq!(
            std::fs::read(watched_path.join("data.txt")).unwrap(),
            b"Xgoodbye\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn hanging_external_clean_filter_times_out_and_is_treated_as_a_change() {
        let root = temp_dir("skip-worktree-external-filter-hang");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        std::fs::write(upstream_path.join(".gitattributes"), "*.txt filter=hang\n").unwrap();
        std::fs::write(upstream_path.join("data.txt"), "hello\n").unwrap();
        let v1 = commit_all(&upstream, "v1");
        drop(upstream);
        git2::Repository::clone(upstream_path.to_string_lossy().as_ref(), &watched_path)
            .unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        drop(watched);
        set_repo_config(&watched_path, "filter.hang.clean", "sleep 5");
        std::fs::write(watched_path.join("data.txt"), "Xhello\n").unwrap();
        set_index_flag(&watched_path, "--skip-worktree", "data.txt");

        let processor = processor_for(&watched_path);
        let entry_id = processor
            .repo
            .index()
            .unwrap()
            .get_path(Path::new("data.txt"), 0)
            .unwrap()
            .id;
        let started = std::time::Instant::now();
        let matched = processor._matches_after_external_clean_filter_with_timeout(
            &watched_path,
            Path::new("data.txt"),
            &watched_path.join("data.txt"),
            entry_id,
            std::time::Duration::from_millis(200),
        );
        assert!(!matched);
        assert!(started.elapsed() < std::time::Duration::from_secs(4));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn assume_unchanged_deleted_file_refuses_fast_forward() {
        let (root, watched_path, v1, _v2) =
            repo_reset_to_v1_with_upstream_v2("assume-unchanged-deleted");
        set_index_flag(&watched_path, "--assume-unchanged", "src/main.rs");
        std::fs::remove_file(watched_path.join("src/main.rs")).unwrap();

        let processor = processor_for(&watched_path);
        assert_eq!(
            processor._tracked_local_changes().unwrap(),
            vec![PathBuf::from("src/main.rs")]
        );
        let err = processor.update_from_remote().unwrap_err();
        assert!(format!("{:?}", err).contains("local changes to tracked files"));
        drop(processor);
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        drop(watched);
        assert!(!watched_path.join("src/main.rs").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skip_worktree_deleted_file_survives_fast_forward() {
        let (root, watched_path, _v1, v2) =
            repo_reset_to_v1_with_upstream_v2("skip-worktree-deleted");
        set_index_flag(&watched_path, "--skip-worktree", "keep.txt");
        std::fs::remove_file(watched_path.join("keep.txt")).unwrap();

        let processor = processor_for(&watched_path);
        assert_eq!(
            processor._tracked_local_changes().unwrap(),
            vec![PathBuf::from("keep.txt")]
        );
        processor.update_from_remote().unwrap();
        drop(processor);

        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        drop(watched);
        assert!(!watched_path.join("keep.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skip_worktree_mode_change_on_a_touched_file_refuses_fast_forward() {
        let (root, watched_path, v1, _v2) =
            repo_reset_to_v1_with_upstream_v2("skip-worktree-mode");
        set_index_flag(&watched_path, "--skip-worktree", "src/main.rs");
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions =
                std::fs::metadata(watched_path.join("src/main.rs")).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(watched_path.join("src/main.rs"), permissions).unwrap();
        }

        let processor = processor_for(&watched_path);
        assert_eq!(
            processor._tracked_local_changes().unwrap(),
            vec![PathBuf::from("src/main.rs")]
        );
        let err = processor.update_from_remote().unwrap_err();
        assert!(format!("{:?}", err).contains("local changes to tracked files"));
        drop(processor);
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        drop(watched);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn assume_unchanged_file_replaced_by_a_directory_refuses_fast_forward() {
        let (root, watched_path, v1, _v2) =
            repo_reset_to_v1_with_upstream_v2("assume-unchanged-dir");
        set_index_flag(&watched_path, "--assume-unchanged", "src/main.rs");
        std::fs::remove_file(watched_path.join("src/main.rs")).unwrap();
        std::fs::create_dir(watched_path.join("src/main.rs")).unwrap();
        std::fs::write(watched_path.join("src/main.rs/precious.txt"), "precious\n").unwrap();

        let processor = processor_for(&watched_path);
        assert_eq!(
            processor._tracked_local_changes().unwrap(),
            vec![PathBuf::from("src/main.rs")]
        );
        let err = processor.update_from_remote().unwrap_err();
        assert!(format!("{:?}", err).contains("local changes to tracked files"));
        drop(processor);
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        drop(watched);
        assert_eq!(
            std::fs::read_to_string(watched_path.join("src/main.rs/precious.txt")).unwrap(),
            "precious\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skip_worktree_file_replaced_by_a_directory_refuses_fast_forward() {
        let (root, watched_path, v1, _v2) =
            repo_reset_to_v1_with_upstream_v2("skip-worktree-dir");
        set_index_flag(&watched_path, "--skip-worktree", "src/main.rs");
        std::fs::remove_file(watched_path.join("src/main.rs")).unwrap();
        std::fs::create_dir(watched_path.join("src/main.rs")).unwrap();
        std::fs::write(watched_path.join("src/main.rs/precious.txt"), "precious\n").unwrap();

        let processor = processor_for(&watched_path);
        assert_eq!(
            processor._tracked_local_changes().unwrap(),
            vec![PathBuf::from("src/main.rs")]
        );
        let err = processor.update_from_remote().unwrap_err();
        assert!(format!("{:?}", err).contains("local changes to tracked files"));
        drop(processor);
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        drop(watched);
        assert_eq!(
            std::fs::read_to_string(watched_path.join("src/main.rs/precious.txt")).unwrap(),
            "precious\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rollback_preserves_wip_on_a_file_the_update_does_not_touch() {
        let root = temp_dir("rollback-wip-untouched");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let watched = git2::Repository::init_opts(&watched_path, &init_opts).unwrap();
        write_fixture_crate(&watched_path, "fixture-version-one");
        std::fs::write(watched_path.join("keep.txt"), "committed base\n").unwrap();
        std::fs::write(watched_path.join("only-v1.txt"), "only in v1\n").unwrap();
        let v1 = commit_all(&watched, "v1");
        write_fixture_crate(&watched_path, "fixture-version-two");
        std::fs::remove_file(watched_path.join("only-v1.txt")).unwrap();
        std::fs::write(watched_path.join("new.txt"), "new in v2\n").unwrap();
        let v2 = commit_all(&watched, "v2");
        drop(watched);
        std::fs::write(watched_path.join("keep.txt"), "standing wip\n").unwrap();

        let processor = processor_for(&watched_path);
        assert!(processor._rollback_fast_forward(v1, v2));
        assert_eq!(head_oid(&processor.repo), v1);
        assert!(
            std::fs::read_to_string(watched_path.join("src/main.rs"))
                .unwrap()
                .contains("fixture-version-one")
        );
        assert_eq!(
            std::fs::read_to_string(watched_path.join("keep.txt")).unwrap(),
            "standing wip\n"
        );
        assert_eq!(
            std::fs::read_to_string(watched_path.join("only-v1.txt")).unwrap(),
            "only in v1\n"
        );
        assert!(!watched_path.join("new.txt").exists());
        drop(processor);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn glob_characters_in_a_changed_filename_do_not_expand_to_sibling_files() {
        let root = temp_dir("glob-filename");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        std::fs::write(upstream_path.join("data[1].txt"), "glob one\n").unwrap();
        std::fs::write(upstream_path.join("data1.txt"), "sibling base\n").unwrap();
        let v1 = commit_all(&upstream, "v1");
        std::fs::write(upstream_path.join("data[1].txt"), "glob two\n").unwrap();
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
        std::fs::write(watched_path.join("data1.txt"), "sibling wip\n").unwrap();

        processor_for(&watched_path).update_from_remote().unwrap();

        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        drop(watched);
        assert_eq!(
            std::fs::read_to_string(watched_path.join("data[1].txt")).unwrap(),
            "glob two\n"
        );
        assert_eq!(
            std::fs::read_to_string(watched_path.join("data1.txt")).unwrap(),
            "sibling wip\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    fn commit_all_no_delete(repo: &git2::Repository, message: &str) -> Oid {
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .unwrap();
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

    #[test]
    fn non_utf8_filename_in_the_update_still_refuses_when_dirty() {
        use std::os::unix::ffi::OsStringExt;
        let root = temp_dir("non-utf8-guard");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        let bad_name = std::ffi::OsString::from_vec(b"bad-\xff.txt".to_vec());
        std::fs::write(upstream_path.join(&bad_name), "version one\n").unwrap();
        let v1 = commit_all_no_delete(&upstream, "v1");
        std::fs::write(upstream_path.join(&bad_name), "version two\n").unwrap();
        commit_all_no_delete(&upstream, "v2");
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
        std::fs::write(watched_path.join(&bad_name), "local wip\n").unwrap();

        let err = processor_for(&watched_path)
            .update_from_remote()
            .unwrap_err();
        assert!(format!("{:?}", err).contains("local changes to tracked files"));
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        drop(watched);
        assert_eq!(
            std::fs::read(watched_path.join(&bad_name)).unwrap(),
            b"local wip\n"
        );
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

    #[test]
    fn directory_to_file_typechange_fast_forwards_on_clean_tree() {
        let root = temp_dir("dir-to-file-typechange");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        std::fs::create_dir_all(upstream_path.join("thing")).unwrap();
        std::fs::write(upstream_path.join("thing/inner.txt"), "inner\n").unwrap();
        std::fs::write(upstream_path.join("thing/other.txt"), "other\n").unwrap();
        let v1 = commit_all(&upstream, "v1");
        std::fs::remove_dir_all(upstream_path.join("thing")).unwrap();
        std::fs::write(upstream_path.join("thing"), "now a file\n").unwrap();
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
        assert!(watched_path.join("thing").is_dir());

        processor_for(&watched_path).update_from_remote().unwrap();

        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        drop(watched);
        assert!(watched_path.join("thing").is_file());
        assert_eq!(
            std::fs::read_to_string(watched_path.join("thing")).unwrap(),
            "now a file\n"
        );
        assert!(
            std::fs::read_to_string(watched_path.join("src/main.rs"))
                .unwrap()
                .contains("fixture-version-two")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn untracked_file_inside_tracked_directory_still_refuses_typechange() {
        let root = temp_dir("dir-to-file-untracked-inside");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        std::fs::create_dir_all(upstream_path.join("thing")).unwrap();
        std::fs::write(upstream_path.join("thing/inner.txt"), "inner\n").unwrap();
        let v1 = commit_all(&upstream, "v1");
        std::fs::remove_dir_all(upstream_path.join("thing")).unwrap();
        std::fs::write(upstream_path.join("thing"), "now a file\n").unwrap();
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
        std::fs::write(watched_path.join("thing/stray.txt"), "untracked\n").unwrap();

        let err = processor_for(&watched_path)
            .update_from_remote()
            .unwrap_err();
        assert!(format!("{:?}", err).contains("untracked files in the way"));
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        drop(watched);
        assert_eq!(
            std::fs::read_to_string(watched_path.join("thing/stray.txt")).unwrap(),
            "untracked\n"
        );

        std::fs::remove_file(watched_path.join("thing/stray.txt")).unwrap();
        processor_for(&watched_path).update_from_remote().unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        drop(watched);
        assert!(watched_path.join("thing").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn staged_addition_inside_tracked_directory_still_refuses_typechange() {
        let root = temp_dir("dir-to-file-staged-inside");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        std::fs::create_dir_all(upstream_path.join("thing")).unwrap();
        std::fs::write(upstream_path.join("thing/inner.txt"), "inner\n").unwrap();
        let v1 = commit_all(&upstream, "v1");
        std::fs::remove_dir_all(upstream_path.join("thing")).unwrap();
        std::fs::write(upstream_path.join("thing"), "now a file\n").unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-two");
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
        std::fs::write(watched_path.join("thing/staged.txt"), "staged\n").unwrap();
        let watched = git2::Repository::open(&watched_path).unwrap();
        let mut index = watched.index().unwrap();
        index
            .add_path(Path::new("thing/staged.txt"))
            .unwrap();
        index.write().unwrap();
        drop(index);
        drop(watched);

        let err = processor_for(&watched_path)
            .update_from_remote()
            .unwrap_err();
        assert!(format!("{:?}", err).contains("untracked files in the way"));
        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v1);
        drop(watched);
        assert_eq!(
            std::fs::read_to_string(watched_path.join("thing/staged.txt")).unwrap(),
            "staged\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn directory_to_file_typechange_with_empty_subdirectory_fast_forwards() {
        let root = temp_dir("dir-to-file-empty-subdir");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        std::fs::create_dir_all(upstream_path.join("thing")).unwrap();
        std::fs::write(upstream_path.join("thing/inner.txt"), "inner\n").unwrap();
        let v1 = commit_all(&upstream, "v1");
        std::fs::remove_dir_all(upstream_path.join("thing")).unwrap();
        std::fs::write(upstream_path.join("thing"), "now a file\n").unwrap();
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
        std::fs::create_dir_all(watched_path.join("thing/empty")).unwrap();

        processor_for(&watched_path).update_from_remote().unwrap();

        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        drop(watched);
        assert!(watched_path.join("thing").is_file());
        assert_eq!(
            std::fs::read_to_string(watched_path.join("thing")).unwrap(),
            "now a file\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn nested_directory_to_file_typechange_fast_forwards() {
        let root = temp_dir("nested-dir-to-file");
        let upstream_path = root.join("upstream");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-one");
        std::fs::create_dir_all(upstream_path.join("a/b")).unwrap();
        std::fs::write(upstream_path.join("a/b/inner.txt"), "inner\n").unwrap();
        std::fs::write(upstream_path.join("a/keep.txt"), "keep\n").unwrap();
        let v1 = commit_all(&upstream, "v1");
        std::fs::remove_dir_all(upstream_path.join("a/b")).unwrap();
        std::fs::write(upstream_path.join("a/b"), "now a file\n").unwrap();
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

        processor_for(&watched_path).update_from_remote().unwrap();

        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        drop(watched);
        assert!(watched_path.join("a/b").is_file());
        assert_eq!(
            std::fs::read_to_string(watched_path.join("a/b")).unwrap(),
            "now a file\n"
        );
        assert_eq!(
            std::fs::read_to_string(watched_path.join("a/keep.txt")).unwrap(),
            "keep\n"
        );
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
    fn rollback_cleanup_removes_matching_symlink_and_skips_fifo() {
        let root = temp_dir("rollback-cleanup-types");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let watched = git2::Repository::init_opts(&watched_path, &init_opts).unwrap();
        write_fixture_crate(&watched_path, "fixture-version-one");
        let v1 = commit_all(&watched, "v1");
        std::fs::write(watched_path.join("new.txt"), "new file\n").unwrap();
        std::os::unix::fs::symlink("src/main.rs", watched_path.join("link.txt")).unwrap();
        std::os::unix::fs::symlink("Cargo.toml", watched_path.join("stale-link.txt")).unwrap();
        std::fs::write(watched_path.join("fifo.txt"), "fifo content\n").unwrap();
        let v2 = commit_all(&watched, "v2");
        let v1_commit = watched.find_commit(v1).unwrap();
        watched
            .reset(v1_commit.as_object(), git2::ResetType::Mixed, None)
            .unwrap();
        drop(v1_commit);
        drop(watched);
        std::fs::remove_file(watched_path.join("stale-link.txt")).unwrap();
        std::os::unix::fs::symlink("src/main.rs", watched_path.join("stale-link.txt")).unwrap();
        std::fs::remove_file(watched_path.join("fifo.txt")).unwrap();
        let mkfifo = std::process::Command::new("mkfifo")
            .arg(watched_path.join("fifo.txt"))
            .status()
            .unwrap();
        assert!(mkfifo.success());

        processor_for(&watched_path)._remove_partial_update_files(v1, v2);

        assert!(!watched_path.join("new.txt").exists());
        assert!(std::fs::symlink_metadata(watched_path.join("link.txt")).is_err());
        assert!(
            std::fs::symlink_metadata(watched_path.join("stale-link.txt"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        use std::os::unix::fs::FileTypeExt;
        assert!(
            std::fs::symlink_metadata(watched_path.join("fifo.txt"))
                .unwrap()
                .file_type()
                .is_fifo()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rollback_verification_reports_restored_and_unrestored_head() {
        let root = temp_dir("rollback-verification");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let watched = git2::Repository::init_opts(&watched_path, &init_opts).unwrap();
        write_fixture_crate(&watched_path, "fixture-version-one");
        let v1 = commit_all(&watched, "v1");
        write_fixture_crate(&watched_path, "fixture-version-two");
        let v2 = commit_all(&watched, "v2");
        drop(watched);

        let processor = processor_for(&watched_path);
        assert!(processor._rollback_fast_forward(v1, v2));
        assert_eq!(head_oid(&processor.repo), v1);
        assert!(
            std::fs::read_to_string(watched_path.join("src/main.rs"))
                .unwrap()
                .contains("fixture-version-one")
        );

        let v2_commit = processor.repo.find_commit(v2).unwrap();
        processor
            .repo
            .reset(v2_commit.as_object(), git2::ResetType::Hard, None)
            .unwrap();
        drop(v2_commit);
        assert_eq!(head_oid(&processor.repo), v2);

        let lock_path = watched_path.join(".git/refs/heads/master.lock");
        std::fs::write(&lock_path, "").unwrap();
        assert!(!processor._rollback_fast_forward(v1, v2));
        assert_eq!(head_oid(&processor.repo), v2);

        std::fs::remove_file(&lock_path).unwrap();
        assert!(processor._rollback_fast_forward(v1, v2));
        assert_eq!(head_oid(&processor.repo), v1);
        drop(processor);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rollback_verification_follows_branch_ref_not_detached_head() {
        let root = temp_dir("rollback-verification-branch-ref");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let watched = git2::Repository::init_opts(&watched_path, &init_opts).unwrap();
        write_fixture_crate(&watched_path, "fixture-version-one");
        let v1 = commit_all(&watched, "v1");
        write_fixture_crate(&watched_path, "fixture-version-two");
        let v2 = commit_all(&watched, "v2");
        drop(watched);

        let processor = processor_for(&watched_path);
        let branch_target = |processor: &RepoProcessor| {
            processor
                .repo
                .find_reference("refs/heads/master")
                .unwrap()
                .target()
                .unwrap()
        };

        processor.repo.set_head_detached(v1).unwrap();
        let lock_path = watched_path.join(".git/refs/heads/master.lock");
        std::fs::write(&lock_path, "").unwrap();
        assert!(!processor._rollback_fast_forward(v1, v2));
        assert_eq!(branch_target(&processor), v2);
        assert_eq!(head_oid(&processor.repo), v1);
        assert!(
            std::fs::read_to_string(watched_path.join("src/main.rs"))
                .unwrap()
                .contains("fixture-version-one")
        );

        std::fs::remove_file(&lock_path).unwrap();
        processor.repo.set_head_detached(v2).unwrap();
        assert!(processor._rollback_fast_forward(v1, v2));
        assert_eq!(branch_target(&processor), v1);
        assert_eq!(head_oid(&processor.repo), v1);
        assert!(!processor.repo.head_detached().unwrap());
        assert!(
            std::fs::read_to_string(watched_path.join("src/main.rs"))
                .unwrap()
                .contains("fixture-version-one")
        );
        drop(processor);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rollback_restores_rolled_back_tree_when_head_detached_at_fetched_commit() {
        let root = temp_dir("rollback-tree-detached-at-fetched");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let watched = git2::Repository::init_opts(&watched_path, &init_opts).unwrap();
        write_fixture_crate(&watched_path, "fixture-version-one");
        std::fs::write(watched_path.join("only-v1.txt"), "only in v1\n").unwrap();
        let v1 = commit_all(&watched, "v1");
        write_fixture_crate(&watched_path, "fixture-version-two");
        std::fs::remove_file(watched_path.join("only-v1.txt")).unwrap();
        std::fs::write(watched_path.join("new.txt"), "new in v2\n").unwrap();
        let v2 = commit_all(&watched, "v2");
        drop(watched);

        let processor = processor_for(&watched_path);
        processor.repo.set_head_detached(v2).unwrap();
        let lock_path = watched_path.join(".git/refs/heads/master.lock");
        std::fs::write(&lock_path, "").unwrap();
        assert!(!processor._rollback_fast_forward(v1, v2));
        assert_eq!(
            processor
                .repo
                .find_reference("refs/heads/master")
                .unwrap()
                .target()
                .unwrap(),
            v2
        );
        assert!(
            std::fs::read_to_string(watched_path.join("src/main.rs"))
                .unwrap()
                .contains("fixture-version-one")
        );
        assert!(!watched_path.join("new.txt").exists());
        assert_eq!(
            std::fs::read_to_string(watched_path.join("only-v1.txt")).unwrap(),
            "only in v1\n"
        );
        assert!(processor.repo.head_detached().unwrap());
        drop(processor);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rollback_reports_failure_when_the_tree_cannot_be_restored() {
        let root = temp_dir("rollback-tree-restore-failure");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let watched = git2::Repository::init_opts(&watched_path, &init_opts).unwrap();
        write_fixture_crate(&watched_path, "fixture-version-one");
        std::fs::write(watched_path.join("only-v1.txt"), "only in v1\n").unwrap();
        let v1 = commit_all(&watched, "v1");
        write_fixture_crate(&watched_path, "fixture-version-two");
        std::fs::remove_file(watched_path.join("only-v1.txt")).unwrap();
        std::fs::write(watched_path.join("new.txt"), "new in v2\n").unwrap();
        let v2 = commit_all(&watched, "v2");
        drop(watched);

        let processor = processor_for(&watched_path);
        let index_lock = watched_path.join(".git/index.lock");
        std::fs::write(&index_lock, "").unwrap();
        assert!(!processor._rollback_fast_forward(v1, v2));
        assert_eq!(
            processor
                .repo
                .find_reference("refs/heads/master")
                .unwrap()
                .target()
                .unwrap(),
            v1
        );

        std::fs::remove_file(&index_lock).unwrap();
        assert!(processor._rollback_fast_forward(v1, v2));
        assert!(
            std::fs::read_to_string(watched_path.join("src/main.rs"))
                .unwrap()
                .contains("fixture-version-one")
        );
        assert!(!watched_path.join("new.txt").exists());
        assert_eq!(
            std::fs::read_to_string(watched_path.join("only-v1.txt")).unwrap(),
            "only in v1\n"
        );
        let mut status_opts = git2::StatusOptions::new();
        status_opts.include_untracked(false);
        let tracked_status_clean = {
            let statuses = processor
                .repo
                .statuses(Some(&mut status_opts))
                .unwrap();
            statuses.is_empty()
        };
        assert!(tracked_status_clean);
        drop(processor);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rollback_reports_failure_when_head_cannot_be_reattached() {
        let root = temp_dir("rollback-head-restore-failure");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let watched = git2::Repository::init_opts(&watched_path, &init_opts).unwrap();
        write_fixture_crate(&watched_path, "fixture-version-one");
        let v1 = commit_all(&watched, "v1");
        write_fixture_crate(&watched_path, "fixture-version-two");
        let v2 = commit_all(&watched, "v2");
        drop(watched);

        let processor = processor_for(&watched_path);
        processor.repo.set_head_detached(v2).unwrap();
        let head_lock = watched_path.join(".git/HEAD.lock");
        std::fs::write(&head_lock, "").unwrap();
        assert!(!processor._rollback_fast_forward(v1, v2));
        assert_eq!(
            processor
                .repo
                .find_reference("refs/heads/master")
                .unwrap()
                .target()
                .unwrap(),
            v1
        );

        std::fs::remove_file(&head_lock).unwrap();
        assert!(processor._rollback_fast_forward(v1, v2));
        assert_eq!(head_oid(&processor.repo), v1);
        assert!(!processor.repo.head_detached().unwrap());
        drop(processor);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rollback_verification_fails_when_branch_ref_cannot_be_read() {
        let root = temp_dir("rollback-verification-unreadable-ref");
        let watched_path = root.join("watched");
        let mut init_opts = RepositoryInitOptions::new();
        init_opts.initial_head("master");
        let watched = git2::Repository::init_opts(&watched_path, &init_opts).unwrap();
        write_fixture_crate(&watched_path, "fixture-version-one");
        let v1 = commit_all(&watched, "v1");
        write_fixture_crate(&watched_path, "fixture-version-two");
        let v2 = commit_all(&watched, "v2");
        drop(watched);

        let processor = processor_for(&watched_path);
        let ref_path = watched_path.join(".git/refs/heads/master");
        assert!(ref_path.exists());
        std::fs::remove_file(&ref_path).unwrap();
        assert!(!processor._verify_rollback(v1, v2));

        std::fs::write(&ref_path, "ref: refs/heads/master-missing\n").unwrap();
        assert!(!processor._verify_rollback(v1, v2));
        drop(processor);
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
        assert_eq!(normalize_path_lexically("../../a"), "../../a");
        assert_eq!(normalize_path_lexically("../.."), "../..");
        assert_eq!(normalize_path_lexically("x/../../../y"), "../../y");
        assert_eq!(normalize_path_lexically("../a/../.."), "../..");
        assert_eq!(normalize_path_lexically("a/../../b"), "../b");
        let _ = std::fs::remove_dir_all(&root);
    }

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
    fn failed_trackers_are_shared_across_a_symlinked_spelling() {
        let root = temp_dir("tracker-symlink-processors");
        let repo_path = root.join("repo");
        git2::Repository::init(&repo_path).unwrap();
        let link_path = root.join("link");
        std::os::unix::fs::symlink(&repo_path, &link_path).unwrap();
        let failed_builds = new_failed_builds();
        let failed_restarts = new_failed_restarts();
        let oid = Oid::from_str("3333333333333333333333333333333333333333").unwrap();

        let via_link = processor_with_path_str(
            link_path.to_string_lossy().into_owned(),
            failed_builds.clone(),
            failed_restarts.clone(),
        );
        via_link.record_failed_build(oid);
        via_link.record_failed_restart(oid);

        let via_real = processor_with_path_str(
            repo_path.to_string_lossy().into_owned(),
            failed_builds.clone(),
            failed_restarts.clone(),
        );
        assert!(via_real.build_failed_at(oid));
        assert!(via_real.restart_failed_at(oid));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tracker_key_is_stored_at_construction_not_recomputed_per_operation() {
        let root = temp_dir("tracker-stored-key");
        let repo_path = root.join("repo");
        git2::Repository::init(&repo_path).unwrap();
        let link_path = root.join("link");
        std::os::unix::fs::symlink(&repo_path, &link_path).unwrap();
        let failed_builds = new_failed_builds();
        let failed_restarts = new_failed_restarts();
        let oid = Oid::from_str("4444444444444444444444444444444444444444").unwrap();

        let processor = processor_with_path_str(
            link_path.to_string_lossy().into_owned(),
            failed_builds.clone(),
            failed_restarts.clone(),
        );
        std::fs::remove_file(&link_path).unwrap();
        processor.record_failed_build(oid);
        assert_eq!(
            failed_builds
                .lock()
                .unwrap()
                .get(&tracker_key(&repo_path.to_string_lossy())),
            Some(&oid),
            "the record must land under the canonical key captured at construction, not a per-operation fallback spelling"
        );
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
