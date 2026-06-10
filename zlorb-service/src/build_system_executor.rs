use std::{path::Path, process::Stdio};

///  An isolated utility service responsible for running external shell commands (the build step).
pub struct _BuildSystemExecutor {
    stdout_pipe: Stdio,
}

impl _BuildSystemExecutor {
    fn _execute(_target_path: &Path, _command: &str) {}
}
