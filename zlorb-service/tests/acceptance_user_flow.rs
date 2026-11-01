use git2::{IndexAddOption, Oid, RepositoryInitOptions, Signature};
use std::io::Read;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

struct RootCleanup(PathBuf);

impl Drop for RootCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct DaemonGuard(Child);

impl DaemonGuard {
    fn exited(&mut self) -> Option<std::process::ExitStatus> {
        self.0.try_wait().ok().flatten()
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = Command::new("kill")
                .args(["-KILL", &format!("-{}", self.0.id())])
                .status();
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn temp_root() -> PathBuf {
    let nanos = SystemTime::now()
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
    let workdir = repo.workdir().unwrap().to_path_buf();
    let tracked: Vec<String> = index
        .iter()
        .map(|entry| String::from_utf8_lossy(&entry.path).into_owned())
        .collect();
    for path in tracked {
        if !workdir.join(&path).exists() {
            index.remove_path(Path::new(&path)).unwrap();
        }
    }
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

fn newest_source_mtime(dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                if let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) {
                    if newest.is_none_or(|n| mtime > n) {
                        newest = Some(mtime);
                    }
                }
            }
        }
    }
    newest
}

fn ensure_ctl_binary() -> PathBuf {
    let ctl = ctl_binary();
    if ctl.exists() {
        assert_ctl_fresh(&ctl);
        return ctl;
    }
    let target_dir = ctl
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("acceptance-ctl-target");
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let build = Command::new(cargo)
        .args(["build", "-p", "zlorb-ctl", "--locked"])
        .current_dir(&workspace)
        .env("CARGO_TARGET_DIR", &target_dir)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "failed to build zlorb-ctl for this test: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let built = target_dir
        .join("debug")
        .join(format!("zlorb-ctl{}", std::env::consts::EXE_SUFFIX));
    assert!(
        built.exists(),
        "zlorb-ctl build succeeded but no binary at {}",
        built.display()
    );
    built
}

fn assert_ctl_fresh(ctl: &Path) {
    let ctl_mtime = std::fs::metadata(ctl)
        .and_then(|m| m.modified())
        .unwrap();
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    for crate_dir in ["zlorb-ctl", "zlorb-lib", "zlorb-web"] {
        let src = workspace.join(crate_dir).join("src");
        if let Some(newest) = newest_source_mtime(&src) {
            assert!(
                ctl_mtime >= newest,
                "zlorb-ctl binary at {} is older than the {} sources in this tree; rebuild the workspace binaries (cargo build --workspace or cargo test --workspace) so this test exercises the current ctl",
                ctl.display(),
                crate_dir
            );
        }
    }
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

fn count_in_log(path: &Path, needle: &str) -> usize {
    std::fs::read_to_string(path)
        .map(|contents| contents.matches(needle).count())
        .unwrap_or(0)
}

fn read_log(path: &Path) -> String {
    let mut contents = String::new();
    if let Ok(mut file) = std::fs::File::open(path) {
        let _ = file.read_to_string(&mut contents);
    }
    let tail: Vec<&str> = contents.lines().rev().take(80).collect();
    tail.into_iter().rev().collect::<Vec<_>>().join("\n")
}

fn wait_for_log(
    daemon: &mut DaemonGuard,
    log_path: &Path,
    needle: &str,
    timeout: Duration,
    failure: &str,
) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(status) = daemon.exited() {
            panic!(
                "daemon exited with {} while waiting for {:?}; daemon log tail:\n{}",
                status,
                needle,
                read_log(log_path)
            );
        }
        if read_log(log_path).contains(needle) {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    panic!(
        "{}; daemon log tail:\n{}",
        failure,
        read_log(log_path)
    );
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
    std::fs::write(upstream_path.join("old.txt"), "old file\n").unwrap();
    let v1 = commit_all(&upstream, "v1");
    drop(upstream);
    let watched = git2::Repository::clone(
        upstream_path.to_string_lossy().as_ref(),
        &watched_path,
    )
    .unwrap();
    assert_eq!(watched.head().unwrap().target().unwrap(), v1);
    drop(watched);

    let ctl = ensure_ctl_binary();

    let daemon_log_path = root.join("daemon.log");
    let daemon_log = std::fs::File::create(&daemon_log_path).unwrap();
    let daemon_stderr = daemon_log.try_clone().unwrap();
    let mut daemon_cmd = Command::new(env!("CARGO_BIN_EXE_zlorb-service"));
    daemon_cmd
        .env("ZLORB_CONFIG_DIR", &config_dir)
        .env_remove("CARGO_TARGET_DIR")
        .stdout(Stdio::from(daemon_log))
        .stderr(Stdio::from(daemon_stderr));
    #[cfg(unix)]
    daemon_cmd.process_group(0);
    let daemon = daemon_cmd.spawn().unwrap();
    let mut daemon_guard = DaemonGuard(daemon);

    wait_for_log(
        &mut daemon_guard,
        &daemon_log_path,
        "repos: []",
        Duration::from_secs(30),
        "daemon did not complete an initial cycle with an empty repo list within 30s",
    );

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
    assert!(
        repos.contains("name = \"watched\""),
        "ctl add entry missing: {}",
        repos
    );
    assert!(
        repos.contains(&watched_path.to_string_lossy().into_owned()),
        "ctl add path missing: {}",
        repos
    );

    wait_for_log(
        &mut daemon_guard,
        &daemon_log_path,
        "Updating repo: watched",
        Duration::from_secs(30),
        "daemon did not process the added repo within 30s",
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

    assert!(
        repos.contains("build_command = \"\""),
        "unexpected ctl add entry shape: {}",
        repos
    );
    let edited = repos.replace(
        "build_command = \"\"",
        &format!(
            "build_strategy = \"rust\"\nbuild_command = \"\"\nrestart_command = \"./target/debug/fixture-app{} > served.txt\"",
            std::env::consts::EXE_SUFFIX
        ),
    );
    let edit_tmp = config_dir.join("repositories.toml.tmp");
    std::fs::write(&edit_tmp, edited).unwrap();
    std::fs::rename(&edit_tmp, &repos_path).unwrap();

    let cycles_before_edit = count_in_log(&daemon_log_path, "Updating repo: watched");
    let deadline = Instant::now() + Duration::from_secs(30);
    while count_in_log(&daemon_log_path, "Updating repo: watched") < cycles_before_edit + 2 {
        if let Some(status) = daemon_guard.exited() {
            panic!(
                "daemon exited with {} while idling at v1; daemon log tail:\n{}",
                status,
                read_log(&daemon_log_path)
            );
        }
        if Instant::now() > deadline {
            panic!(
                "daemon did not complete two cycles with the edited config within 30s; daemon log tail:\n{}",
                read_log(&daemon_log_path)
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(
        !watched_path.join("served.txt").exists(),
        "restart command ran even though HEAD never changed"
    );

    let upstream = git2::Repository::open(&upstream_path).unwrap();
    write_fixture_crate(&upstream_path, "fixture-version-two");
    std::fs::remove_file(upstream_path.join("old.txt")).unwrap();
    std::fs::write(upstream_path.join("new.txt"), "new file\n").unwrap();
    let v2 = commit_all(&upstream, "v2");
    drop(upstream);

    let artifact = fixture_artifact(&watched_path);
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut built_output = None;
    while Instant::now() < deadline {
        if let Some(status) = daemon_guard.exited() {
            panic!(
                "daemon exited with {} during the update/build wait; daemon log tail:\n{}",
                status,
                read_log(&daemon_log_path)
            );
        }
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

    let served_path = watched_path.join("served.txt");
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut served_output = None;
    while Instant::now() < deadline {
        if let Some(status) = daemon_guard.exited() {
            panic!(
                "daemon exited with {} during the restart wait; daemon log tail:\n{}",
                status,
                read_log(&daemon_log_path)
            );
        }
        if let Ok(contents) = std::fs::read_to_string(&served_path) {
            if contents.contains("fixture-version-two") {
                served_output = Some(contents);
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let served_output = match served_output {
        Some(output) => output,
        None => panic!(
            "daemon did not run the restart command after the v2 build within 60s; daemon log tail:\n{}",
            read_log(&daemon_log_path)
        ),
    };
    assert!(served_output.contains("fixture-version-two"));

    let watched = git2::Repository::open(&watched_path).unwrap();
    assert_eq!(watched.head().unwrap().target().unwrap(), v2);
    let source = std::fs::read_to_string(watched_path.join("src/main.rs")).unwrap();
    assert!(source.contains("fixture-version-two"));
    assert_eq!(
        std::fs::read_to_string(watched_path.join("new.txt")).unwrap(),
        "new file\n"
    );
    assert!(!watched_path.join("old.txt").exists());
    let statuses = watched.statuses(None).unwrap();
    assert!(
        statuses
            .iter()
            .all(|entry| entry.status() == git2::Status::WT_NEW),
        "no tracked file may be dirty after the fast-forward, got: {:?}",
        statuses
            .iter()
            .map(|entry| (entry.path().map(str::to_string), entry.status()))
            .collect::<Vec<_>>()
    );
    drop(statuses);
    drop(watched);
    drop(daemon_guard);

    let _ = std::fs::remove_dir_all(&root);
}
