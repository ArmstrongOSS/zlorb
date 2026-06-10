use crate::{repo_processor::RepoProcessor, service_config::ServiceConfig};
use std::fs;
use std::{fs::ReadDir, path::PathBuf};
use zlorb_lib::{
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
        let config_path = self.home_dir.join(".config/zlorb/service-config.json");
        let c = ServiceConfig::default();
        let f = serde_json::to_string(&c)
            .map_err(|e| ZlorbError::SerializationErrorGeneric(e.to_string()))?;
        create_file_with_content(config_path, &f)?;
        Ok(f)
    }

    pub fn initialize_repo_configs(&self) -> Result<ReadDir, ZlorbError> {
        let p = self.home_dir.join(".config/zlorb/configs");
        fs::create_dir_all(&p).map_err(ZlorbError::Io)?;
        fs::read_dir(p).map_err(ZlorbError::Io)
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
        let configs_dir_path = get_home_dir().join(".config/zlorb/configs");

        // metadata checks for file/folder metadata and essentially can be used
        // to determine if something exists on the filesystem
        if fs::metadata(&configs_dir_path).is_err() {
            self.initialize_repo_configs()?;
        }

        let configs_dir = fs::read_dir(&configs_dir_path)
            .map_err(ZlorbError::Io)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(ZlorbError::Io)?;

        configs_dir
            .into_iter()
            .map(|dir| {
                let p = dir.path().join("config.json");
                let file_contents = read_file_from_filesystem(p).unwrap();
                let repo: RepoConfig = serde_json::from_str(&file_contents).map_err(|e| {
                    ZlorbError::ConfigParseError(format!(
                        "Parsing failed for {:?}: {}",
                        dir.path(),
                        e
                    ))
                })?;
                Ok(RepoProcessor::new(repo))
            })
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
