use std::{
    env,
    fs::{self, ReadDir},
    io::Write,
    iter::Enumerate,
    process,
};
use zlorb_lib::{
    config::RepositoryConfiguration, create_config_from_toml, error::ZlorbError, get_home_dir,
    log::Logger,
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
    let repos = self::get_all();
    if repos.is_none() {
        Logger::info("There are no repos to remove".into());
        return Ok(());
    }

    let repos = repos.unwrap();
    let mut mapped_repos = repos.map(|item| {
        let x = item.1;
        if x.is_err() {
            Logger::info("Failed to map repos to good type.. aborting".into());
            panic!("Panic due to previous error");
        }
        let x = x.unwrap();
        x.path()
    });

    let found = mapped_repos.find(|item| {
        let file_name = item.file_name();
        if file_name.is_none() {
            return false;
        }
        let file_name = file_name.unwrap();
        let file_name = file_name.to_str();
        if file_name.is_none() {
            return false;
        }
        let file_name = file_name.unwrap();
        file_name == repo_name
    });

    if found.is_none() {
        Logger::info(format!("Theres no config found with name: {}", repo_name));
        return Ok(());
    }

    let found = found.unwrap();
    match std::fs::remove_dir_all(found) {
        Ok(_) => {
            Logger::info(format!("Removed config for: {}", repo_name));
        }
        Err(e) => {
            Logger::info(format!(
                "Unable to remove config for {} because: {:?}",
                repo_name, e
            ));
        }
    };

    Ok(())
}

pub(crate) fn get_all() -> Option<Enumerate<ReadDir>> {
    let mut home_dir = get_home_dir();
    home_dir.push(".config/zlorb/configs");

    if let Ok(dir) = fs::read_dir(home_dir.clone()) {
        return Some(dir.enumerate());
    }

    Logger::info("Config directory doesnt exist. Creating it now...".to_string());
    let create_dir_results = fs::create_dir_all(home_dir.clone());
    if create_dir_results.is_ok() {
        let files = fs::read_dir(home_dir);
        if files.is_err() {
            Logger::info(format!(
                "Failed to create config directory for reason: {}",
                files.err().unwrap(),
            ));
            panic!("Exiting due to previous error")
        }
        let files_unwrapped = files.unwrap();
        return Some(files_unwrapped.enumerate());
    }

    Logger::info(format!(
        "Failed to create config directory for reason: {}",
        create_dir_results.err().unwrap(),
    ));
    panic!("Exiting due to previous error")
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
