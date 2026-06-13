mod utils;
use crate::utils::{daemon::DaemonManager, repo};
use clap::{Parser, Subcommand};
use zlorb_lib::error::ZlorbError;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Subcommand, Debug, Clone)]
enum Commands {
    /// remove a repo from the configs watched by the daemon @ ~/.config/zlorb/configs
    Remove {
        #[arg(short, long)]
        repo_name: String,
    },
    /// adds a repo to the configs watched by the daemon @ ~/.config/zlorb/configs
    Add,
    /// list repos currently watched by the daemon @ ~/.config/zlorb/configs
    List,
    /// starts the repo watch daemon
    Start,
    /// runs a journalctl watcher to see realtime logs
    Watch,
    /// runs the zlorb-web binary to start a web server;
    ///
    /// This is being compiled as a dependency to ctl because the expectation
    /// is that if youre going to use the web ui you dont need to use the ctl
    Web,
}

fn main() -> Result<(), ZlorbError> {
    let args = Args::parse();

    let res = match args.cmd {
        Commands::Add => repo::add(),
        Commands::List => repo::list(),
        Commands::Start => DaemonManager::start(),
        Commands::Remove { repo_name } => repo::remove(repo_name),
        Commands::Watch => repo::watch(),
        Commands::Web => {
            zlorb_web::run();
            Ok(())
        }
    };

    if let Err(e) = res {
        e.print();
    }
    Ok(())
}
