mod managers;

use crate::{
    build_system_executor::managers::{
        bun::BunManager,
        manager::{BuildStrategy, Manager},
        rust::RustManager,
    },
    repo_processor::RepoProcessor,
};
use zlorb_lib::error::ZlorbError;

///  An isolated utility service responsible for running external shell commands (the build step).
pub struct BuildSystemExecutor<'a> {
    pub processor: &'a RepoProcessor,
}

impl<'a> BuildSystemExecutor<'a> {
    pub fn run_build(&self) -> Result<(), ZlorbError> {
        let config = &self.processor.config;
        match self.determine_strategy() {
            BuildStrategy::Bun(bun_manager) => bun_manager.exec(config),
            BuildStrategy::Rust(rust_manager) => rust_manager.exec(config),
            BuildStrategy::None => Err(ZlorbError::InvalidConfig(format!(
                "The build command '{}' provided to repo: {} was invalid",
                config.build_command, config.name
            ))),
        }
    }

    fn determine_strategy(&self) -> BuildStrategy {
        // this is fragile and prone to error so TODO fix this shit
        let build_command = Some(self.processor.config.build_command.clone());
        match build_command {
            Some(x) if x.contains("bun") => BuildStrategy::Bun(BunManager {}),
            Some(x) if x.contains("cargo") => BuildStrategy::Rust(RustManager {}),
            _ => BuildStrategy::None,
        }
    }
}
