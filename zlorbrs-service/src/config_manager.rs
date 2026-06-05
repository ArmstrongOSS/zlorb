use crate::{repo_processor::RepoProcessor, service_config::ServiceConfig};
use std::{fs::ReadDir, path::PathBuf};
use zlorbrs_lib::log::Logger;
use zlorbrs_lib::{
    config::RepoConfig, create_file_with_content, error::ZlorbError, get_home_dir,
    read_file_from_filesystem,
};

#[derive(Default)]
pub struct ConfigManager {
    home_dir: PathBuf,
}

impl ConfigManager {
    pub fn new() -> Self {
        Self {
            home_dir: get_home_dir(),
        }
    }
    pub fn initialize_default_config(&self) -> Result<String, ZlorbError> {
        let p = self.home_dir.join("/.config/zlorbrs/service-config.json");
        let c = ServiceConfig::default();
        let f =
            serde_json::to_string(&c).map_err(|e| ZlorbError::SerializationError(e.to_string()))?;
        create_file_with_content(p, &f)?;
        Ok(f)
    }

    pub fn initialize_repo_configs(&self) -> Result<ReadDir, ZlorbError> {
        let p = self.home_dir.join("/.config/zlorbrs/configs");
        std::fs::create_dir_all(&p).map_err(|e| ZlorbError::Io(e))?;
        std::fs::read_dir(p).map_err(|e| ZlorbError::Io(e))
    }

    pub fn load_service_config(&self) -> Result<ServiceConfig, ZlorbError> {
        let config_file_path = self.home_dir.join(".config/zlorbrs/service-config.json");
        let config_file = read_file_from_filesystem(config_file_path.clone());
        let opened: String;
        if config_file.is_none() {
            opened = self.initialize_default_config()?;
        } else {
            opened = config_file.unwrap();
        }
        serde_json::from_str::<ServiceConfig>(&opened).map_err(|_| {
            ZlorbError::ConfigParseError("Failed to convert config file to json string".to_string())
        })
    }

    pub fn load_all_repo_configs(&self) -> Result<Vec<RepoProcessor>, ZlorbError> {
        let configs_dir_path = get_home_dir().join(".config/zlorbrs/configs");
        let config_dir_exists =
            std::fs::exists(&configs_dir_path).map_err(|e| ZlorbError::Io(e))?;
        if !config_dir_exists {
            self.initialize_repo_configs()?;
        }
        let configs_dir = std::fs::read_dir(&configs_dir_path)
            .map_err(|_| ZlorbError::ConfigNotFound(configs_dir_path))?;
        Logger::info(format!("Loaded configs: {:?}", configs_dir));
        let configs = configs_dir
            .map(|d| {
                Logger::info(format!("Loading config: {:?}", d));
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

    pub fn load_repo_config(&self, name: String) -> String {
        let path = self
            .home_dir
            .join(format!(".config/zlorbrs/configs/{}/config.json", name));
        read_file_from_filesystem(path).unwrap()
    }
}
