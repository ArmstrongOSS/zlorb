use crate::config_manager::ConfigManager;
use crate::repo_processor::RepoProcessor;
use crate::service_config::ServiceConfig;
use zlorb_lib::error::ZlorbError;
use zlorb_lib::log::Logger;

/// The primary orchestrator. It manages the system's global state and the continuous operation loop.
pub struct ServiceCoordinator {
    service_config: Option<ServiceConfig>,
    repo_configs: Option<Vec<RepoProcessor>>,
    first_run_flag: bool,
    config_manager: ConfigManager,
}

impl ServiceCoordinator {
    pub fn new() -> Self {
        ServiceCoordinator {
            service_config: None,
            repo_configs: None,
            first_run_flag: true,
            config_manager: ConfigManager::new(),
        }
    }

    pub fn run_loop(&mut self) -> Result<(), ZlorbError> {
        // we're frontloading all of our configs on start so theyre kept in memory
        // reducing the need to hit the file system every iteration
        // if a repo is added, this service should just be restarted
        Logger::info("---- Setting up repo and service configs".into());
        self.repo_configs = Some(self.config_manager.load_all_repo_configs()?);
        self.service_config = Some(self.config_manager.load_service_config()?);
        Logger::info("---- Starting service".into());
        loop {
            // we're throttling the loop so as to not peg the cpu at max usage
            self.wait_for_run();
            self.run_cycle()?;
        }
    }

    fn wait_for_run(&mut self) {
        if !self.first_run_flag {
            std::thread::sleep(std::time::Duration::from_secs(
                self.service_config.as_mut().unwrap().sleep_time,
            ));
        } else {
            self.first_run_flag = false
        }
    }

    fn run_cycle(&self) -> Result<(), ZlorbError> {
        let repo_configs = self.repo_configs.as_ref();
        if let Some(repos) = repo_configs {
            repos.iter().for_each(|repo| {
                Logger::info(format!("Updating repo: {}", repo.config.name));
                let _r = repo.update_from_remote();
                if let Err(e) = _r {
                    e.print();
                }
            });
            return Ok(());
        };
        Err(ZlorbError::Other(format!(
            "Repo configs MUST be set on initialization: {:?}",
            repo_configs
        )))
    }
}
