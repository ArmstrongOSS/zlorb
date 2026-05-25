use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct ServiceConfig {
    pub sleep_time: u64,
    username: String,
    token: String,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            sleep_time: 5,
            username: "NO USERNAME PROVIDED".into(),
            token: "NO TOKEN PROVIDED".into(),
        }
    }
}
