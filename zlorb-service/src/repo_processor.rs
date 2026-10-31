use git2::{
    AnnotatedCommit, Branch, BranchType, Cred, FetchOptions, MergeAnalysis, MergePreference, Oid,
    Reference, Remote, RemoteCallbacks,
};
use std::{fmt::Debug, path::PathBuf};
use zlorb_lib::{config::RepositoryConfiguration, error::ZlorbError, log::Logger};

use crate::build_system_executor::BuildSystemExecutor;

pub struct RepoProcessor {
    pub(crate) repo_path: PathBuf,
    pub repo: git2::Repository,
    pub config: RepositoryConfiguration,
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
        })
    }

    pub fn update_from_remote(&self) -> Result<(), ZlorbError> {
        Logger::info(format!("Updating from remote for {}", self.config.name));
        let has_updates = self.fetch_remote_updates()?;
        Logger::info(format!("Remote updates found: {}", has_updates));

        let out_dir_missing = self.config.out_dir.is_some() && !self.check_if_out_dir_exists()?;
        Logger::info(format!("Out directory missing (expected): {}", out_dir_missing));

        let should_run_build = has_updates || out_dir_missing;
        Logger::info(format!("Should run build: {}", should_run_build));

        match should_run_build {
            true => {
                Logger::info(format!(
                    "Found updates or missing output dir for {}. Pulling changes...",
                    self.config.name
                ));
                let exec = BuildSystemExecutor { processor: self };
                exec.run_build()
            }
            false => {
                Logger::info(format!("No build needed for {}", self.config.name));
                Ok(())
            }
        }
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
}
