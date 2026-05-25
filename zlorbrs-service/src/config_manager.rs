use std::path::PathBuf;
use zlorbrs_lib::{
    create_file_with_content, error::ZlorbError, get_home_dir, read_file_from_filesystem,
};

use crate::service_config::ServiceConfig;

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

    pub fn load_repo_config(&self, name: String) -> String {
        let path = self
            .home_dir
            .join(format!(".config/zlorbrs/configs/{}/config.json", name));
        read_file_from_filesystem(path).unwrap()
    }
}
