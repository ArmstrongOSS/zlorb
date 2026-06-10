use log::{error, info};
use std::{
    env,
    fs::{self, ReadDir},
    iter::Enumerate,
    process,
};
use zlorb_lib::{config::RepoConfig, error::ZlorbError, get_home_dir};

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

pub(crate) fn list() {
    let repos = self::get_all();
    if repos.is_none() {
        error!("No configurations found");
        return;
    }

    let mapped_repos = repos.unwrap().map(|item| item.1.unwrap().path());
    println!("{:#?}", Vec::from_iter(mapped_repos));
}
