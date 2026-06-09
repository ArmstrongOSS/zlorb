use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct ZlorbConfiguration {
    pub(crate) refresh_interval: u64
}

#[derive(Deserialize)]
pub(crate) struct RepositoryConfiguration {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) remote: String,
    pub(crate) branch: String,
    pub(crate) build_command: String
}

#[derive(Deserialize)]
pub(crate) struct Configuration {
    pub(crate) zlorb: ZlorbConfiguration,
    #[serde(rename = "repository")]
    pub(crate) repositories: Vec<RepositoryConfiguration>
}