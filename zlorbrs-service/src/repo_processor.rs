use git2::{
    AnnotatedCommit, Branch, BranchType, Cred, FetchOptions, MergeAnalysis, MergePreference, Oid,
    Reference, Remote, RemoteCallbacks,
};
use std::{fmt::Debug, path::PathBuf, process::Stdio};
use zlorbrs_lib::{config::RepoConfig, error::ZlorbError};

pub struct RepoProcessor {
    pub(crate) repo_path: PathBuf,
    pub repo: git2::Repository,
    pub config: RepoConfig,
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
    pub fn new(config: RepoConfig) -> Self {
        let err = format!("Failed to acquire repo: {}", config.path);
        let repo = git2::Repository::open(&config.path).expect(err.as_str());

        return Self {
            repo_path: PathBuf::from(&config.path),
            repo,
            config,
        };
    }

    pub fn update_from_remote(&self) -> Result<(), ZlorbError> {
        let has_updates = self.fetch_remote_updates()?;
        if has_updates {
            self.run_build();
        }
        Ok(())
    }

    fn fetch_remote_updates(&self) -> Result<bool, ZlorbError> {
        let local_branch = self._get_local_branch()?;
        let local_oid = self._get_local_oid_from_branch(local_branch)?;
        let mut remote = self._get_remote()?;
        self._download_new_data(&mut remote);
        let analysis = self._get_analysis()?;
        let (merge_analysis, _, fetch_commit) = analysis;
        if merge_analysis.is_up_to_date() {
            return Ok(true);
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

        Ok(self._should_trigger_update(remote_ref, local_oid)?)
    }

    fn run_build(&self) -> Result<(), ZlorbError> {
        let path = self.config.path.clone();
        let build_command = self.config.build_command.clone();

        let handle = std::thread::spawn(move || -> Result<(), ZlorbError> {
            std::env::set_current_dir(path).map_err(|e| ZlorbError::Io(e))?;
            let build_handle = std::process::Command::new(build_command)
                .stdout(Stdio::piped())
                .output()
                .map_err(|_| ZlorbError::Other("Unable to retrieve build handle".to_string()))?;

            if build_handle.status.code() == Some(1) {
                return Err(ZlorbError::Other(
                    "build returned status code 1 resulting in failure".to_string(),
                ));
            }
            Ok(())
        });

        handle
            .join()
            .map_err(|_| ZlorbError::Other("Faild to join the thread".to_string()))?;
        Ok(())
    }

    fn _setup_fetchoptions_with_creds(&self) -> FetchOptions {
        let mut callbacks = RemoteCallbacks::new();
        //TODO: Complete the credential stuff
        callbacks.credentials(|_, _, _| Cred::userpass_plaintext("", ""));
        let mut fetch_options = FetchOptions::new();
        fetch_options.remote_callbacks(callbacks);
        fetch_options
    }

    fn _get_local_branch(&self) -> Result<Branch, ZlorbError> {
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

    fn _get_remote(&self) -> Result<Remote, ZlorbError> {
        let remote = self
            .repo
            .find_remote("origin")
            .map_err(|e| ZlorbError::Git(e))?;
        Ok(remote)
    }

    fn _download_new_data(&self, remote: &mut Remote) {
        remote.fetch(
            &[self.config.branch.clone()],
            Some(&mut self._setup_fetchoptions_with_creds()),
            None,
        );
    }

    fn _get_analysis(
        &self,
    ) -> Result<(MergeAnalysis, MergePreference, AnnotatedCommit), ZlorbError> {
        let r = &self.repo;
        let fetch_head = r
            .find_reference("FETCH_HEAD")
            .map_err(|e| ZlorbError::Git(e))?;
        let fetch_commit = r
            .reference_to_annotated_commit(&fetch_head)
            .map_err(|e| ZlorbError::Git(e))?;
        let analysis = r
            .merge_analysis(&[&fetch_commit])
            .map_err(|e| ZlorbError::Git(e))?;
        let (m, n) = analysis;
        Ok((m, n, fetch_commit))
    }

    fn _setup_repo_for_checkout(&self, fetch_commit: AnnotatedCommit) -> Result<(), ZlorbError> {
        let refname = format!("refs/heads/{}", self.config.branch);
        self.repo
            .find_reference(&refname)
            .map_err(|e| ZlorbError::Git(e))?
            .set_target(fetch_commit.id(), "Fast-Forward")
            .map_err(|e| ZlorbError::Git(e))?;
        self.repo
            .set_head(&refname)
            .map_err(|e| ZlorbError::Git(e))?;
        Ok(())
    }

    fn _checkout_and_get_ref(&self) -> Result<Reference, ZlorbError> {
        let mut checkout = git2::build::CheckoutBuilder::default();
        self.repo
            .checkout_head(Some(&mut checkout))
            .map_err(|e| ZlorbError::Git(e))?;

        let remote_ref = self
            .repo
            .resolve_reference_from_short_name(&format!("origin/{}", self.config.branch))
            .map_err(|e| ZlorbError::Git(e))?;

        Ok(remote_ref)
    }

    fn _should_trigger_update(
        &self,
        remote_ref: Reference,
        local_oid: Oid,
    ) -> Result<bool, ZlorbError> {
        let remote_iod: Oid = remote_ref.target().expect("Remote ref has no target");
        let dist_dir_path = PathBuf::from(&self.config.path).join("/dist");
        let dist_dir_exists = std::fs::exists(dist_dir_path).map_err(|e| ZlorbError::Io(e))?;
        let local_iod_matches_remote = local_oid == remote_iod;
        if !dist_dir_exists || !local_iod_matches_remote {
            return Ok(true);
        }
        return Ok(false);
    }
}
