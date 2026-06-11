use log::{error, info};
use std::{
    env,
    fs::{self, OpenOptions, ReadDir, create_dir_all},
    io::Write,
    iter::Enumerate,
    process,
};
use zlorb_lib::{config::RepoConfig, error::ZlorbError, get_home_dir};

use crate::configuration::RepositoryConfiguration;

pub(crate) fn watch() {
    let out = process::Command::new("journalctl")
        .arg("-f")
        .arg("-u")
        .arg("zlorb")
        .status()
        .map_err(ZlorbError::Io);
    match out {
        Ok(status) => {
            if status.success() {
                println!("End")
            } else {
                println!("End: Fail")
            }
        }
        Err(e) => {
            e.print();
        }
    }
}

pub(crate) fn remove(repo_name: String) {
    let repos = self::get_all();
    if repos.is_none() {
        error!("There are no repos to remove");
        return;
    }
    let repos = repos.unwrap();
    let mut mapped_repos = repos.map(|item| {
        let x = item.1;
        if x.is_err() {
            error!("Failed to map repos to good type.. aborting");
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
        error!("Theres no config found with name: {}", repo_name);
        return;
    }
    let found = found.unwrap();
    match std::fs::remove_dir_all(found) {
        Ok(_) => {
            info!("Removed config for: {}", repo_name);
        }
        Err(e) => {
            error!("Unable to remove config for {} because: {:?}", repo_name, e);
        }
    };
}

pub(crate) fn get_all() -> Option<Enumerate<ReadDir>> {
    let mut home_dir = get_home_dir();
    home_dir.push(".config/zlorb/configs");

    if let Ok(dir) = fs::read_dir(home_dir.clone()) {
        return Some(dir.enumerate());
    }

    error!("Config directory doesnt exist. Creating it now...");
    let create_dir_results = fs::create_dir_all(home_dir.clone());
    if create_dir_results.is_ok() {
        let files = fs::read_dir(home_dir);
        if files.is_err() {
            error!(
                "Failed to create config directory for reason: {}",
                files.err().unwrap()
            );
            panic!("Exiting due to previous error")
        }
        let files_unwrapped = files.unwrap();
        return Some(files_unwrapped.enumerate());
    }

    error!(
        "Failed to create config directory for reason: {}",
        create_dir_results.err().unwrap()
    );
    panic!("Exiting due to previous error")
}

pub(crate) fn add() {
    let current_dir_pathbuf = env::current_dir().unwrap();

    let dir_name = current_dir_pathbuf.file_name();
    if dir_name.is_none() {
        error!("Failed to get current directory name");
        return;
    }

    let current_configs = self::get_all();
    if current_configs.is_none() {
        error!("The configs directory contains nothing");
        return;
    }

    let found_config = current_configs
        .unwrap()
        .find(|x| x.1.as_ref().unwrap().file_name() == dir_name.unwrap());

    if found_config.is_some() {
        error!(
            "{:?} is already configured. If you want to edit the configuration file, you can find it at HOME/zlorb/configs/{:?}",
            dir_name, dir_name
        );
        return;
    }
    let repo_name = dir_name.unwrap().to_str().unwrap().to_string();
    let repo = RepoConfig::new(repo_name.clone());
    let _ = repo.load(repo_name);
}

/// Adds the current working directory as a tracked repository in the global configuration.
///
/// ### Errors
/// This function will return an error if:
/// * The current working directory cannot be determined.
/// * The current working directory is not a valid Git repository.
/// * The system fails to locate or create the `~/.config/zlob/` directory.
/// * There are file I/O permissions or serialization errors when appending to `repositories.toml`.
///
/// TODO: Read the file before writing to prevent duplicates
/// TODO: Ensure the directory being added is a valid git repository
pub(crate) fn add_toml() -> Result<(), Box<dyn std::error::Error>> {
    // get current working directory
    let cwd = std::env::current_dir()?;
    let path_str = cwd.to_string_lossy().to_string();

    // use folder name as repository name
    let repository_name = cwd
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap();

    // TODO: extract actual git remote/branch
    let current_repository = RepositoryConfiguration {
        name: repository_name.clone(),
        path: path_str,
        remote: "origin".to_string(),
        branch: "master".to_string(),
        build_command: String::new(),
    };

    let mut configuration_path = get_home_dir().join(".config/zlorb");
    let _ = create_dir_all(&configuration_path); // ensure `~/.config/zlorb/` exists
    configuration_path.push("repositories.toml");

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&configuration_path)?;
    let mut toml_string = format!("\n[[repository]]\n");
    toml_string.push_str(&toml::to_string(&current_repository)?);

    file.write_all(toml_string.as_bytes())?;
    println!(
        "Directory '{}' added to tracked repositories",
        repository_name
    );

    Ok(())
}

pub(crate) fn list() {
    let repos = self::get_all();
    if repos.is_none() {
        error!("No configurations found");
        return;
    }

    let mapped_repos = repos.unwrap().map(|item| item.1.unwrap().path());
    println!("{:#?}", Vec::from_iter(mapped_repos));
}
