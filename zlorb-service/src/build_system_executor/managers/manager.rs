use crate::build_system_executor::{BunManager, managers::rust::RustManager};
use zlorb_lib::{config::RepositoryConfiguration, error::ZlorbError};

pub trait Manager {
    fn exec(&self, config: &RepositoryConfiguration) -> Result<(), ZlorbError>;
}

macro_rules! build_strategies {
    ($($variant:ident => $manager:ident),+ $(,)?) => {
        pub enum BuildStrategy {
            $($variant($manager)),+
        }

        impl BuildStrategy {
            #[cfg(test)]
            pub(crate) fn all() -> Vec<Self> {
                vec![$(Self::$variant($manager)),+]
            }

            #[cfg(test)]
            pub(crate) fn kind(&self) -> zlorb_lib::config::BuildStrategyKind {
                match self {
                    $(Self::$variant(_) => zlorb_lib::config::BuildStrategyKind::$variant),+
                }
            }
        }
    };
}

build_strategies!(Bun => BunManager, Rust => RustManager);
