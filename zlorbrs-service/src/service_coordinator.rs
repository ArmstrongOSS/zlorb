use zlorbrs_lib::{config::RepoConfig, error::ZlorbError, get_home_dir, read_file_from_filesystem};

use crate::repo_processor::RepoProcessor;
use crate::service_config::ServiceConfig;

/// The primary orchestrator. It manages the system's global state and the continuous operation loop.
pub struct ServiceCoordinator {
    service_config: ServiceConfig,
    repo_configs: Vec<RepoProcessor>,
    first_run_flag: bool,
}

impl ServiceCoordinator {
    pub fn new(service_config: ServiceConfig) -> Self {
        // we're frontloading all of our configs on start so theyre kept in memory
        // reducing the need to hit the file system every iteration
        let configs = ServiceCoordinator::gather_repo_configs().unwrap();
        ServiceCoordinator {
            service_config,
            repo_configs: configs,
            first_run_flag: true,
        }
    }

    pub fn run_loop(&mut self) -> Result<(), ZlorbError> {
        loop {
            // we're throttling the loop so as to not peg
            // the cpu at max usage
            self.wait_for_run();
            self.run_cycle()?;
        }
    }

    fn wait_for_run(&mut self) {
        if !self.first_run_flag {
            std::thread::sleep(std::time::Duration::from_secs(
                self.service_config.sleep_time,
            ));
        } else {
            self.first_run_flag = false
        }
    }

    fn run_cycle(&self) -> Result<(), ZlorbError> {
        self.repo_configs.iter().for_each(|repo| {
            let _r = repo.update_from_remote();
        });
        Ok(())
    }

    fn gather_repo_configs() -> Result<Vec<RepoProcessor>, ZlorbError> {
        let configs_dir_path = get_home_dir().join(".config/zlorbrs/configs");
        let configs_dir = std::fs::read_dir(&configs_dir_path)
            .map_err(|_| ZlorbError::ConfigNotFound(configs_dir_path))?;
        let configs = configs_dir
            .map(|d| {
                let dir = d.unwrap();
                let p = dir.path().join("config.json");
                let file_contents = read_file_from_filesystem(p).unwrap();
                let repo = serde_json::from_str::<RepoConfig>(&file_contents).unwrap();
                let repo_processor = RepoProcessor::new(repo);
                return repo_processor;
            })
            .collect();

        Ok(configs)
    }
}
