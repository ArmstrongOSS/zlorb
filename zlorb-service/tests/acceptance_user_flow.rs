use git2::{IndexAddOption, Oid, RepositoryInitOptions, Signature};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct RootCleanup(PathBuf);

impl Drop for RootCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct DaemonGuard(Child);

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn temp_root() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "zlorb-acceptance-test-{}-{}",
        std::process::id(),
        nanos
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_fixture_crate(repo_path: &Path, marker: &str) {
    std::fs::create_dir_all(repo_path.join("src")).unwrap();
    std::fs::write(
        repo_path.join("Cargo.toml"),
        "[package]\nname = \"fixture-app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        repo_path.join("src/main.rs"),
        format!("fn main() {{ println!(\"{}\"); }}\n", marker),
    )
    .unwrap();
}

fn commit_all(repo: &git2::Repository, message: &str) -> Oid {
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_oid = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();
    let sig = Signature::now("zlorb-test", "zlorb-test@example.com").unwrap();
    let parent = repo
        .head()
        .ok()
        .and_then(|head| head.target())
        .map(|oid| repo.find_commit(oid).unwrap());
    match parent {
        Some(parent) => repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
            .unwrap(),
        None => repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &[])
            .unwrap(),
    }
}

fn ctl_binary() -> PathBuf {
    let service = PathBuf::from(env!("CARGO_BIN_EXE_zlorb-service"));
    let dir = service.parent().unwrap();
    dir.join(format!("zlorb-ctl{}", std::env::consts::EXE_SUFFIX))
}

fn fixture_artifact(watched: &Path) -> PathBuf {
    watched
        .join("target")
        .join("debug")
        .join(format!("fixture-app{}", std::env::consts::EXE_SUFFIX))
}

fn run_artifact(artifact: &Path) -> Option<String> {
    let output = Command::new(artifact).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn read_log(path: &Path) -> String {
    let mut contents = String::new();
    if let Ok(mut file) = std::fs::File::open(path) {
        let _ = file.read_to_string(&mut contents);
    }
    let tail: Vec<&str> = contents.lines().rev().take(40).collect();
    tail.into_iter().rev().collect::<Vec<_>>().join("\n")
}

#[test]
fn full_user_flow_add_edit_remote_update_and_build() {
    let root = temp_root();
    let _root_cleanup = RootCleanup(root.clone());
    let config_dir = root.join("config");
    let upstream_path = root.join("upstream");
    let watched_path = root.join("watched");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("service-config.toml"),
        "sleep_time = 1\nusername = \"u\"\ntoken = \"t\"\n",
    )
    .unwrap();

    let mut init_opts = RepositoryInitOptions::new();
    init_opts.initial_head("master");
    let upstream = git2::Repository::init_opts(&upstream_path, &init_opts).unwrap();
    write_fixture_crate(&upstream_path, "fixture-version-one");
    let v1 = commit_all(&upstream, "v1");
    drop(upstream);
    let watched = git2::Repository::clone(
        upstream_path.to_string_lossy().as_ref(),
        &watched_path,
    )
    .unwrap();
    assert_eq!(watched.head().unwrap().target().unwrap(), v1);
    drop(watched);

    let ctl = ctl_binary();
    assert!(
        ctl.exists(),
        "built zlorb-ctl not found at {}; build the workspace binaries (cargo build --workspace or cargo test --workspace) before running this test",
        ctl.display()
    );

    let daemon_log_path = root.join("daemon.log");
    let daemon_log = std::fs::File::create(&daemon_log_path).unwrap();
    let daemon = Command::new(env!("CARGO_BIN_EXE_zlorb-service"))
        .env("ZLORB_CONFIG_DIR", &config_dir)
        .env_remove("CARGO_TARGET_DIR")
        .stdout(Stdio::from(daemon_log))
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let daemon_guard = DaemonGuard(daemon);

    let add = Command::new(&ctl)
        .arg("add")
        .current_dir(&watched_path)
        .env("ZLORB_CONFIG_DIR", &config_dir)
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "zlorb-ctl add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );
    let repos_path = config_dir.join("repositories.toml");
    let repos = std::fs::read_to_string(&repos_path).unwrap();
    assert!(repos.contains("watched"), "ctl add entry missing: {}", repos);
    assert!(
        repos.contains(&watched_path.to_string_lossy().into_owned()),
        "ctl add path missing: {}",
        repos
    );

    assert!(
        repos.contains("build_command = \"\""),
        "unexpected ctl add entry shape: {}",
        repos
    );
    let edited = repos.replace(
        "build_command = \"\"",
        "build_strategy = \"rust\"\nbuild_command = \"\"",
    );
    std::fs::write(&repos_path, edited).unwrap();

    let pickup_deadline = Instant::now() + Duration::from_secs(30);
    let mut picked_up = false;
    while Instant::now() < pickup_deadline {
        if read_log(&daemon_log_path).contains("Updating repo: watched") {
            picked_up = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(
        picked_up,
        "daemon did not process the added repo within 30s; daemon log tail:\n{}",
        read_log(&daemon_log_path)
    );
    let watched = git2::Repository::open(&watched_path).unwrap();
    assert_eq!(watched.head().unwrap().target().unwrap(), v1);
    drop(watched);
    assert!(
        std::fs::read_to_string(watched_path.join("src/main.rs"))
            .unwrap()
            .contains("fixture-version-one")
    );
    assert!(!fixture_artifact(&watched_path).exists());

    let upstream = git2::Repository::open(&upstream_path).unwrap();
    write_fixture_crate(&upstream_path, "fixture-version-two");
    let v2 = commit_all(&upstream, "v2");
    drop(upstream);

    let artifact = fixture_artifact(&watched_path);
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut built_output = None;
    while Instant::now() < deadline {
        if artifact.exists() {
            if let Some(stdout) = run_artifact(&artifact) {
                if stdout.contains("fixture-version-two") {
                    built_output = Some(stdout);
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let built_output = match built_output {
        Some(output) => output,
        None => panic!(
            "daemon did not build fixture-version-two within 120s; daemon log tail:\n{}",
            read_log(&daemon_log_path)
        ),
    };
    assert!(built_output.contains("fixture-version-two"));

    let watched = git2::Repository::open(&watched_path).unwrap();
    assert_eq!(watched.head().unwrap().target().unwrap(), v2);
    let source = std::fs::read_to_string(watched_path.join("src/main.rs")).unwrap();
    assert!(source.contains("fixture-version-two"));
    drop(watched);
    drop(daemon_guard);

    let _ = std::fs::remove_dir_all(&root);
}
