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
        self._setup_repo_for_checkout(fetch_commit)?;
        let remote_ref = self._checkout_and_get_ref()?;

        self._should_trigger_update(remote_ref, local_oid)
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
        assert!(!watched_path.join("target/debug/fixture-app").exists());
        drop(watched);

        let upstream = git2::Repository::open(&upstream_path).unwrap();
        write_fixture_crate(&upstream_path, "fixture-version-two");
        let v2 = commit_all(&upstream, "v2");
        drop(upstream);

        processor_for(&watched_path).update_from_remote().unwrap();

        let watched = git2::Repository::open(&watched_path).unwrap();
        assert_eq!(head_oid(&watched), v2);
        let source = std::fs::read_to_string(watched_path.join("src/main.rs")).unwrap();
        assert!(source.contains("fixture-version-two"));
        let statuses = watched.statuses(None).unwrap();
        assert!(
            statuses
                .iter()
                .all(|entry| entry.path() != Some("src/main.rs")),
            "src/main.rs must be clean after the fast-forward, got: {:?}",
            statuses
                .iter()
                .map(|entry| (entry.path().map(str::to_string), entry.status()))
                .collect::<Vec<_>>()
        );
        let artifact = watched_path.join("target/debug/fixture-app");
        assert!(artifact.exists());
        let output = std::process::Command::new(&artifact).output().unwrap();
        assert!(output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("fixture-version-two")
        );
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
    }
}
