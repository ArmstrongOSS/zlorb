use crate::service_coordinator::ServiceCoordinator;
use zlorb_lib::error::ZlorbError;

mod build_system_executor;
mod config_manager;
mod repo_processor;
mod service_config;
mod service_coordinator;

fn main() -> Result<(), ZlorbError> {
    colog::init();
    println!("Testing webhook");
    ServiceCoordinator::new().run_loop()
}
