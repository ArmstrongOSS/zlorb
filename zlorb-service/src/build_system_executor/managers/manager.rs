use crate::build_system_executor::{BunManager, managers::rust::RustManager};
use zlorbrs_lib::{config::RepoConfig, error::ZlorbError};

pub trait Manager {
    fn exec(&self, config: &RepoConfig) -> Result<(), ZlorbError>;
}

pub enum BuildStrategy {
    Bun(BunManager),
    Rust(RustManager),
    None,
}
