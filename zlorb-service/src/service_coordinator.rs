use crate::config_manager::ConfigManager;
use crate::repo_processor::{FailedBuilds, RepoProcessor, new_failed_builds};
use crate::service_config::ServiceConfig;
use zlorb_lib::error::ZlorbError;
use zlorb_lib::log::Logger;

pub struct ServiceCoordinator {
    service_config: ServiceConfig,
    service_config_loaded: bool,
    repo_configs: Vec<RepoProcessor>,
    first_run_flag: bool,
    config_manager: ConfigManager,
    failed_builds: FailedBuilds,
}

impl ServiceCoordinator {
    pub fn new() -> Self {
        Self::with_config_manager(ConfigManager::new())
    }

    pub fn with_config_manager(config_manager: ConfigManager) -> Self {
        ServiceCoordinator {
            service_config: ServiceConfig::default(),
            service_config_loaded: false,
            repo_configs: Vec::new(),
            first_run_flag: true,
            config_manager,
            failed_builds: new_failed_builds(),
        }
    }

    pub fn reload_configs(&mut self) {
        match self.config_manager.load_service_config() {
            Ok(config) => {
                self.service_config = config;
                self.service_config_loaded = true;
            }
            Err(ZlorbError::ConfigNotFound(_)) if !self.service_config_loaded => {
                match self.config_manager.initialize_default_config() {
                    Ok(_) => {
                        if let Ok(config) = self.config_manager.load_service_config() {
                            self.service_config = config;
                            self.service_config_loaded = true;
                        }
                    }
                    Err(e) => Logger::error(format!(
                        "Failed to create default service config, keeping defaults: {:?}",
                        e
                    )),
                }
            }
            Err(e) => Logger::error(format!(
                "Failed to load service config, keeping previous config: {:?}",
                e
            )),
        }
        match self.config_manager.load_all_repo_configs() {
            Ok(mut repos) => {
                for repo in &mut repos {
                    repo.failed_builds = self.failed_builds.clone();
                }
                self.repo_configs = repos;
            }
            Err(e) => Logger::error(format!(
                "Failed to load repository configs, keeping previous repos: {:?}",
                e
            )),
        }
    }

    pub fn run_loop(&mut self) -> Result<(), ZlorbError> {
        Logger::info("---- Starting service".into());
        loop {
            Logger::info("---- loop".into());
            self.reload_configs();
            self.wait_for_run();
            self.run_cycle();
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

    fn run_cycle(&self) {
        Logger::info(format!("repos: {:#?}", self.repo_configs));
        for repo in &self.repo_configs {
            Logger::info(format!("Updating repo: {}", repo.config.name));
            match repo.update_from_remote() {
                Ok(_) => Logger::info(format!("Successfully updated {}", repo.config.name)),
                Err(e) => {
                    Logger::error(format!("Failed to update {}: {:?}. This may be a configuration issue for this branch; ensure the details in ~/.config/zlorb/repositories.toml are accurate.", repo.config.name, e));
                    e.print();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "zlorb-coordinator-test-{}-{}-{}",
            std::process::id(),
            label,
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn init_repo(path: &Path) {
        std::fs::create_dir_all(path).unwrap();
        git2::Repository::init(path).unwrap();
    }

    fn repo_entry(name: &str, path: &Path) -> String {
        format!(
            "[[repository]]\nname = \"{}\"\npath = \"{}\"\nremote = \"origin\"\nbranch = \"master\"\nbuild_command = \"cargo build\"\n",
            name,
            path.display()
        )
    }

    fn write_repos(config_dir: &Path, contents: &str) {
        std::fs::create_dir_all(config_dir).unwrap();
        std::fs::write(config_dir.join("repositories.toml"), contents).unwrap();
    }

    fn write_service_config(config_dir: &Path, sleep_time: u64) {
        std::fs::create_dir_all(config_dir).unwrap();
        std::fs::write(
            config_dir.join("service-config.toml"),
            format!(
                "sleep_time = {}\nusername = \"u\"\ntoken = \"t\"\n",
                sleep_time
            ),
        )
        .unwrap();
    }

    #[test]
    fn reload_picks_up_repo_added_without_restart() {
        let root = temp_dir("add");
        let config_dir = root.join("config");
        let first = root.join("first");
        let second = root.join("second");
        init_repo(&first);
        init_repo(&second);
        write_repos(&config_dir, &repo_entry("first", &first));
        write_service_config(&config_dir, 5);
        let mut coordinator =
            ServiceCoordinator::with_config_manager(ConfigManager::with_config_dir(
                config_dir.clone(),
            ));
        coordinator.reload_configs();
        assert_eq!(coordinator.repo_configs.len(), 1);
        let contents = format!(
            "{}{}",
            repo_entry("first", &first),
            repo_entry("second", &second)
        );
        write_repos(&config_dir, &contents);
        coordinator.reload_configs();
        assert_eq!(coordinator.repo_configs.len(), 2);
        assert_eq!(coordinator.repo_configs[1].config.name, "second");
    }

    #[test]
    fn reload_applies_service_config_edit_without_restart() {
        let root = temp_dir("service-edit");
        let config_dir = root.join("config");
        write_repos(&config_dir, "");
        write_service_config(&config_dir, 42);
        let mut coordinator =
            ServiceCoordinator::with_config_manager(ConfigManager::with_config_dir(
                config_dir.clone(),
            ));
        coordinator.reload_configs();
        assert_eq!(coordinator.service_config.sleep_time, 42);
        write_service_config(&config_dir, 9);
        coordinator.reload_configs();
        assert_eq!(coordinator.service_config.sleep_time, 9);
    }

    #[test]
    fn malformed_repo_edit_keeps_previous_repos() {
        let root = temp_dir("malformed-keeps");
        let config_dir = root.join("config");
        let repo_path = root.join("repo");
        init_repo(&repo_path);
        write_repos(&config_dir, &repo_entry("repo", &repo_path));
        write_service_config(&config_dir, 5);
        let mut coordinator =
            ServiceCoordinator::with_config_manager(ConfigManager::with_config_dir(
                config_dir.clone(),
            ));
        coordinator.reload_configs();
        assert_eq!(coordinator.repo_configs.len(), 1);
        write_repos(&config_dir, "this is not = valid = toml [[");
        coordinator.reload_configs();
        assert_eq!(coordinator.repo_configs.len(), 1);
        assert_eq!(coordinator.repo_configs[0].config.name, "repo");
    }

    #[test]
    fn reload_picks_up_repo_removed_without_restart() {
        let root = temp_dir("remove");
        let config_dir = root.join("config");
        let first = root.join("first");
        let second = root.join("second");
        init_repo(&first);
        init_repo(&second);
        let contents = format!(
            "{}{}",
            repo_entry("first", &first),
            repo_entry("second", &second)
        );
        write_repos(&config_dir, &contents);
        write_service_config(&config_dir, 5);
        let mut coordinator =
            ServiceCoordinator::with_config_manager(ConfigManager::with_config_dir(
                config_dir.clone(),
            ));
        coordinator.reload_configs();
        assert_eq!(coordinator.repo_configs.len(), 2);
        write_repos(&config_dir, &repo_entry("second", &second));
        coordinator.reload_configs();
        assert_eq!(coordinator.repo_configs.len(), 1);
        assert_eq!(coordinator.repo_configs[0].config.name, "second");
    }

    #[test]
    fn missing_service_config_after_load_keeps_previous_and_is_not_recreated() {
        let root = temp_dir("missing-service-keeps");
        let config_dir = root.join("config");
        write_repos(&config_dir, "");
        write_service_config(&config_dir, 21);
        let mut coordinator =
            ServiceCoordinator::with_config_manager(ConfigManager::with_config_dir(
                config_dir.clone(),
            ));
        coordinator.reload_configs();
        assert_eq!(coordinator.service_config.sleep_time, 21);
        std::fs::remove_file(config_dir.join("service-config.toml")).unwrap();
        coordinator.reload_configs();
        assert_eq!(coordinator.service_config.sleep_time, 21);
        assert!(!config_dir.join("service-config.toml").exists());
    }

    #[test]
    fn first_reload_creates_default_service_config_when_missing() {
        let root = temp_dir("first-default");
        let config_dir = root.join("config");
        write_repos(&config_dir, "");
        let mut coordinator =
            ServiceCoordinator::with_config_manager(ConfigManager::with_config_dir(
                config_dir.clone(),
            ));
        coordinator.reload_configs();
        assert_eq!(
            coordinator.service_config.sleep_time,
            ServiceConfig::default().sleep_time
        );
        assert!(config_dir.join("service-config.toml").exists());
    }

    #[test]
    fn malformed_service_edit_keeps_previous_service_config() {
        let root = temp_dir("malformed-service");
        let config_dir = root.join("config");
        write_repos(&config_dir, "");
        write_service_config(&config_dir, 33);
        let mut coordinator =
            ServiceCoordinator::with_config_manager(ConfigManager::with_config_dir(
                config_dir.clone(),
            ));
        coordinator.reload_configs();
        assert_eq!(coordinator.service_config.sleep_time, 33);
        std::fs::write(config_dir.join("service-config.toml"), "sleep_time = \"nope\"\n")
            .unwrap();
        coordinator.reload_configs();
        assert_eq!(coordinator.service_config.sleep_time, 33);
    }

    #[test]
    fn reload_shares_one_failed_builds_tracker_across_processors() {
        let root = temp_dir("tracker-shared");
        let config_dir = root.join("config");
        let repo_path = root.join("repo");
        init_repo(&repo_path);
        write_repos(&config_dir, &repo_entry("repo", &repo_path));
        write_service_config(&config_dir, 5);
        let mut coordinator =
            ServiceCoordinator::with_config_manager(ConfigManager::with_config_dir(
                config_dir.clone(),
            ));
        coordinator.reload_configs();
        assert_eq!(coordinator.repo_configs.len(), 1);
        assert!(std::sync::Arc::ptr_eq(
            &coordinator.repo_configs[0].failed_builds,
            &coordinator.failed_builds
        ));
        let first_tracker = coordinator.repo_configs[0].failed_builds.clone();
        coordinator.reload_configs();
        assert_eq!(coordinator.repo_configs.len(), 1);
        assert!(std::sync::Arc::ptr_eq(
            &coordinator.repo_configs[0].failed_builds,
            &first_tracker
        ));
    }
}
