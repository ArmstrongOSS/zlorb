use git2::{BranchType, Cred, Error, FetchOptions, Oid, Remote, RemoteCallbacks, Repository};
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::{fs, io::Error as IoError, process::Stdio};
use zlorbrs_lib::config::Config;

#[derive(Serialize, Deserialize, Default, Debug)]
struct ServiceConfig {
    sleep_time: u64,
}

fn setup_config_stuff() -> Result<ServiceConfig, ()> {
    let path_to_config_file_for_service = format!(
        "{}/.config/zlorbrs/service-config.json",
        std::env::home_dir().unwrap().to_str().unwrap()
    );

    if !fs::exists(&path_to_config_file_for_service).unwrap() {
        info!("Service config file not found.. creating it now");
        let _ = fs::write(
            &path_to_config_file_for_service,
            &serde_json::to_string(&ServiceConfig::default()).unwrap(),
        );
    }
    let config_file = std::fs::read_to_string(path_to_config_file_for_service).unwrap();

    let config_data = serde_json::from_str::<ServiceConfig>(&config_file)
        .expect("Failed to convert config file to json string");
    Ok(config_data)
}

fn main() -> Result<(), IoError> {
    env_logger::init();

    let config_data = setup_config_stuff().expect("Failed to setup configuration stuff");

    let mut first_run = true;

    loop {
        // Handle napping at first run
        if first_run {
            first_run = false;
        } else {
            take_a_nap(config_data.sleep_time);
        }

        let dir_path = format!(
            "{}/.config/zlorbrs/configs",
            std::env::home_dir().unwrap().to_str().unwrap()
        );
        let directories = std::fs::read_dir(dir_path);
        if directories.is_err() {
            error!("There are no configuration files created yet");
            continue;
        }

        directories.unwrap().for_each(|item_wrap| {
            let item = item_wrap.unwrap();
            let file_contents =
                fs::read_to_string(format!("{}/config.json", item.path().to_str().unwrap()))
                    .unwrap();
            let config_json = serde_json::from_str::<Config>(&file_contents).unwrap();

            info!(" "); // this just makes logging easier to read
            info!("================ {} ===============", config_json.name);

            let repo = Repository::open(config_json.clone().path).expect("Failed to open repo");

            // ======= Fetching ==========
            // fast forward any changes if there is one
            let local_branch = repo
                .find_branch(&config_json.branch, BranchType::Local)
                .expect("Local branch not found");
            let local_iod: Oid = local_branch
                .get()
                .target()
                .expect("Local branch has no target");
            debug!("before iod: {local_iod}");

            let _ = fast_forward(&repo, &config_json);

            let remote_ref = repo
                .resolve_reference_from_short_name(&format!("origin/{}", config_json.branch))
                .expect("Remote ref not found");
            let remote_iod: Oid = remote_ref.target().expect("Remote ref has no target");
            debug!("remote iod: {remote_iod}");
            // ======= END ==========

            let dist_dir_exists = match std::fs::read_dir(format!("{}/dist", config_json.path)) {
                Ok(_) => true,
                Err(_) => false,
            };

            if !dist_dir_exists || local_iod != remote_iod {
                kick_off_build(&config_json);
            }
        });
    }
}

fn kick_off_build(config_json: &Config) {
    info!("Looks like we got some build pending, lets do that!");
    let path = format!("{}", config_json.path);
    debug!("Running build for: {}", config_json.path);

    let build_command = config_json.build_command.clone();
    let handle = std::thread::spawn(move || {
        let set_dir_res = std::env::set_current_dir(path.clone());
        if set_dir_res.is_err() {
            error!(
                "Failed to set the current directory: {}\nDir: {}",
                set_dir_res.err().unwrap(),
                path
            );
        }

        let build_handle = std::process::Command::new(build_command)
            .stdout(Stdio::piped())
            .output();

        match build_handle {
            Ok(h) => {
                debug!("got status: {:?}", h.status);
                match h.status.code() {
                    Some(0) => {
                        // create util split_to_debug_lines
                        let human_readable = String::from_utf8(h.stdout).unwrap();
                        let split_readable: Vec<&str> = human_readable.split("\n").collect();
                        for line in split_readable {
                            info!("build succeed: {:#?}", line);
                        }
                    }
                    Some(1) => {
                        error!("build error: {:?}", h.stderr);
                    }
                    _ => {}
                };
            }
            Err(e) => {
                error!("Total failure of bun_handle: {}", e);
            }
        };
    });

    handle.join().unwrap();
}

fn take_a_nap(sleep_time: u64) {
    std::thread::sleep(std::time::Duration::from_secs(sleep_time));
}

fn fast_forward(repo: &Repository, config_json: &Config) -> Result<(), git2::Error> {
    let remote: Result<Remote, git2::Error> = repo.find_remote("origin");
    if remote.is_err() {
        error!("Remote Not Found");
        return Err(Error::from_str("Remote Not Found"));
    }

    // setup credentails
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_, _, _| {
        Cred::userpass_plaintext(
            // TODO: use credential helper instead
            "USERNAME", "PASSWORD",
        )
    });
    // apply credentials to fetch options
    let mut fetch_options = FetchOptions::new();
    fetch_options.remote_callbacks(callbacks);

    let fetch_res = remote.unwrap().fetch(
        &[config_json.branch.clone()],
        Some(&mut fetch_options),
        None,
    );
    if fetch_res.is_err() {
        error!("failed to fetch remote: {}", fetch_res.err().unwrap());
    }

    let fetch_head = repo.find_reference("FETCH_HEAD").unwrap();
    let fetch_commit = repo.reference_to_annotated_commit(&fetch_head).unwrap();
    let analysis = repo.merge_analysis(&[&fetch_commit]).unwrap();

    if analysis.0.is_up_to_date() {
        info!("repo is already up to date, skipping fast forward");
        return Ok(());
    }

    if analysis.0.is_fast_forward() {
        info!("Repo needs an update, updating...");
        let refname = format!("refs/heads/{}", config_json.branch);
        let mut reference = repo.find_reference(&refname).unwrap();
        reference
            .set_target(fetch_commit.id(), "Fast-Forward")
            .unwrap();
        repo.set_head(&refname).unwrap();
        return repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()));
    }

    error!("Fast-forward only!");
    Err(Error::from_str("Fast-forward only!"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::path::PathBuf;
    use zlorbrs_lib::shared_test_utils::ENV_MUTEX;

    struct TestEnv {
        home_dir: PathBuf,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.home_dir);
        }
    }

    fn setup_test_env(test_name: &str) -> TestEnv {
        let lock = ENV_MUTEX.lock().unwrap();

        // Setup mocked HOME directory
        let mut home_dir = env::temp_dir();
        home_dir.push(format!("zlorbrs_svc_home_{}", test_name));
        let _ = fs::remove_dir_all(&home_dir);
        fs::create_dir_all(&home_dir).unwrap();

        let home_dir = home_dir.canonicalize().unwrap_or(home_dir);

        unsafe {
            env::set_var("HOME", home_dir.to_str().unwrap());
        }

        TestEnv {
            home_dir,
            _lock: lock,
        }
    }

    #[test]
    fn test_setup_config_stuff_success() {
        let env = setup_test_env("svc_config_success");

        // Create the expected configuration file
        let config_dir = env.home_dir.join(".config/zlorbrs");
        fs::create_dir_all(&config_dir).unwrap();

        let config_file_path = config_dir.join("service-config.json");
        let valid_json = r#"{ "sleep_time": 42 }"#;
        fs::write(config_file_path, valid_json).unwrap();

        let result = setup_config_stuff();
        assert!(result.is_ok());

        let config = result.unwrap();
        assert_eq!(config.sleep_time, 42);
    }

    #[test]
    fn test_setup_config_stuff_missing() {
        let _env = setup_test_env("svc_config_missing");

        // Do not create the file. setup_config_stuff should fail gracefully.
        let result = setup_config_stuff();
        assert!(result.is_err());
    }
}
