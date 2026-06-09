use std::fs;

use crate::service_coordinator::ServiceCoordinator;
use zlorbrs_lib::error::ZlorbError;

mod build_system_executor;
mod config_manager;
mod configuration;
mod repo_processor;
mod service_config;
mod service_coordinator;

fn main() -> Result<(), ZlorbError> {
    // colog::init();
    
    // TODO: you can do the error handling
    let config_path = std::path::Path::new("res/config.toml");
    let config_content = fs::read_to_string(config_path).unwrap();
    let config: configuration::Configuration = toml::from_str(&config_content).unwrap();

    println!("Configuration successfully loaded!");
    println!("Refresh interval: {} seconds", config.zlorb.refresh_interval);
    println!("--- TRACKED REPOSITORIES ---");
    for repository in config.repositories {
        println!("[{}] {}/{} in {}", repository.name, repository.remote, repository.branch, repository.path);
    }

    ServiceCoordinator::new().run_loop()
}
