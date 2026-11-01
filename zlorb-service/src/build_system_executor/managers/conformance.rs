use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use zlorb_lib::{
    config::{BuildStrategyKind, RepositoryConfiguration},
    error::ZlorbError,
};

use crate::build_system_executor::{
    managers::{
        bun::BunManager, manager::BuildStrategy, rust::RustManager, template::TemplateManager,
    },
    select_strategy,
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Step {
    script: &'static str,
    label: &'static str,
    fail_fragment: &'static str,
}

struct ConformanceCase {
    name: &'static str,
    runtime_name: &'static str,
    steps: &'static [Step],
    run: fn(&RepositoryConfiguration, &Path) -> Result<(), ZlorbError>,
}

fn run_bun(config: &RepositoryConfiguration, runtime: &Path) -> Result<(), ZlorbError> {
    BunManager.exec_with_bun_path(config, runtime)
}

fn run_rust(config: &RepositoryConfiguration, runtime: &Path) -> Result<(), ZlorbError> {
    RustManager.exec_with_cargo_path(config, runtime)
}

fn run_template(config: &RepositoryConfiguration, runtime: &Path) -> Result<(), ZlorbError> {
    let search_path = runtime.parent().map(|dir| dir.to_path_buf().into_os_string());
    TemplateManager.exec_with_search_path(config, search_path)
}

fn bun_case() -> ConformanceCase {
    ConformanceCase {
        name: "bun",
        runtime_name: "bun",
        steps: &[
            Step {
                script: "install",
                label: "install",
                fail_fragment: "Failed to install dependencies",
            },
            Step {
                script: "run",
                label: "run build",
                fail_fragment: "Failed to build",
            },
        ],
        run: run_bun,
    }
}

fn rust_case() -> ConformanceCase {
    ConformanceCase {
        name: "rust",
        runtime_name: "cargo",
        steps: &[Step {
            script: "build",
            label: "build",
            fail_fragment: "Failed to build",
        }],
        run: run_rust,
    }
}

fn template_case() -> ConformanceCase {
    ConformanceCase {
        name: "template",
        runtime_name: "template-runtime",
        steps: &[
            Step {
                script: "install",
                label: "install",
                fail_fragment: "Failed to install dependencies",
            },
            Step {
                script: "run",
                label: "run build",
                fail_fragment: "Failed to build",
            },
        ],
        run: run_template,
    }
}

fn case_for_kind(kind: BuildStrategyKind) -> ConformanceCase {
    match kind {
        BuildStrategyKind::Bun => bun_case(),
        BuildStrategyKind::Rust => rust_case(),
    }
}

fn case_for_strategy(strategy: &BuildStrategy) -> ConformanceCase {
    match strategy {
        BuildStrategy::Bun(_) => bun_case(),
        BuildStrategy::Rust(_) => rust_case(),
    }
}

fn next_kind(kind: BuildStrategyKind) -> Option<BuildStrategyKind> {
    match kind {
        BuildStrategyKind::Bun => Some(BuildStrategyKind::Rust),
        BuildStrategyKind::Rust => None,
    }
}

fn registered_kinds() -> Vec<BuildStrategyKind> {
    let mut kinds = Vec::new();
    let mut current = Some(BuildStrategyKind::Bun);
    while let Some(kind) = current {
        kinds.push(kind);
        current = next_kind(kind);
    }
    kinds
}

fn registered_cases() -> Vec<ConformanceCase> {
    let mut cases: Vec<ConformanceCase> = registered_kinds()
        .iter()
        .map(|kind| case_for_kind(*kind))
        .collect();
    cases.push(template_case());
    cases
}

fn strategy_name(kind: BuildStrategyKind) -> &'static str {
    match kind {
        BuildStrategyKind::Bun => "bun",
        BuildStrategyKind::Rust => "rust",
    }
}

struct Fixture {
    bin_dir: PathBuf,
    repo_dir: PathBuf,
}

fn fixture(case_name: &str, label: &str) -> Fixture {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!(
        "zlorb-conformance-{}-{}-{}-{}",
        std::process::id(),
        case_name,
        label,
        n
    ));
    let bin_dir = root.join("bin");
    let repo_dir = root.join("repo");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&bin_dir).unwrap();
    std::fs::create_dir_all(&repo_dir).unwrap();
    Fixture { bin_dir, repo_dir }
}

impl Fixture {
    fn root(&self) -> PathBuf {
        self.bin_dir.parent().unwrap().to_path_buf()
    }

    fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(self.root());
    }
}

fn write_stand_in(fixture: &Fixture, case: &ConformanceCase, fail_at: Option<usize>) -> PathBuf {
    std::os::unix::fs::symlink("/bin/sh", fixture.bin_dir.join(case.runtime_name)).unwrap();
    for (index, step) in case.steps.iter().enumerate() {
        let mut body = format!(
            "echo \"call:{}\" >> invocations.log\npwd >> invocations.log\necho conformance-warning >&2\n",
            step.label
        );
        if fail_at == Some(index) {
            body.push_str("echo conformance-boom >&2\nexit 3\n");
        }
        std::fs::write(fixture.repo_dir.join(step.script), body).unwrap();
    }
    fixture.bin_dir.join(case.runtime_name)
}

fn repo_config(repo_dir: &Path) -> RepositoryConfiguration {
    RepositoryConfiguration {
        name: "demo".to_string(),
        path: repo_dir.to_string_lossy().into_owned(),
        remote: "origin".to_string(),
        branch: "master".to_string(),
        build_strategy: None,
        build_command: "".to_string(),
        restart_command: None,
        out_dir: None,
    }
}

#[test]
fn registered_strategies_resolve_to_their_conformance_cases() {
    for kind in registered_kinds() {
        let mut config = repo_config(Path::new("/tmp/demo"));
        config.build_strategy = Some(strategy_name(kind).to_string());
        let strategy = select_strategy(&config).unwrap();
        assert_eq!(case_for_strategy(&strategy).name, case_for_kind(kind).name);
    }
}

#[test]
fn conformance_success_runs_steps_in_order_in_the_repo_dir() {
    for case in registered_cases() {
        let fixture = fixture(case.name, "ok");
        let runtime = write_stand_in(&fixture, &case, None);
        let result = (case.run)(&repo_config(&fixture.repo_dir), &runtime);
        assert!(result.is_ok(), "{}: exec failed: {:?}", case.name, result);
        let log = std::fs::read_to_string(fixture.repo_dir.join("invocations.log")).unwrap();
        let repo_canon = fixture.repo_dir.canonicalize().unwrap();
        let repo_canon = repo_canon.to_string_lossy();
        let mut expected: Vec<&str> = Vec::new();
        for step in case.steps {
            expected.push(step.label);
            expected.push(repo_canon.as_ref());
        }
        let lines: Vec<String> = log
            .lines()
            .map(|line| {
                line.strip_prefix("call:")
                    .map(|label| label.to_string())
                    .unwrap_or_else(|| line.to_string())
            })
            .collect();
        let expected: Vec<String> = expected.iter().map(|line| line.to_string()).collect();
        assert_eq!(lines, expected, "{}: invocation log mismatch", case.name);
        fixture.cleanup();
    }
}

#[test]
fn conformance_step_failure_carries_step_name_and_stderr() {
    for case in registered_cases() {
        for fail_index in 0..case.steps.len() {
            let label = format!("fail{fail_index}");
            let fixture = fixture(case.name, &label);
            let runtime = write_stand_in(&fixture, &case, Some(fail_index));
            let err = (case.run)(&repo_config(&fixture.repo_dir), &runtime).unwrap_err();
            let debug = format!("{err:?}");
            let step = &case.steps[fail_index];
            assert!(
                debug.contains(step.fail_fragment),
                "{}: error does not name the failing step {}: {}",
                case.name,
                step.label,
                debug
            );
            assert!(
                debug.contains("conformance-boom"),
                "{}: error does not carry the command stderr: {}",
                case.name,
                debug
            );
            fixture.cleanup();
        }
    }
}

#[test]
fn conformance_missing_runtime_errors_instead_of_panicking() {
    for case in registered_cases() {
        let fixture = fixture(case.name, "missing");
        let missing = fixture.bin_dir.join("no-such-runtime");
        let result = (case.run)(&repo_config(&fixture.repo_dir), &missing);
        assert!(result.is_err(), "{}: missing runtime was not an error", case.name);
        fixture.cleanup();
    }
}
