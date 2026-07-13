use std::process::{Command, ExitStatus};
use zlorb_lib::{error::ZlorbError, log::Logger};

/// Responsible for starting/stopping/restarting the zlorb service daemon
pub struct DaemonManager {}
impl DaemonManager {
    pub fn start() -> Result<(), ZlorbError> {
        let out_status = DaemonManager::run_process_check()?;
        let is_running = out_status.success();
        match is_running {
            true => DaemonManager::run_service_restart_cmd()?,
            false => DaemonManager::run_service_start_cmd()?,
        };
        Ok(())
    }

    fn run_process_check() -> Result<ExitStatus, ZlorbError> {
        Command::new("systemctl")
            .args(["is-active", "--quiet", "zlorb"])
            .status()
            .map_err(ZlorbError::Io)
    }

    fn run_service_start_cmd() -> Result<ExitStatus, ZlorbError> {
        Logger::info("Starting the daemon".into());
        let out = Command::new("systemctl")
            .args(["start", "zlorb"])
            .status()
            .map_err(ZlorbError::Io);
        Logger::info("...Started".into());
        out
    }

    fn run_service_restart_cmd() -> Result<ExitStatus, ZlorbError> {
        Logger::info("Daemon already running, restarting it".into());
        let out = Command::new("systemctl")
            .args(["restart", "zlorb"])
            .status()
            .map_err(ZlorbError::Io);
        Logger::info("...Restarted".into());
        out
    }
}
