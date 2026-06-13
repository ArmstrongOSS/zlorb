use std::{env, io::Write, process};
use zlorb_lib::{
    config::{RepositoriesConfigurationFile, RepositoryConfiguration},
    create_config_from_toml,
    error::ZlorbError,
};

pub(crate) fn watch() -> Result<(), ZlorbError> {
    let out = process::Command::new("journalctl")
        .arg("-f")
        .arg("-u")
        .arg("zlorb")
        .status()
        .map_err(ZlorbError::Io);
    match out {
        Ok(status) => {
            if status.success() {
                println!("End");
                Ok(())
            } else {
                println!("End: Fail");
                Ok(())
            }
        }
        Err(e) => {
            e.print();
            Err(e)
        }
    }
}

pub(crate) fn remove(repo_name: String) -> Result<(), ZlorbError> {
    let (mut config, mut file) = create_config_from_toml(true)?;
    // check if repo exists before attempting remove
    throw_if_repo_not_found(&repo_name, &config)?;

    let filtered_repos = config
        .repositories
        .into_iter()
        .filter(|repo| repo.name != repo_name)
        .collect();
    config.repositories = filtered_repos;
    let out_toml = toml::to_string(&config).map_err(ZlorbError::TomlSerializationError)?;
    file.write_all(out_toml.as_bytes())?;
    Ok(())
}

/// Adds the current working directory as a tracked repository in the global configuration.
/// **Global configs location:** `~/.config/zlorb`
///
/// TODO: Ensure the directory being added is a valid git repository
pub(crate) fn add() -> Result<(), ZlorbError> {
    let cwd = env::current_dir()?;
    let path_str = cwd.to_string_lossy().to_string();

    let repo_name = cwd
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap();

    let (mut config, mut file) = create_config_from_toml(true)?;

    throw_if_repo_exists(&repo_name, &config)?;

    let new_repo = RepositoryConfiguration {
        name: repo_name,
        path: path_str,
        remote: "origin".to_string(),
        branch: "master".to_string(),
        build_command: String::new(),
    };

    config.repositories.push(new_repo);
    let out_toml = toml::to_string(&config).map_err(ZlorbError::TomlSerializationError)?;
    file.write_all(out_toml.as_bytes())?;
    Ok(())
}

pub(crate) fn list() -> Result<(), ZlorbError> {
    let (config, _) = create_config_from_toml(false)?;

    let mapped_repos = config.repositories.iter().map(|item| item.path.clone());
    println!("{:#?}", Vec::from_iter(mapped_repos));
    Ok(())
}

fn throw_if_repo_exists(
    repo_name: &String,
    config: &RepositoriesConfigurationFile,
) -> Result<(), ZlorbError> {
    if let Some(found_el) = config
        .repositories
        .iter()
        .find(|item| &item.name == repo_name)
    {
        let err = format!("{} is already in the configuration", found_el.name);
        return Err(ZlorbError::Other(err));
    }

    Ok(())
}

fn throw_if_repo_not_found(
    repo_name: &String,
    config: &RepositoriesConfigurationFile,
) -> Result<(), ZlorbError> {
    if config
        .repositories
        .iter()
        .find(|repo| &repo.name == repo_name)
        .is_none()
    {
        let err = format!("repo '{}' not found in tracked repos", repo_name);
        return Err(ZlorbError::Other(err));
    };

    Ok(())
}
