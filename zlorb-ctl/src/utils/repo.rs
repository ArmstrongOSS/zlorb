use std::{env, io::Write, process};
use zlorb_lib::{
    config::RepositoryConfiguration, create_config_from_toml, error::ZlorbError, log::Logger,
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
    if let None = config
        .repositories
        .iter()
        .find(|repo| repo.name == repo_name)
    {
        Logger::error(format!("repo '{}' not found in tracked repos", repo_name));
    };
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
    // we dont want to write a new config to the configs file
    // if the same directory has already been added
    if let Some(found_el) = config
        .repositories
        .iter()
        .find(|item| item.name == repo_name)
    {
        let err = format!("{} is already in the configuration", found_el.name);
        return Err(ZlorbError::Other(err));
    }

    let new_repo = RepositoryConfiguration {
        name: repo_name,
        path: path_str,
        remote: "origin".to_string(),
        branch: "master".to_string(),
        build_command: String::new(),
    };

    println!("{:?}", config);
    config.repositories.push(new_repo);
    println!("{:?}", config.repositories);
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
