use crate::build_system_executor::managers::{bun::BunManager, rust::RustManager};
use zlorb_lib::{
    config::{BuildStrategyKind, RepositoryConfiguration},
    error::ZlorbError,
};

pub trait Manager {
    fn exec(&self, config: &RepositoryConfiguration) -> Result<(), ZlorbError>;
}

macro_rules! build_strategies {
    ($($variant:ident => $manager:ident),+ $(,)?) => {
        pub enum BuildStrategy {
            $($variant($manager)),+
        }

        impl BuildStrategy {
            pub(crate) fn for_kind(kind: BuildStrategyKind) -> Self {
                match kind {
                    $(BuildStrategyKind::$variant => Self::$variant($manager)),+
                }
            }

            pub(crate) fn exec(&self, config: &RepositoryConfiguration) -> Result<(), ZlorbError> {
                match self {
                    $(Self::$variant(manager) => manager.exec(config)),+
                }
            }

            #[cfg(test)]
            pub(crate) fn exec_with_runtime_path(
                &self,
                config: &RepositoryConfiguration,
                runtime: &std::path::Path,
            ) -> Result<(), ZlorbError> {
                match self {
                    $(Self::$variant(manager) => manager.exec_with_runtime_path(config, runtime)),+
                }
            }

            #[cfg(test)]
            pub(crate) fn all() -> Vec<Self> {
                vec![$(Self::$variant($manager)),+]
            }

            #[cfg(test)]
            pub(crate) fn kind(&self) -> BuildStrategyKind {
                match self {
                    $(Self::$variant(_) => BuildStrategyKind::$variant),+
                }
            }
        }
    };
}

build_strategies!(Bun => BunManager, Rust => RustManager);
