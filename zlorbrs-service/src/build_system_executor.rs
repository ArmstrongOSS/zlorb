use std::{path::Path, process::Stdio};

///  An isolated utility service responsible for running external shell commands (the build step).
pub struct BuildSystemExecutor {
    stdout_pipe: Stdio,
}

impl BuildSystemExecutor {
    fn execute(target_path: &Path, command: &str) {}
}
