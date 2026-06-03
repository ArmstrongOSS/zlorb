use serde::{Deserialize, Serialize};
use std::{fs, io};

use crate::log::Logger;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoConfig {
    pub name: String,          // repo identifier
    pub path: String,          // absolute path to repo
    pub branch: String,        // e.g. main
    pub remote: String,        // e.g. origin
    pub build_command: String, // e.g. npm run build
}

impl RepoConfig {
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

    pub fn load(&self, repo_name: String) -> Result<String, io::Error> {
        Logger::Info(format!("Loading config for {}", repo_name));
        let mut contents = fs::read_to_string(format!(
            "{}/.config/zlorbrs/configs/{}",
            std::env::home_dir().unwrap().to_str().unwrap(),
            repo_name
        ));
        if contents.is_err() {
            Logger::Info("Theres no config so we need to create one".into());
            contents = Ok(self.save());
        }
        Logger::Info(format!("Found contents: {:#?}", contents));
        contents
    }

    pub fn save(&self) -> String {
        Logger::Info("Generating configuration file. System assumes Bun build script".into());
        let directory_path = format!(
            "{}/.config/zlorbrs/configs/{}",
            std::env::home_dir().unwrap().to_str().unwrap(),
            self.name
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
        let data = serde_json::to_string(self).unwrap();
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
