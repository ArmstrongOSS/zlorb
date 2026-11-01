use serde::{Deserialize, Serialize};

use crate::error::ZlorbError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildStrategyKind {
    Bun,
    Rust,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepositoryConfiguration {
    pub name: String,
    pub path: String,
    pub remote: String,
    pub branch: String,
    #[serde(default)]
    pub build_strategy: Option<String>,
    #[serde(default)]
    pub build_command: String,
    #[serde(default)]
    pub restart_command: Option<String>,
    pub out_dir: Option<String>,
}

impl RepositoryConfiguration {
    pub fn resolved_restart_command(&self) -> Option<&str> {
        self.restart_command
            .as_deref()
            .map(str::trim)
            .filter(|command| !command.is_empty())
    }

    pub fn resolved_build_strategy(&self) -> Result<BuildStrategyKind, ZlorbError> {
        let accepted = "\"bun\", \"rust\", or \"cargo\" (alias for \"rust\")";
        if let Some(raw) = &self.build_strategy {
            let value = raw.trim().to_lowercase();
            if !value.is_empty() {
                return match value.as_str() {
                    "bun" => Ok(BuildStrategyKind::Bun),
                    "rust" | "cargo" => Ok(BuildStrategyKind::Rust),
                    _ => Err(ZlorbError::InvalidConfig(format!(
                        "repo '{}' has unknown build_strategy '{}'; expected {}",
                        self.name, raw, accepted
                    ))),
                };
            }
        }
        let legacy = self.build_command.trim().to_lowercase();
        match legacy.as_str() {
            "bun" => Ok(BuildStrategyKind::Bun),
            "cargo" | "cargo build" => Ok(BuildStrategyKind::Rust),
            "" => Err(ZlorbError::InvalidConfig(format!(
                "repo '{}' has no build_strategy; set build_strategy to {} in repositories.toml",
                self.name, accepted
            ))),
            _ => Err(ZlorbError::InvalidConfig(format!(
                "repo '{}' has no build_strategy and its legacy build_command '{}' does not name a strategy; set build_strategy to {} in repositories.toml",
                self.name,
                self.build_command.trim(),
                accepted
            ))),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RepositoriesConfigurationFile {
    #[serde(rename = "repository")]
    pub repositories: Vec<RepositoryConfiguration>,
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
    fn explicit_bun_selects_bun() {
        assert_eq!(
            config(Some("bun"), "").resolved_build_strategy().unwrap(),
            BuildStrategyKind::Bun
        );
    }

    #[test]
    fn explicit_rust_selects_rust() {
        assert_eq!(
            config(Some("rust"), "").resolved_build_strategy().unwrap(),
            BuildStrategyKind::Rust
        );
    }

    #[test]
    fn explicit_cargo_selects_rust() {
        assert_eq!(
            config(Some("cargo"), "").resolved_build_strategy().unwrap(),
            BuildStrategyKind::Rust
        );
    }

    #[test]
    fn explicit_strategy_wins_over_build_command() {
        assert_eq!(
            config(Some("bun"), "cargo build")
                .resolved_build_strategy()
                .unwrap(),
            BuildStrategyKind::Bun
        );
    }

    #[test]
    fn unknown_explicit_strategy_is_error() {
        let err = config(Some("webpack"), "")
            .resolved_build_strategy()
            .unwrap_err();
        assert!(format!("{:?}", err).contains("webpack"));
        assert!(format!("{:?}", err).contains("demo"));
    }

    #[test]
    fn strategy_error_messages_list_the_full_accepted_set() {
        for err in [
            config(Some("webpack"), "").resolved_build_strategy().unwrap_err(),
            config(None, "").resolved_build_strategy().unwrap_err(),
            config(None, "bunny").resolved_build_strategy().unwrap_err(),
        ] {
            let ZlorbError::InvalidConfig(message) = err else {
                panic!("expected InvalidConfig, got {:?}", err);
            };
            assert!(message.contains("\"bun\""), "missing bun in: {}", message);
            assert!(message.contains("\"rust\""), "missing rust in: {}", message);
            assert!(message.contains("\"cargo\""), "missing cargo in: {}", message);
            assert!(
                message.contains("alias for \"rust\""),
                "cargo is not named as the rust alias in: {}",
                message
            );
        }
    }

    #[test]
    fn explicit_strategy_is_case_insensitive() {
        assert_eq!(
            config(Some(" BUN "), "").resolved_build_strategy().unwrap(),
            BuildStrategyKind::Bun
        );
        assert_eq!(
            config(Some("Cargo"), "").resolved_build_strategy().unwrap(),
            BuildStrategyKind::Rust
        );
    }

    #[test]
    fn legacy_commands_follow_the_same_case_rule_as_explicit_strategies() {
        assert_eq!(
            config(None, "BUN").resolved_build_strategy().unwrap(),
            BuildStrategyKind::Bun
        );
        assert_eq!(
            config(None, "Cargo").resolved_build_strategy().unwrap(),
            BuildStrategyKind::Rust
        );
        assert_eq!(
            config(None, "  Cargo Build  ").resolved_build_strategy().unwrap(),
            BuildStrategyKind::Rust
        );
    }

    #[test]
    fn legacy_bun_command_selects_bun() {
        assert_eq!(
            config(None, "bun").resolved_build_strategy().unwrap(),
            BuildStrategyKind::Bun
        );
    }

    #[test]
    fn legacy_cargo_commands_select_rust() {
        assert_eq!(
            config(None, "cargo").resolved_build_strategy().unwrap(),
            BuildStrategyKind::Rust
        );
        assert_eq!(
            config(None, "cargo build")
                .resolved_build_strategy()
                .unwrap(),
            BuildStrategyKind::Rust
        );
    }

    #[test]
    fn missing_strategy_and_command_is_error() {
        assert!(config(None, "").resolved_build_strategy().is_err());
        assert!(config(Some(""), "").resolved_build_strategy().is_err());
        assert!(config(Some("  "), "").resolved_build_strategy().is_err());
    }

    #[test]
    fn substring_lookalikes_do_not_select_a_strategy() {
        assert!(config(None, "bunny").resolved_build_strategy().is_err());
        assert!(config(None, "cargo build --release")
            .resolved_build_strategy()
            .is_err());
        assert!(config(None, "bun run build")
            .resolved_build_strategy()
            .is_err());
    }

    #[test]
    fn toml_without_build_strategy_uses_legacy_command() {
        let parsed: RepositoryConfiguration = toml::from_str(
            "name = \"demo\"\npath = \"/tmp/demo\"\nremote = \"origin\"\nbranch = \"master\"\nbuild_command = \"bun\"\n",
        )
        .unwrap();
        assert_eq!(parsed.build_strategy, None);
        assert_eq!(
            parsed.resolved_build_strategy().unwrap(),
            BuildStrategyKind::Bun
        );
    }

    #[test]
    fn toml_entry_missing_build_command_parses_via_value() {
        let document: toml::Value = toml::from_str(
            "[[repository]]\nname = \"demo\"\npath = \"/tmp/demo\"\nremote = \"origin\"\nbranch = \"master\"\nbuild_strategy = \"bun\"\n",
        )
        .unwrap();
        let entry = document
            .get("repository")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .clone();
        let parsed: RepositoryConfiguration = entry.try_into().unwrap();
        assert_eq!(parsed.build_command, "");
        assert_eq!(
            parsed.resolved_build_strategy().unwrap(),
            BuildStrategyKind::Bun
        );
    }

    #[test]
    fn toml_with_restart_command_parses_and_resolves_it() {
        let parsed: RepositoryConfiguration = toml::from_str(
            "name = \"demo\"\npath = \"/tmp/demo\"\nremote = \"origin\"\nbranch = \"master\"\nbuild_strategy = \"rust\"\nrestart_command = \" ./deploy.sh \"\n",
        )
        .unwrap();
        assert_eq!(parsed.restart_command.as_deref(), Some(" ./deploy.sh "));
        assert_eq!(parsed.resolved_restart_command(), Some("./deploy.sh"));
    }

    #[test]
    fn missing_or_blank_restart_command_resolves_to_none() {
        let parsed: RepositoryConfiguration = toml::from_str(
            "name = \"demo\"\npath = \"/tmp/demo\"\nremote = \"origin\"\nbranch = \"master\"\nbuild_strategy = \"rust\"\n",
        )
        .unwrap();
        assert_eq!(parsed.restart_command, None);
        assert_eq!(parsed.resolved_restart_command(), None);
        let mut blank = config(None, "");
        blank.restart_command = Some("  ".to_string());
        assert_eq!(blank.resolved_restart_command(), None);
    }

    #[test]
    fn toml_with_build_strategy_parses_it() {
        let parsed: RepositoryConfiguration = toml::from_str(
            "name = \"demo\"\npath = \"/tmp/demo\"\nremote = \"origin\"\nbranch = \"master\"\nbuild_strategy = \"rust\"\nbuild_command = \"\"\n",
        )
        .unwrap();
        assert_eq!(parsed.build_strategy.as_deref(), Some("rust"));
        assert_eq!(
            parsed.resolved_build_strategy().unwrap(),
            BuildStrategyKind::Rust
        );
    }
}
