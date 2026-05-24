use git2::{BranchType, Cred, FetchOptions, Oid, RemoteCallbacks, Repository};
use log::error;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, DirEntry, ReadDir},
    path::PathBuf,
    process::Stdio,
};
use zlorbrs_lib::{config::Config, error::ZlorbError, get_home_dir};

pub mod service;

#[derive(Serialize, Deserialize, Default, Debug)]
struct ServiceConfig {
    sleep_time: u64,
    username: String,
    token: String,
}

fn main() -> Result<(), ZlorbError> {
    colog::init();
    let config_data = setup_config_stuff()?;
    let mut first_run = true;
    loop {
        handle_loop_start(&mut first_run, config_data.sleep_time.clone());
        let directories = get_directories();
        if let Err(_) = directories {
            continue;
        }
        let dirs = directories?;
        dirs.for_each(|directory| {
            let dir = directory.unwrap();
            let config = get_config_json(dir).unwrap();
            let repo = get_repo(&config).unwrap();
            initiate_fast_forward(&repo, &config).unwrap();
        });
    }
}

fn get_config_json(dir: DirEntry) -> Result<Config, ZlorbError> {
    let mut path = PathBuf::from(dir.path());
    path.push("/config.json");
    let file_contents =
        fs::read_to_string(path).map_err(|e| ZlorbError::SerializationError(e.to_string()))?;
    let config_json = serde_json::from_str::<Config>(&file_contents)
        .map_err(|e| ZlorbError::SerializationError(e.to_string()))?;
    Ok(config_json)
}

fn get_repo(config: &Config) -> Result<Repository, ZlorbError> {
    let repo = Repository::open(&config.path).expect("Failed to open repo");
    Ok(repo)
}

fn initiate_fast_forward(repo: &Repository, config_json: &Config) -> Result<(), ZlorbError> {
    let local_branch = repo
        .find_branch(&config_json.branch, BranchType::Local)
        .map_err(|e| ZlorbError::Other(e.to_string()))?;
    let local_iod: Oid = local_branch
        .get()
        .target()
        .expect("Local branch has no target"); // TODO we probably dont want to panic here

    let _ = fast_forward(&repo, &config_json);

    let remote_ref = repo
        .resolve_reference_from_short_name(&format!("origin/{}", config_json.branch))
        .expect("Remote ref not found");
    let remote_iod: Oid = remote_ref.target().expect("Remote ref has no target");

    let mut dist_dir_path = PathBuf::from(&config_json.path);
    dist_dir_path.push("/dist");
    let dist_dir_exists = std::fs::exists(dist_dir_path).map_err(|e| ZlorbError::Io(e))?;
    let local_iod_matches_remote = local_iod == remote_iod;
    if !dist_dir_exists || !local_iod_matches_remote {
        kick_off_build(&config_json).map_err(|e| ZlorbError::Other(e.to_string()))?;
    }
    Ok(())
}

fn kick_off_build(config_json: &Config) -> Result<(), ZlorbError> {
    let path = PathBuf::from(&config_json.path);
    let build_command = config_json.build_command.clone();
    let handle = std::thread::spawn(move || -> Result<(), ZlorbError> {
        std::env::set_current_dir(path).map_err(|e| ZlorbError::Io(e))?;
        let build_handle = std::process::Command::new(build_command)
            .stdout(Stdio::piped())
            .output()
            .map_err(|_| ZlorbError::Other("Unable to retrieve build handle".to_string()))?;

        if build_handle.status.code() == Some(1) {
            return Err(ZlorbError::Other(
                "build returned status code 1 resulting in failure".to_string(),
            ));
        }
        Ok(())
    });

    handle
        .join()
        .map_err(|_| ZlorbError::Other("Faild to join the thread".to_string()))?
}

fn take_a_nap(sleep_time: u64) {
    std::thread::sleep(std::time::Duration::from_secs(sleep_time));
}

fn fast_forward(repo: &Repository, config_json: &Config) -> Result<(), ZlorbError> {
    let mut remote = repo.find_remote("origin").map_err(|e| ZlorbError::Git(e))?;

    // setup credentails
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_, _, _| Cred::userpass_plaintext());
    // apply credentials to fetch options
    let mut fetch_options = FetchOptions::new();
    fetch_options.remote_callbacks(callbacks);

    let fetch_res = remote.fetch(
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
        return Ok(());
    }

    if !analysis.0.is_fast_forward() {
        return Err(ZlorbError::Git(git2::Error::new(
            git2::ErrorCode::NotFastForward,
            git2::ErrorClass::Invalid,
            "Fast-forward only!",
        )));
    }

    let refname = format!("refs/heads/{}", config_json.branch);
    repo.find_reference(&refname)
        .map_err(|e| ZlorbError::Git(e))?
        .set_target(fetch_commit.id(), "Fast-Forward")
        .map_err(|e| ZlorbError::Git(e))?;
    repo.set_head(&refname).map_err(|e| ZlorbError::Git(e))?;
    let mut checkout = git2::build::CheckoutBuilder::default();
    repo.checkout_head(Some(&mut checkout))
        .map_err(|e| ZlorbError::Git(e))?;
    Ok(())
}

fn create_config(home_dir: &PathBuf) -> Result<(), ZlorbError> {
    let conf = &serde_json::to_string(&ServiceConfig::default())
        .map_err(|e| ZlorbError::ConfigParseError(e.to_string()))?;
    let _ = fs::write(&home_dir, conf).map_err(|e| ZlorbError::Io(e));
    Ok(())
}

fn setup_config_stuff() -> Result<ServiceConfig, ZlorbError> {
    let home_dir = get_home_dir().join("/.config/zlorbrs/service-config.json");
    let file_exists =
        fs::exists(&home_dir).map_err(|_| ZlorbError::ConfigNotFound(home_dir.clone()))?;

    if !file_exists {
        let _ = create_config(&home_dir);
    }

    read_config_file(&home_dir)
}

fn read_config_file(home_dir: &PathBuf) -> Result<ServiceConfig, ZlorbError> {
    let config_file = std::fs::read_to_string(&home_dir)
        .map_err(|_| ZlorbError::FileNotFOund(home_dir.clone()))?;
    let config_data = serde_json::from_str::<ServiceConfig>(&config_file).map_err(|_| {
        ZlorbError::ConfigParseError("Failed to convert config file to json string".to_string())
    })?;
    Ok(config_data)
}

fn handle_loop_start(first_run: &mut bool, sleep_time: u64) {
    if !std::mem::replace(first_run, false) {
        take_a_nap(sleep_time);
    }
}

fn get_directory_path() -> PathBuf {
    get_home_dir().join(".config/zlorbrs/configs")
}

fn get_directories() -> Result<ReadDir, ZlorbError> {
    let dir_path = get_directory_path();
    let dir = std::fs::read_dir(dir_path).map_err(|e| ZlorbError::Io(e))?;
    Ok(dir)
}
