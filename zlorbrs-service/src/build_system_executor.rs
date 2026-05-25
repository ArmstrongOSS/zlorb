use std::{path::Path, process::Stdio};

pub struct BuildSystemExecutor {
    stdout_pipe: Stdio,
}

impl BuildSystemExecutor {
    fn execute(target_path: &Path, command: &str) {}
}
