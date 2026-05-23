use std::process::{self, Command};

use log::{error, info};
use zlorbrs_lib::error::ZlorbError;

pub(crate) fn start() -> Result<(), ZlorbError> {
    let mut _output = Command::new("systemctl")
        .args(&["is-active", "--quiet", "zlorbrs"])
        .status()
        .map_err(|e| ZlorbError::Io(e))?;
    let is_running = _output.success();
    if !is_running {
        info!("Starting the daemon");
        _output = Command::new("systemctl")
            .args(&["start", "zlorbrs"])
            .status()
            .map_err(|e| ZlorbError::Io(e))?;
        info!("...Started");
    } else {
        info!("Daemon already running, restarting it");
        _output = Command::new("systemctl")
            .args(&["restart", "zlorbrs"])
            .status()
            .map_err(|e| ZlorbError::Io(e))?;
        info!("...Restarted");
    }

    Ok(())
}
