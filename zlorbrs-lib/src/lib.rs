pub mod config;
pub mod error;

use std::path::PathBuf;

use log::error;

pub fn get_home_dir() -> PathBuf {
    let home_dir = match std::env::home_dir() {
        Some(x) => x,
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
