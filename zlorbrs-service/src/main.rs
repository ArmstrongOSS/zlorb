use crate::{config_manager::ConfigManager, service_coordinator::ServiceCoordinator};
use zlorbrs_lib::error::ZlorbError;

mod build_system_executor;
mod config_manager;
mod directory_scanner;
mod repo_processor;
mod service_config;
mod service_coordinator;

fn main() -> Result<(), ZlorbError> {
    colog::init();
    let config = ConfigManager::new();
    let mut service = ServiceCoordinator::new(config.load_service_config()?);
    service.run_loop()?;
    Ok(())
}
