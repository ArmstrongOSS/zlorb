use crate::{repo_processor::RepoProcessor, service_config::ServiceConfig};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use zlorb_lib::config::RepositoryConfiguration;
use zlorb_lib::log::Logger;
use zlorb_lib::{create_file_with_content, error::ZlorbError, get_zlorb_config_dir};

pub struct ConfigManager {
    config_dir: PathBuf,
    reported_strategy_errors: RefCell<HashSet<String>>,
}

impl ConfigManager {
    pub fn new() -> Self {
        Self::with_config_dir(get_zlorb_config_dir())
    }

    pub fn with_config_dir(config_dir: PathBuf) -> Self {
        Self {
            config_dir,
            reported_strategy_errors: RefCell::new(HashSet::new()),
        }
    }

    fn unreported_strategy_errors(
        &self,
        repos: &[RepositoryConfiguration],
    ) -> Vec<(String, bool)> {
        let mut current = HashSet::new();
        let mut fresh = Vec::new();
        {
            let mut reported = self.reported_strategy_errors.borrow_mut();
            for repo in repos {
                if let Err(e) = repo.resolved_build_strategy() {
                    let key = format!("{}\0{:?}", repo.name, e);
                    current.insert(key.clone());
                    if reported.insert(key) {
                        let blank = repo
                            .build_strategy
                            .as_deref()
                            .map(str::trim)
                            .unwrap_or("")
                            .is_empty()
                            && repo.build_command.trim().is_empty();
                        fresh.push((
                            format!("Invalid build strategy for repo {}: {:?}", repo.name, e),
                            blank,
                        ));
                    }
                }
            }
            reported.retain(|key| current.contains(key));
        }
        fresh
    }

    fn service_config_path(&self) -> PathBuf {
        self.config_dir.join("service-config.toml")
    }

    fn repositories_path(&self) -> PathBuf {
        self.config_dir.join("repositories.toml")
    }

    pub fn initialize_default_config(&self) -> Result<String, ZlorbError> {
        let config_path = self.service_config_path();
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent).map_err(ZlorbError::Io)?;
        }
        let c = ServiceConfig::default();
        let f = toml::to_string(&c).map_err(ZlorbError::TomlSerializationError)?;
        create_file_with_content(config_path, &f)?;
        Ok(f)
    }

    pub fn load_service_config(&self) -> Result<ServiceConfig, ZlorbError> {
        let config_file_path = self.service_config_path();
        let contents = match std::fs::read_to_string(&config_file_path) {
            Ok(contents) => contents,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(ZlorbError::ConfigNotFound(config_file_path));
            }
            Err(e) => return Err(ZlorbError::Io(e)),
        };
        toml::from_str::<ServiceConfig>(&contents)
            .map_err(ZlorbError::TomlDeserializationError)
    }

    pub fn load_repo_configurations(
        &self,
    ) -> Result<Vec<RepositoryConfiguration>, ZlorbError> {
        let path = self.repositories_path();
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(ZlorbError::Io(e)),
        };
        if contents.trim().is_empty() {
            return Ok(Vec::new());
        }
        let document = toml::from_str::<toml::Value>(&contents)
            .map_err(ZlorbError::TomlDeserializationError)?;
        let entries = match document.get("repository") {
            None => return Ok(Vec::new()),
            Some(toml::Value::Array(entries)) => entries.clone(),
            Some(single) => vec![single.clone()],
        };
        let mut configs = Vec::new();
        for entry in entries {
            match entry.try_into::<RepositoryConfiguration>() {
                Ok(config) => configs.push(config),
                Err(e) => {
                    Logger::error(format!(
                        "Skipping invalid repository entry in {}: {}",
                        path.display(),
                        e
                    ));
                }
            }
        }
        Ok(configs)
    }

    pub fn load_all_repo_configs(&self) -> Result<Vec<RepoProcessor>, ZlorbError> {
        let repos = self.load_repo_configurations()?;
        for (message, blank) in self.unreported_strategy_errors(&repos) {
            if blank {
                Logger::warn(message);
            } else {
                Logger::error(message);
            }
        }
        let mut processors = Vec::new();
        for repo in repos {
            match RepoProcessor::new(repo.clone()) {
                Ok(processor) => processors.push(processor),
                Err(e) => {
                    Logger::error(format!(
                        "Skipping repo {} at {}: {:?}",
                        repo.name, repo.path, e
                    ));
                }
            }
        }
        Ok(processors)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "zlorb-config-test-{}-{}-{}",
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

    #[test]
    fn missing_repositories_file_is_empty_not_error() {
        let config_dir = temp_dir("missing");
        let manager = ConfigManager::with_config_dir(config_dir);
        let repos = manager.load_all_repo_configs().unwrap();
        assert!(repos.is_empty());
    }

    #[test]
    fn malformed_repositories_file_is_error() {
        let config_dir = temp_dir("malformed");
        write_repos(&config_dir, "this is not = valid = toml [[");
        let manager = ConfigManager::with_config_dir(config_dir);
        assert!(manager.load_all_repo_configs().is_err());
        assert!(manager.load_repo_configurations().is_err());
    }

    #[test]
    fn bad_repo_entry_is_skipped_and_good_repo_still_loads() {
        let root = temp_dir("skip-bad");
        let config_dir = root.join("config");
        let good = root.join("good");
        init_repo(&good);
        let bad_path = root.join("does-not-exist");
        let contents = format!(
            "{}{}",
            repo_entry("good", &good),
            repo_entry("bad", &bad_path)
        );
        write_repos(&config_dir, &contents);
        let manager = ConfigManager::with_config_dir(config_dir);
        let repos = manager.load_all_repo_configs().unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].config.name, "good");
    }

    #[test]
    fn reload_picks_up_repos_added_after_first_load() {
        let root = temp_dir("reload-add");
        let config_dir = root.join("config");
        let first = root.join("first");
        let second = root.join("second");
        init_repo(&first);
        init_repo(&second);
        write_repos(&config_dir, &repo_entry("first", &first));
        let manager = ConfigManager::with_config_dir(config_dir.clone());
        assert_eq!(manager.load_all_repo_configs().unwrap().len(), 1);
        let contents = format!(
            "{}{}",
            repo_entry("first", &first),
            repo_entry("second", &second)
        );
        write_repos(&config_dir, &contents);
        let reloaded = manager.load_all_repo_configs().unwrap();
        assert_eq!(reloaded.len(), 2);
        assert_eq!(reloaded[1].config.name, "second");
    }

    #[test]
    fn reload_picks_up_edited_repo_and_service_config() {
        let root = temp_dir("reload-edit");
        let config_dir = root.join("config");
        let repo_path = root.join("repo");
        init_repo(&repo_path);
        write_repos(&config_dir, &repo_entry("repo", &repo_path));
        std::fs::write(
            config_dir.join("service-config.toml"),
            "sleep_time = 42\nusername = \"u\"\ntoken = \"t\"\n",
        )
        .unwrap();
        let manager = ConfigManager::with_config_dir(config_dir.clone());
        assert_eq!(manager.load_service_config().unwrap().sleep_time, 42);
        let before = manager.load_all_repo_configs().unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].config.build_command, "cargo build");
        assert_eq!(before[0].config.build_strategy, None);
        let edited = repo_entry("repo", &repo_path).replace(
            "build_command = \"cargo build\"",
            "build_strategy = \"rust\"\nbuild_command = \"cargo build --release\"",
        );
        write_repos(&config_dir, &edited);
        std::fs::write(
            config_dir.join("service-config.toml"),
            "sleep_time = 7\nusername = \"u\"\ntoken = \"t\"\n",
        )
        .unwrap();
        let reloaded = manager.load_all_repo_configs().unwrap();
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded[0].config.build_command, "cargo build --release");
        assert_eq!(reloaded[0].config.build_strategy.as_deref(), Some("rust"));
        assert_eq!(
            reloaded[0].config.resolved_build_strategy().unwrap(),
            zlorb_lib::config::BuildStrategyKind::Rust
        );
        assert_eq!(manager.load_service_config().unwrap().sleep_time, 7);
    }

    #[test]
    fn entry_missing_a_field_is_skipped_and_good_entry_still_loads() {
        let root = temp_dir("skip-invalid-entry");
        let config_dir = root.join("config");
        let good = root.join("good");
        init_repo(&good);
        let bad = root.join("bad");
        init_repo(&bad);
        let bad_entry = format!(
            "[[repository]]\nname = \"bad\"\npath = \"{}\"\nremote = \"origin\"\nbuild_command = \"cargo build\"\n",
            bad.display()
        );
        let contents = format!("{}{}", repo_entry("good", &good), bad_entry);
        write_repos(&config_dir, &contents);
        let manager = ConfigManager::with_config_dir(config_dir);
        let parsed = manager.load_repo_configurations().unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "good");
        let repos = manager.load_all_repo_configs().unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].config.name, "good");
    }

    fn repo_config(
        name: &str,
        build_strategy: Option<&str>,
        build_command: &str,
    ) -> RepositoryConfiguration {
        RepositoryConfiguration {
            name: name.to_string(),
            path: "/tmp/demo".to_string(),
            remote: "origin".to_string(),
            branch: "master".to_string(),
            build_strategy: build_strategy.map(|s| s.to_string()),
            build_command: build_command.to_string(),
            restart_command: None,
            out_dir: None,
        }
    }

    #[test]
    fn strategy_error_is_reported_once_across_reloads() {
        let manager = ConfigManager::with_config_dir(temp_dir("dedupe-once"));
        let repos = vec![repo_config("demo", Some("webpack"), "")];
        assert_eq!(manager.unreported_strategy_errors(&repos).len(), 1);
        assert!(manager.unreported_strategy_errors(&repos).is_empty());
        assert!(manager.unreported_strategy_errors(&repos).is_empty());
    }

    #[test]
    fn changed_strategy_error_is_reported_again() {
        let manager = ConfigManager::with_config_dir(temp_dir("dedupe-changed"));
        let first = vec![repo_config("demo", Some("webpack"), "")];
        assert_eq!(manager.unreported_strategy_errors(&first).len(), 1);
        let second = vec![repo_config("demo", Some("parcel"), "")];
        let fresh = manager.unreported_strategy_errors(&second);
        assert_eq!(fresh.len(), 1);
        assert!(fresh[0].0.contains("parcel"));
    }

    #[test]
    fn fixed_then_broken_strategy_error_is_reported_again() {
        let manager = ConfigManager::with_config_dir(temp_dir("dedupe-fixed"));
        let broken = vec![repo_config("demo", Some("webpack"), "")];
        assert_eq!(manager.unreported_strategy_errors(&broken).len(), 1);
        let fixed = vec![repo_config("demo", Some("rust"), "")];
        assert!(manager.unreported_strategy_errors(&fixed).is_empty());
        assert_eq!(manager.unreported_strategy_errors(&broken).len(), 1);
    }

    #[test]
    fn removed_then_readded_strategy_error_is_reported_again() {
        let manager = ConfigManager::with_config_dir(temp_dir("dedupe-removed"));
        let broken = vec![repo_config("demo", Some("webpack"), "")];
        assert_eq!(manager.unreported_strategy_errors(&broken).len(), 1);
        assert!(manager.unreported_strategy_errors(&[]).is_empty());
        assert_eq!(manager.unreported_strategy_errors(&broken).len(), 1);
    }

    #[test]
    fn blank_ctl_state_is_flagged_blank_and_unknown_strategy_is_not() {
        let manager = ConfigManager::with_config_dir(temp_dir("dedupe-blank"));
        let blank = vec![repo_config("fresh", None, "")];
        let fresh = manager.unreported_strategy_errors(&blank);
        assert_eq!(fresh.len(), 1);
        assert!(fresh[0].1);
        let unknown = vec![repo_config("demo", Some("webpack"), "")];
        let fresh = manager.unreported_strategy_errors(&unknown);
        assert_eq!(fresh.len(), 1);
        assert!(!fresh[0].1);
        let legacy = vec![repo_config("old", None, "bunny")];
        let fresh = manager.unreported_strategy_errors(&legacy);
        assert_eq!(fresh.len(), 1);
        assert!(!fresh[0].1);
    }

    #[test]
    fn valid_repos_have_no_strategy_errors_to_report() {
        let manager = ConfigManager::with_config_dir(temp_dir("dedupe-valid"));
        let repos = vec![
            repo_config("a", Some("bun"), ""),
            repo_config("b", None, "cargo build"),
        ];
        assert!(manager.unreported_strategy_errors(&repos).is_empty());
    }

    #[test]
    fn comment_only_repositories_file_is_empty_not_error() {
        let config_dir = temp_dir("comment-only");
        write_repos(&config_dir, "# no repos yet\n");
        let manager = ConfigManager::with_config_dir(config_dir);
        assert!(manager.load_all_repo_configs().unwrap().is_empty());
    }

    #[test]
    fn missing_service_config_is_error_not_rewritten() {
        let config_dir = temp_dir("missing-service");
        std::fs::create_dir_all(&config_dir).unwrap();
        let manager = ConfigManager::with_config_dir(config_dir.clone());
        assert!(manager.load_service_config().is_err());
        assert!(!config_dir.join("service-config.toml").exists());
    }

    #[test]
    fn malformed_service_config_is_error() {
        let config_dir = temp_dir("bad-service");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("service-config.toml"), "sleep_time = \"nope\"\n").unwrap();
        let manager = ConfigManager::with_config_dir(config_dir);
        assert!(manager.load_service_config().is_err());
    }
}
