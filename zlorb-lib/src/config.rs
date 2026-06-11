use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct RepositoryConfiguration {
    pub name: String,
    pub path: String,
    pub remote: String,
    pub branch: String,
    pub build_command: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RepositoriesConfigurationFile {
    #[serde(rename = "repository")]
    pub repositories: Vec<RepositoryConfiguration>,
}
