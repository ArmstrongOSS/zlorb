use crate::{repo_processor::RepoProcessor, service_config::ServiceConfig};
use std::path::PathBuf;
use zlorb_lib::create_config_from_toml;
use zlorb_lib::{
    create_file_with_content, error::ZlorbError, get_home_dir, read_file_from_filesystem,
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
        let config_path = self.home_dir.join(".config/zlorb/service-config.json");
        let c = ServiceConfig::default();
        let f = serde_json::to_string(&c)
            .map_err(|e| ZlorbError::SerializationErrorGeneric(e.to_string()))?;
        create_file_with_content(config_path, &f)?;
        Ok(f)
    }

    pub fn load_service_config(&self) -> Result<ServiceConfig, ZlorbError> {
        let config_file_path = self.home_dir.join(".config/zlorb/service-config.json");

        let opened: Result<String, ZlorbError> =
            match read_file_from_filesystem(config_file_path.clone()) {
                Some(conf) => Ok(conf),
                None => match self.initialize_default_config() {
                    Ok(new_conf) => Ok(new_conf),
                    Err(e) => Err(e),
                },
            };

        if let Err(opened_err) = opened {
            return Err(opened_err);
        }
        
        serde_json::from_str::<ServiceConfig>(&opened.unwrap())
            .map_err(ZlorbError::SerializationError)
    }

    pub fn load_all_repo_configs(&self) -> Result<Vec<RepoProcessor>, ZlorbError> {
        // we need to parse the toml file to data structure
        let (config, _file) = create_config_from_toml()?;

        config
            .repositories
            .into_iter()
            .map(|repo| Ok(RepoProcessor::new(repo)))
            .collect::<Result<Vec<RepoProcessor>, ZlorbError>>()
    }

    pub fn _load_repo_config(&self, name: String) -> Result<String, ZlorbError> {
        let path = self
            .home_dir
            .join(".config/zlorb/configs")
            .join(name)
            .join("config.json");
        Ok(read_file_from_filesystem(path).unwrap())
    }
}
