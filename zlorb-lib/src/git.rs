use git2::Repository;

use crate::log::Logger;

/// Returns true if the given path opens as a git repository.
pub fn is_git_repo(path: &str) -> bool {
    Repository::open(path).is_ok()
}

pub fn fetch_update(repo_path: String) {
    let _repo: Option<Repository> = match Repository::open(repo_path) {
        Ok(repo) => Some(repo),
        Err(e) => {
            Logger::error(format!("failed to init: {}", e));
            None
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;

    #[test]
    fn test_fetch_update_valid_repo() {
        // Initialize an empty repository
        let mut tmp_dir = env::temp_dir();
        tmp_dir.push("zlorb_git_test_valid");
        let _ = fs::remove_dir_all(&tmp_dir);
        fs::create_dir_all(&tmp_dir).unwrap();

        let _repo = Repository::init(&tmp_dir).unwrap();

        // Should load the repo and not error (no panic/return value to check right now)
        fetch_update(tmp_dir.to_str().unwrap().to_string());

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_fetch_update_invalid_repo() {
        // Directory that is not a git repository
        let mut tmp_dir = env::temp_dir();
        tmp_dir.push("zlorb_git_test_invalid");
        let _ = fs::remove_dir_all(&tmp_dir);
        fs::create_dir_all(&tmp_dir).unwrap();

        // Should log an error but not panic
        fetch_update(tmp_dir.to_str().unwrap().to_string());

        let _ = fs::remove_dir_all(&tmp_dir);
    }
}
