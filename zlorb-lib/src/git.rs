use git2::Repository;
use log::error;

pub fn fetch_update(repo_path: String) {
    let repo: Option<Repository> = match Repository::open(repo_path) {
        Ok(repo) => Some(repo),
        Err(e) => {
            error!("failed to init: {}", e);
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
