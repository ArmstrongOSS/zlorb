const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m"; // Reset to default

pub struct Logger {}

impl Logger {
    pub fn info(msg: String) {
        println!("[{}INFO{}]: {}", GREEN, RESET, msg);
    }

    pub fn error(msg: String) {
        println!("[{}ERROR{}]: {}", RED, RESET, msg);
    }

    pub fn debug(msg: String) {
        println!("[{}DEBUG{}]: {}", YELLOW, RESET, msg);
    }
}
