mod utils;
use crate::utils::{daemon, repo};
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Subcommand, Debug, Clone)]
enum Commands {
    Remove {
        #[arg(short, long)]
        repo_name: String,
    },
    Add,
    List,
    Start,
    Watch,
}

fn main() {
    // when running the program, use RUST_LOG with (error, info, debug)
    colog::init();

    let args = Args::parse();

    match args.cmd {
        Commands::Add => repo::add(),
        Commands::List => repo::list(),
        Commands::Start => daemon::start(),
        Commands::Remove { repo_name } => repo::remove(repo_name),
        Commands::Watch => repo::watch(),
    }
}
