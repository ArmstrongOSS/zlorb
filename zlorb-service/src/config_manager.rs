use crate::{repo_processor::RepoProcessor, service_config::ServiceConfig};
use std::path::PathBuf;
use zlorb_lib::config::RepositoryConfiguration;
use zlorb_lib::log::Logger;
use zlorb_lib::{create_file_with_content, error::ZlorbError, get_zlorb_config_dir};

pub struct ConfigManager {
    config_dir: PathBuf,
}

impl ConfigManager {
    pub fn new() -> Self {
        Self::with_config_dir(get_zlorb_config_dir())
    }

    pub fn with_config_dir(config_dir: PathBuf) -> Self {
        Self { config_dir }
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
        let mut processors = Vec::new();
        for repo in repos {
            if let Err(e) = repo.resolved_build_strategy() {
                Logger::error(format!(
                    "Invalid build strategy for repo {}: {:?}",
                    repo.name, e
                ));
            }
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
        let edited = repo_entry("repo", &repo_path).replace("cargo build", "cargo build --release");
        write_repos(&config_dir, &edited);
        std::fs::write(
            config_dir.join("service-config.toml"),
            "sleep_time = 7\nusername = \"u\"\ntoken = \"t\"\n",
        )
        .unwrap();
        assert_eq!(
            manager.load_all_repo_configs().unwrap()[0].config.build_command,
            "cargo build --release"
        );
        assert_eq!(manager.load_service_config().unwrap().sleep_time, 7);
    }

    #[test]
    fn entry_missing_a_field_is_skipped_and_good_entry_still_loads() {
        let root = temp_dir("skip-invalid-entry");
        let config_dir = root.join("config");
        let good = root.join("good");
        init_repo(&good);
        let bad_entry = "[[repository]]\nname = \"bad\"\npath = \"/tmp\"\nremote = \"origin\"\nbranch = \"master\"\n".to_string();
        let contents = format!("{}{}", repo_entry("good", &good), bad_entry);
        write_repos(&config_dir, &contents);
        let manager = ConfigManager::with_config_dir(config_dir);
        let repos = manager.load_all_repo_configs().unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].config.name, "good");
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
