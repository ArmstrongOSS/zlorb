use std::fmt;

pub enum Logger {
    Info(String),
    Debug(String),
    Error(String),
}

impl fmt::Display for Logger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Logger::Info(msg) => Ok(println!("[INFO]: {}", msg)),
            Logger::Debug(msg) => Ok(println!("[DEBUG]: {}", msg)),
            Logger::Error(msg) => Ok(println!("[ERROR]: {}", msg)),
        }
    }
}
