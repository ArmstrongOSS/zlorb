mod utils;
use crate::utils::{daemon::DaemonManager, repo};
use clap::{Parser, Subcommand};

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
}

fn main() {
    // when running the program, use RUST_LOG with (error, info, debug)
    colog::init();

    let args = Args::parse();

    match args.cmd {
        Commands::Add => repo::add(),
        Commands::List => repo::list(),
        Commands::Start => DaemonManager::start().unwrap(),
        Commands::Remove { repo_name } => repo::remove(repo_name),
        Commands::Watch => repo::watch(),
    }
}
