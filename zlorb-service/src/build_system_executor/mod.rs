mod managers;

use crate::{
    build_system_executor::managers::{
        bun::BunManager,
        manager::{BuildStrategy, Manager},
        rust::RustManager,
    },
    repo_processor::RepoProcessor,
};
use zlorb_lib::{
    config::{BuildStrategyKind, RepositoryConfiguration},
    error::ZlorbError,
};

pub struct BuildSystemExecutor<'a> {
    pub processor: &'a RepoProcessor,
}

pub fn select_strategy(
    config: &RepositoryConfiguration,
) -> Result<BuildStrategy, ZlorbError> {
    match config.resolved_build_strategy()? {
        BuildStrategyKind::Bun => Ok(BuildStrategy::Bun(BunManager {})),
        BuildStrategyKind::Rust => Ok(BuildStrategy::Rust(RustManager {})),
    }
}

impl<'a> BuildSystemExecutor<'a> {
    pub fn run_build(&self) -> Result<(), ZlorbError> {
        let config = &self.processor.config;
        match select_strategy(config)? {
            BuildStrategy::Bun(manager) => manager.exec(config),
            BuildStrategy::Rust(manager) => manager.exec(config),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(build_strategy: Option<&str>, build_command: &str) -> RepositoryConfiguration {
        RepositoryConfiguration {
            name: "demo".to_string(),
            path: "/tmp/demo".to_string(),
            remote: "origin".to_string(),
            branch: "master".to_string(),
            build_strategy: build_strategy.map(|s| s.to_string()),
            build_command: build_command.to_string(),
            restart_command: None,
            out_dir: None,
        }
    }

    #[test]
    fn bun_strategy_selects_bun_manager() {
        assert!(matches!(
            select_strategy(&config(Some("bun"), "")).unwrap(),
            BuildStrategy::Bun(_)
        ));
    }

    #[test]
    fn cargo_strategy_selects_rust_manager() {
        assert!(matches!(
            select_strategy(&config(Some("cargo"), "")).unwrap(),
            BuildStrategy::Rust(_)
        ));
        assert!(matches!(
            select_strategy(&config(None, "cargo build")).unwrap(),
            BuildStrategy::Rust(_)
        ));
    }

    #[test]
    fn unknown_strategy_is_error() {
        assert!(select_strategy(&config(Some("webpack"), "")).is_err());
    }

    #[test]
    fn missing_strategy_is_error() {
        assert!(select_strategy(&config(None, "")).is_err());
    }
}
