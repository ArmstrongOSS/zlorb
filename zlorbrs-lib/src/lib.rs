pub mod config;
pub mod error;

use log::error;

pub fn get_home_dir() -> String {
    let home_dir = match std::env::home_dir() {
        Some(x) => String::from(x.to_str().unwrap()),
        None => {
            error!("Failed to get the home directory");
            panic!("Program exited due to previous error");
        }
    };
    home_dir
}

pub mod shared_test_utils {
    use std::sync::Mutex;
    pub static ENV_MUTEX: Mutex<()> = Mutex::new(());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_get_home_dir_success() {
        let _lock = shared_test_utils::ENV_MUTEX.lock().unwrap();

        // Mock HOME so it always succeeds
        let mut tmp_dir = env::temp_dir();
        tmp_dir.push("zlorbrs_lib_home_test");
        std::fs::create_dir_all(&tmp_dir).unwrap();

        unsafe {
            env::set_var("HOME", tmp_dir.to_str().unwrap());
        }

        let home = get_home_dir();
        assert_eq!(home, tmp_dir.to_str().unwrap());

        let _ = std::fs::remove_dir_all(tmp_dir);
    }
}
