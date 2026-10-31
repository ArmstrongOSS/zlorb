use std::path::{Path, PathBuf};
use std::process::Command;
use zlorb_lib::{error::ZlorbError, get_home_dir, get_zlorb_config_dir, is_root, log::Logger};

fn escape_unit_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "%%")
        .replace('"', "\\\"")
}

pub fn render_unit(service_bin: &Path, config_dir: &Path) -> String {
    format!(
        "[Unit]\nDescription=Zlorb Daemon\nAfter=network.target\n\n[Service]\nExecStart=\"{}\"\nType=simple\nRestart=always\nEnvironment=\"RUST_LOG=zlorb_service\"\nEnvironment=\"ZLORB_CONFIG_DIR={}\"\n\n[Install]\nWantedBy=default.target\n",
        escape_unit_value(&service_bin.display().to_string()),
        escape_unit_value(&config_dir.display().to_string())
    )
}

pub fn user_unit_path_from(xdg_config_home: Option<&str>, home_dir: &Path) -> PathBuf {
    let config_home = match xdg_config_home {
        Some(dir) if Path::new(dir.trim()).is_absolute() && !dir.trim().is_empty() => {
            PathBuf::from(dir.trim())
        }
        _ => home_dir.join(".config"),
    };
    config_home
        .join("systemd")
        .join("user")
        .join("zlorb.service")
}

pub fn user_unit_path(home_dir: &Path) -> PathBuf {
    user_unit_path_from(std::env::var("XDG_CONFIG_HOME").ok().as_deref(), home_dir)
}

pub fn system_unit_path() -> PathBuf {
    PathBuf::from("/etc/systemd/system/zlorb.service")
}

pub fn unit_path_for(is_root_user: bool, home_dir: &Path) -> PathBuf {
    if is_root_user {
        system_unit_path()
    } else {
        user_unit_path(home_dir)
    }
}

pub fn resolve_service_binary_from(
    current_exe: &Path,
    cargo_home: Option<&str>,
    home_dir: &Path,
) -> PathBuf {
    if let Some(dir) = current_exe.parent() {
        let sibling = dir.join("zlorb-service");
        if sibling.exists() {
            return sibling;
        }
    }
    match cargo_home {
        Some(dir) if !dir.trim().is_empty() => {
            PathBuf::from(dir.trim()).join("bin").join("zlorb-service")
        }
        _ => home_dir.join(".cargo").join("bin").join("zlorb-service"),
    }
}

pub fn resolve_service_binary(current_exe: &Path, home_dir: &Path) -> PathBuf {
    resolve_service_binary_from(
        current_exe,
        std::env::var("CARGO_HOME").ok().as_deref(),
        home_dir,
    )
}

pub fn ensure_config(config_dir: &Path) -> Result<(), ZlorbError> {
    std::fs::create_dir_all(config_dir).map_err(ZlorbError::Io)?;
    let repos = config_dir.join("repositories.toml");
    if !repos.exists() {
        std::fs::write(&repos, "").map_err(ZlorbError::Io)?;
    }
    Ok(())
}

fn systemctl(is_root_user: bool, args: &[&str]) -> std::io::Result<std::process::ExitStatus> {
    let mut cmd = Command::new("systemctl");
    if !is_root_user {
        cmd.arg("--user");
    }
    cmd.args(args).status()
}

fn manual_steps(is_root_user: bool) -> String {
    if is_root_user {
        "systemctl daemon-reload && systemctl enable --now zlorb.service".to_string()
    } else {
        "systemctl --user daemon-reload && systemctl --user enable --now zlorb.service && sudo loginctl enable-linger $USER".to_string()
    }
}

fn warn_if_other_scope_unit_exists(is_root_user: bool, home_dir: &Path) {
    let other = if is_root_user {
        user_unit_path(home_dir)
    } else {
        system_unit_path()
    };
    if other.exists() {
        println!(
            "warning: another zlorb unit already exists at {}; two daemons in different scopes can watch different config dirs — remove the one you do not use",
            other.display()
        );
    }
}

fn enable_linger() {
    let out = Command::new("loginctl").args(["enable-linger"]).status();
    match out {
        Ok(status) if status.success() => {}
        _ => println!(
            "warning: could not enable linger; the user daemon will stop at logout until you run: sudo loginctl enable-linger $USER"
        ),
    }
}

pub fn setup(print_only: bool) -> Result<(), ZlorbError> {
    let root = is_root();
    let home_dir = get_home_dir();
    let config_dir = get_zlorb_config_dir();
    ensure_config(&config_dir)?;
    let current_exe = std::env::current_exe().map_err(ZlorbError::Io)?;
    let service_bin = resolve_service_binary(&current_exe, &home_dir);
    let unit_path = unit_path_for(root, &home_dir);
    if let Some(parent) = unit_path.parent() {
        std::fs::create_dir_all(parent).map_err(ZlorbError::Io)?;
    }
    let unit = render_unit(&service_bin, &config_dir);
    std::fs::write(&unit_path, &unit).map_err(ZlorbError::Io)?;
    println!("config dir: {}", config_dir.display());
    println!("service binary: {}", service_bin.display());
    println!("systemd unit: {}", unit_path.display());
    warn_if_other_scope_unit_exists(root, &home_dir);
    if print_only {
        println!("--- unit ---\n{}", unit);
        println!("next: {}", manual_steps(root));
        if !service_bin.exists() {
            return Err(ZlorbError::Other(format!(
                "{} does not exist; install zlorb-service first",
                service_bin.display()
            )));
        }
        return Ok(());
    }
    if !service_bin.exists() {
        return Err(ZlorbError::Other(format!(
            "{} does not exist; install zlorb-service, then run: {}",
            service_bin.display(),
            manual_steps(root)
        )));
    }
    let reloaded = systemctl(root, &["daemon-reload"]);
    let enabled = match reloaded {
        Ok(status) if status.success() => systemctl(root, &["enable", "--now", "zlorb.service"]),
        other => other,
    };
    match enabled {
        Ok(status) if status.success() => {
            if !root {
                enable_linger();
            }
            Logger::info("Daemon installed and started".into());
            Ok(())
        }
        _ => Err(ZlorbError::Other(format!(
            "systemd registration did not complete; run: {}",
            manual_steps(root)
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "zlorb-setup-test-{}-{}-{}",
            std::process::id(),
            label,
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn unit_pins_binary_and_config_dir() {
        let unit = render_unit(
            Path::new("/home/u/.cargo/bin/zlorb-service"),
            Path::new("/home/u/.config/zlorb"),
        );
        assert!(unit.contains("ExecStart=\"/home/u/.cargo/bin/zlorb-service\""));
        assert!(unit.contains("ZLORB_CONFIG_DIR=/home/u/.config/zlorb"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn unit_escapes_spaces_percents_and_quotes() {
        let unit = render_unit(
            Path::new("/home/u/my dir/zlorb-service"),
            Path::new("/home/u/100%/zlorb"),
        );
        assert!(unit.contains("ExecStart=\"/home/u/my dir/zlorb-service\""));
        assert!(unit.contains("ZLORB_CONFIG_DIR=/home/u/100%%/zlorb"));
    }

    #[test]
    fn non_root_gets_user_unit_and_root_gets_system_unit() {
        let home = Path::new("/home/u");
        assert_eq!(
            user_unit_path_from(None, home),
            PathBuf::from("/home/u/.config/systemd/user/zlorb.service")
        );
        assert_eq!(
            unit_path_for(true, home),
            PathBuf::from("/etc/systemd/system/zlorb.service")
        );
    }

    #[test]
    fn user_unit_follows_absolute_xdg_config_home() {
        let home = Path::new("/home/u");
        assert_eq!(
            user_unit_path_from(Some("/xdg"), home),
            PathBuf::from("/xdg/systemd/user/zlorb.service")
        );
        assert_eq!(
            user_unit_path_from(Some("relative"), home),
            PathBuf::from("/home/u/.config/systemd/user/zlorb.service")
        );
    }

    #[test]
    fn ensure_config_creates_repositories_file_without_clobbering() {
        let dir = temp_dir("ensure");
        let config_dir = dir.join("zlorb");
        ensure_config(&config_dir).unwrap();
        let repos = config_dir.join("repositories.toml");
        assert!(repos.exists());
        std::fs::write(&repos, "[[repository]]\n").unwrap();
        ensure_config(&config_dir).unwrap();
        assert_eq!(std::fs::read_to_string(&repos).unwrap(), "[[repository]]\n");
    }

    #[test]
    fn service_binary_prefers_sibling_then_cargo_home_then_cargo_bin() {
        let dir = temp_dir("bin");
        let ctl = dir.join("zlorb-ctl");
        std::fs::write(&ctl, "").unwrap();
        let home = dir.join("home");
        assert_eq!(
            resolve_service_binary_from(&ctl, None, &home),
            home.join(".cargo").join("bin").join("zlorb-service")
        );
        assert_eq!(
            resolve_service_binary_from(&ctl, Some("/cargo"), &home),
            PathBuf::from("/cargo/bin/zlorb-service")
        );
        let sibling = dir.join("zlorb-service");
        std::fs::write(&sibling, "").unwrap();
        assert_eq!(resolve_service_binary_from(&ctl, Some("/cargo"), &home), sibling);
    }
}
