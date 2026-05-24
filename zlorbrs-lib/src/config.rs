use std::{fs, io};

use log::info;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub name: String,          // repo identifier
    pub path: String,          // absolute path to repo
    pub branch: String,        // e.g. main
    pub remote: String,        // e.g. origin
    pub build_command: String, // e.g. npm run build
}

impl Config {
    pub fn new(repo_name: String) -> Self {
        Self {
            name: repo_name,
            path: String::from(std::env::current_dir().unwrap().to_str().unwrap()),
            branch: String::from(
                git2::Branch::wrap(
                    git2::Repository::open(std::env::current_dir().unwrap().to_str().unwrap())
                        .unwrap()
                        .references()
                        .unwrap()
                        .next()
                        .unwrap()
                        .unwrap(),
                )
                .name()
                .unwrap()
                .unwrap(),
            ),
            remote: String::from("origin"),
            build_command: String::from("bun run build"),
        }
    }

    pub fn load(repo_name: String) -> Result<String, io::Error> {
        info!("Loading config for {}", repo_name);
        let mut contents = fs::read_to_string(format!(
            "{}/.config/zlorbrs/configs/{}",
            std::env::home_dir().unwrap().to_str().unwrap(),
            repo_name
        ));
        if contents.is_err() {
            info!("Theres no config so we need to create one");
            contents = Ok(Self::save(repo_name));
        }
        info!("Found contents: {:#?}", contents);
        contents
    }

    pub fn save(repo_name: String) -> String {
        info!("Generating configuration file. System assumes Bun build script");
        let directory_path = format!(
            "{}/.config/zlorbrs/configs/{}",
            std::env::home_dir().unwrap().to_str().unwrap(),
            repo_name
        );
        let file_path = format!("{directory_path}/config.json");

        // first create directory
        match std::fs::create_dir_all(directory_path.clone()) {
            Ok(_) => {
                println!("Created config directory at: {directory_path}")
            }
            Err(e) => panic!("{e}"),
        };

        // then write file
        let data = serde_json::to_string(&Config::new(repo_name)).unwrap();
        let raw = data.clone();
        match std::fs::write(file_path.clone(), raw) {
            Ok(_) => {
                println!("Created configuration file at: {file_path}")
            }
            Err(e) => panic!("{e}"),
        };
        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    // use std::sync::Mutex; // removed
    use crate::shared_test_utils::ENV_MUTEX;
    use git2::Repository; // added

    // static TEST_MUTEX: Mutex<()> = Mutex::new(()); // removed

    struct TestEnv {
        home_dir: PathBuf,
        project_dir: PathBuf,
        _lock: std::sync::MutexGuard<'static, ()>,
        original_dir: PathBuf,
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            let _ = env::set_current_dir(&self.original_dir);
            let _ = fs::remove_dir_all(&self.home_dir);
            let _ = fs::remove_dir_all(&self.project_dir);
        }
    }

    fn setup_test_env(test_name: &str) -> TestEnv {
        let lock = ENV_MUTEX.lock().unwrap();

        // 1. Setup mocked HOME directory
        let mut home_dir = env::temp_dir();
        home_dir.push(format!("zlorbrs_home_{}", test_name));
        let _ = fs::remove_dir_all(&home_dir);
        fs::create_dir_all(&home_dir).unwrap();

        unsafe {
            env::set_var("HOME", home_dir.to_str().unwrap());
        }

        // 2. Setup mocked project directory (current_dir)
        let mut project_dir = env::temp_dir();
        project_dir.push(format!("zlorbrs_project_{}", test_name));
        let _ = fs::remove_dir_all(&project_dir);
        fs::create_dir_all(&project_dir).unwrap();

        let home_dir = home_dir.canonicalize().unwrap_or(home_dir);
        let project_dir = project_dir.canonicalize().unwrap_or(project_dir);

        // 3. Initialize a git repository so git2::Repository::open succeeds
        let repo = Repository::init(&project_dir).unwrap();

        // Create an initial commit so we have a branch (usually 'main' or 'master')
        let signature = git2::Signature::now("Test User", "test@example.com").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "Initial commit",
            &tree,
            &[],
        )
        .unwrap();

        // 4. Change current_dir to our mocked project
        let original_dir = env::current_dir().unwrap();
        env::set_current_dir(&project_dir).unwrap();

        TestEnv {
            home_dir,
            project_dir,
            _lock: lock,
            original_dir,
        }
    }

    #[test]
    fn test_config_new() {
        let env = setup_test_env("config_new");
        let repo_name = String::from("test_repo");

        let config = Config::new(repo_name.clone());

        assert_eq!(config.name, repo_name);
        assert_eq!(config.path, env.project_dir.to_str().unwrap());
        // Git branch name depends on global config (master vs main), just ensure it's not empty
        assert!(!config.branch.is_empty());
        assert_eq!(config.remote, "origin");
        assert_eq!(config.build_command, "bun run build");
    }

    #[test]
    fn test_config_save() {
        let env = setup_test_env("config_save");
        let repo_name = String::from("test_repo");

        let saved_json = Config::save(repo_name.clone());

        let expected_config_dir = env
            .home_dir
            .join(".config/zlorbrs/configs")
            .join(&repo_name);
        let expected_file_path = expected_config_dir.join("config.json");

        assert!(
            fs::metadata(&expected_config_dir).is_ok(),
            "Config directory was not created"
        );
        assert!(
            fs::metadata(&expected_file_path).is_ok(),
            "config.json was not created"
        );

        let file_contents = fs::read_to_string(&expected_file_path).unwrap();
        assert_eq!(saved_json, file_contents);

        let parsed_config: Config = serde_json::from_str(&file_contents).unwrap();
        assert_eq!(parsed_config.name, repo_name);
    }

    #[test]
    fn test_config_load_existing() {
        let env = setup_test_env("config_load_existing");
        let repo_name = String::from("test_repo");

        // Manually save a config first
        let _ = Config::save(repo_name.clone());

        // Then try loading it
        let load_result = Config::load(repo_name);
        assert!(load_result.is_ok());

        let loaded_json = load_result.unwrap();
        let config: Config = serde_json::from_str(&loaded_json).unwrap();
        assert_eq!(config.name, "test_repo");
        assert_eq!(config.path, env.project_dir.to_str().unwrap());
    }

    #[test]
    fn test_config_load_missing() {
        let env = setup_test_env("config_load_missing");
        let repo_name = String::from("test_repo");

        let expected_config_dir = env
            .home_dir
            .join(".config/zlorbrs/configs")
            .join(&repo_name);
        let expected_file_path = expected_config_dir.join("config.json");

        // Ensure missing before load
        assert!(fs::metadata(&expected_file_path).is_err());

        // Loading a missing config should automatically create and return it
        let load_result = Config::load(repo_name.clone());
        assert!(load_result.is_ok());

        // Verify it was created
        assert!(
            fs::metadata(&expected_file_path).is_ok(),
            "config.json should be created by load()"
        );

        let loaded_json = load_result.unwrap();
        let config: Config = serde_json::from_str(&loaded_json).unwrap();
        assert_eq!(config.name, "test_repo");
    }
}
