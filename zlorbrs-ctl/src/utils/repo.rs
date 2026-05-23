use log::{error, info};
use std::{
    env,
    fs::{self, ReadDir},
    iter::Enumerate,
    process,
};
use zlorbrs_lib::{config::Config, error::ZlorbError, get_home_dir};

pub(crate) fn watch() {
    let out = process::Command::new("journalctl")
        .arg("-f")
        .arg("-u")
        .arg("zlorbrs")
        .status()
        .map_err(|e| ZlorbError::Io(e));
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
        return file_name == &repo_name;
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
    home_dir.push("/.config/zlorbrs/configs");

    if let Ok(dir) = fs::read_dir(home_dir.clone()) {
        return Some(dir.enumerate());
    }

    error!("Config directory doesnt exist. Creating it now...");
    let create_dir_results = fs::create_dir_all(home_dir.clone());
    if let Ok(_) = create_dir_results {
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
            "{:?} is already configured. If you want to edit the configuration file, you can find it at HOME/zlorbrs/configs/{:?}",
            dir_name, dir_name
        );
        return;
    }

    let _ = Config::load(String::from(dir_name.unwrap().to_str().unwrap()));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use zlorbrs_lib::get_home_dir;

    // Mutex to ensure tests that modify HOME environment variable don't conflict.
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    fn setup_test_env(test_name: &str) -> (PathBuf, std::sync::MutexGuard<'static, ()>) {
        let lock = TEST_MUTEX.lock().unwrap();
        let mut tmp_dir = env::temp_dir();
        tmp_dir.push(format!("zlorbrs_test_{}", test_name));

        // Clean up from previous failed test if any
        let _ = fs::remove_dir_all(&tmp_dir);
        fs::create_dir_all(&tmp_dir).unwrap();

        unsafe {
            env::set_var("HOME", tmp_dir.to_str().unwrap());
        }

        (tmp_dir, lock)
    }

    fn teardown_test_env(tmp_dir: PathBuf) {
        let _ = fs::remove_dir_all(tmp_dir);
    }

    #[test]
    fn test_get_all_empty() {
        let (tmp_dir, _lock) = setup_test_env("get_all_empty");

        let all = get_all();
        assert!(all.is_some());
        let all = all.unwrap();
        assert_eq!(all.count(), 0);

        teardown_test_env(tmp_dir);
    }

    #[test]
    fn test_remove_existing() {
        let (tmp_dir, _lock) = setup_test_env("remove_existing");

        let mut home_dir = get_home_dir();
        home_dir.push("/.config/zlorbrs/configs/test_repo");
        fs::create_dir_all(&home_dir).unwrap();
        fs::write(format!("{}/config.json", home_dir), "{}").unwrap();

        assert!(fs::metadata(&home_dir).is_ok());

        remove(String::from("test_repo"));

        assert!(fs::metadata(&home_dir).is_err());

        teardown_test_env(tmp_dir);
    }

    #[test]
    fn test_remove_non_existent() {
        let (tmp_dir, _lock) = setup_test_env("remove_non_existent");

        // This won't panic because remove handles missing configurations
        remove(String::from("does_not_exist"));

        teardown_test_env(tmp_dir);
    }
}
