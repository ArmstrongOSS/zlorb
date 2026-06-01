use zlorbrs_lib::{config::RepoConfig, error::ZlorbError, get_home_dir, read_file_from_filesystem};

use crate::config_manager::ConfigManager;
use crate::repo_processor::RepoProcessor;
use crate::service_config::ServiceConfig;

/// The primary orchestrator. It manages the system's global state and the continuous operation loop.
pub struct ServiceCoordinator {
    service_config: ServiceConfig,
    repo_configs: Vec<RepoProcessor>,
    first_run_flag: bool,
    config_manager: ConfigManager,
}

impl ServiceCoordinator {
    pub fn new() -> Self {
        // we're frontloading all of our configs on start so theyre kept in memory
        // reducing the need to hit the file system every iteration
        // if a repo is added, this service should just be restarted
        let config_manager = ConfigManager::new();
        let configs = config_manager.load_all_repo_configs().unwrap();
        let service_config = config_manager.load_service_config().unwrap();
        return ServiceCoordinator {
            service_config,
            repo_configs: configs,
            first_run_flag: true,
            config_manager,
        };
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
}
