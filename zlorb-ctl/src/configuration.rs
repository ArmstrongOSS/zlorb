use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(crate) struct RepositoryConfiguration {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) remote: String,
    pub(crate) branch: String,
    pub(crate) build_command: String,
}

#[derive(Deserialize)]
struct RepositoriesConfigurationFile {
    #[serde(rename = "repository")]
    repositories: Vec<RepositoryConfiguration>,
}
