use crate::config_manager::ConfigManager;
use crate::repo_processor::RepoProcessor;
use crate::service_config::ServiceConfig;
use zlorbrs_lib::error::ZlorbError;
use zlorbrs_lib::log::Logger;

/// The primary orchestrator. It manages the system's global state and the continuous operation loop.
pub struct ServiceCoordinator {
    service_config: Option<ServiceConfig>,
    repo_configs: Option<Vec<RepoProcessor>>,
    first_run_flag: bool,
    config_manager: Option<ConfigManager>,
}

impl ServiceCoordinator {
    pub fn new() -> Self {
        return ServiceCoordinator {
            service_config: None,
            repo_configs: None,
            first_run_flag: true,
            config_manager: None,
        };
    }

    pub fn run_loop(&mut self) -> Result<(), ZlorbError> {
        // we're frontloading all of our configs on start so theyre kept in memory
        // reducing the need to hit the file system every iteration
        // if a repo is added, this service should just be restarted
        self.setup_config_manager()?;
        self.setup_repo_configs()?;
        self.setup_service_config()?;

        Logger::info("Starting service".into());
        Logger::info(format!("Loaded repos: {:?}", self.repo_configs));
        loop {
            // we're throttling the loop so as to not peg
            // the cpu at max usage
            self.wait_for_run();
            self.run_cycle()?;
        }
    }

    fn setup_config_manager(&mut self) -> Result<(), ZlorbError> {
        self.config_manager = Some(ConfigManager::new());
        Ok(())
    }

    fn setup_repo_configs(&mut self) -> Result<(), ZlorbError> {
        let cm = self.config_manager.as_mut();
        if cm.is_none() {
            return Err(ZlorbError::ConfigParseError(
                "Config manager hasnt been set yet on service coordinator".into(),
            ));
        }

        if let Some(cm) = cm {
            let rc = cm.load_all_repo_configs()?;
            self.repo_configs = Some(rc);
        }
        Ok(())
    }

    fn setup_service_config(&mut self) -> Result<(), ZlorbError> {
        let cm = self.config_manager.as_mut();
        if cm.is_none() {
            return Err(ZlorbError::ConfigParseError(
                "Config manager hasnt been set yet on service coordinator".into(),
            ));
        }
        if let Some(cm) = cm {
            let sc = cm.load_service_config()?;
            self.service_config = Some(sc);
        }
        Ok(())
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
        let repos = self.repo_configs.as_ref().unwrap();
        repos.iter().for_each(|repo| {
            Logger::info(format!("Updating repo: {}", repo.config.name));
            let _r = repo.update_from_remote();
        });
        Ok(())
    }
}
