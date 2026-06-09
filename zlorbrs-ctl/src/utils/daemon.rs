use log::info;
use std::process::{Command, ExitStatus};
use zlorbrs_lib::error::ZlorbError;

/// Responsible for starting/stopping/restarting the zlorb service daemon
pub struct DaemonManager {}
impl DaemonManager {
    pub fn start() -> Result<(), ZlorbError> {
        let out_status = DaemonManager::run_process_check()?;
        let is_running = out_status.success();
        match is_running {
            true => DaemonManager::run_service_start_cmd()?,
            false => DaemonManager::run_service_restart_cmd()?,
        };
        Ok(())
    }

    fn run_process_check() -> Result<ExitStatus, ZlorbError> {
        Command::new("systemctl")
            .args(["is-active", "--quiet", "zlorbrs"])
            .status()
            .map_err(ZlorbError::Io)
    }

    fn run_service_start_cmd() -> Result<ExitStatus, ZlorbError> {
        info!("Starting the daemon");
        let out = Command::new("systemctl")
            .args(["start", "zlorbrs"])
            .status()
            .map_err(ZlorbError::Io);
        info!("...Started");
        out
    }

    fn run_service_restart_cmd() -> Result<ExitStatus, ZlorbError> {
        info!("Daemon already running, restarting it");
        let out = Command::new("systemctl")
            .args(["restart", "zlorbrs"])
            .status()
            .map_err(ZlorbError::Io);
        info!("...Restarted");
        out
    }
}
