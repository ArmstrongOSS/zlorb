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
    pub fn new(config: RepositoryConfiguration) -> Self {
        Logger::info(format!("Initializing RepoProcessor for {}", config.name));
        let err = format!("Failed to acquire repo: {}", config.path);
        let repo = git2::Repository::open(&config.path).expect(err.as_str());

        Self {
            repo_path: PathBuf::from(&config.path),
            repo,
            config,
        }
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
        // TODO Dont hardcode origin
        let remote = self.repo.find_remote("origin").map_err(ZlorbError::Git)?;
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
        self.repo
            .checkout_head(Some(&mut checkout))
            .map_err(ZlorbError::Git)?;

        let remote_ref = self
            .repo
            .resolve_reference_from_short_name(&format!("origin/{}", self.config.branch))
            .map_err(ZlorbError::Git)?;

        Ok(remote_ref)
    }

    fn _should_trigger_update(
        &self,
        remote_ref: Reference,
        local_oid: Oid,
    ) -> Result<bool, ZlorbError> {
        let remote_iod: Oid = remote_ref.target().expect("Remote ref has no target");
        let dist_dir_path = PathBuf::from(&self.config.path).join("dist");
        let dist_dir_exists = std::fs::exists(dist_dir_path).map_err(ZlorbError::Io)?;
        let local_iod_matches_remote = local_oid == remote_iod;
        if !dist_dir_exists || !local_iod_matches_remote {
            return Ok(true);
        }
        Ok(false)
    }
}
