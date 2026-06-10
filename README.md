# zlorb 🛠️

[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/tristonarmstrong/zlorb/1-overview)

A lightweight, systemd-managed continuous integration tool for Git-based projects. zlorb monitors repositories, detects changes, and triggers builds using Bun, 
keeping your projects up-to-date effortlessly.

## 📖 Overview
zlorb is a Rust-based system with three components: `zlorb-service` (the monitoring daemon), `zlorb-ctl` (a CLI for easy management), and `zlorb-lib` (shared functionality). 
It watches Git repositories for updates, performs safe fast-forward merges, and runs build commands when changes are detected or build artifacts are missing.

## 🔑 Key Features
- 🔍 **Automatic Change Detection**: Tracks repository updates using the `git2` library.
- 🔒 **Safe Git Operations**: Ensures only fast-forward merges are applied.
- 🏗️ **Build Triggering**: Executes Bun builds when changes occur or `dist/` is missing.
- ⚙️ **Systemd Integration**: Runs reliably with automatic restarts.
- 🖥️ **CLI Management**: Easily add, remove, or list repositories without service restarts.

## 🛠️ Installation
Clone the repository:
```bash
git clone <repository-url>
```

Build and install:
```bash
just build
sudo systemctl enable zlorb
sudo systemctl start zlorb
```

## ⚙️ Configuration
zlorb uses JSON configuration files stored in `~/.config/zlorb/`. The global `service-config.json` sets the monitoring interval, while per-repository `config.json` 
files define repository-specific settings.

Example `service-config.json`:
```json
{
  "sleep_time": 60
}
```

Example repository `config.json`:
```json
{
  "name": "my-repo",
  "path": "/path/to/repo",
  "branch": "main",
  "remote": "origin",
  "build_command": "bun build"
}
```

## 🖱️ Usage
Manage repositories with `zlorb-ctl` commands:
```bash
# Add a repository
zlorb-ctl add --name my-repo --path /path/to/repo --branch main --remote origin --build-command "bun build"

# List all configured repositories
zlorb-ctl list

# Remove a repository
zlorb-ctl remove my-repo
```

## 🚀 Deployment
The `justfile` handles building and installing binaries to `/usr/local/bin/` and the systemd unit file to `/usr/lib/systemd/system/`. 
The service runs in the foreground with automatic recovery on failure, ensuring reliable operation.

## 🌟 Getting Started

1. Install zlorb as described above.
1. Configure your repositories using `zlorb-ctl add`.
1. Start the service with `sudo systemctl start zlorb`.
1. Monitor build logs via `journalctl -u zlorb`.

zlorb will automatically keep your repositories updated and built, saving you time and effort.

## 🤝 Contributing
Want to contribute? Check the System Architecture and Workspace Structure for details. Pull requests and feedback are welcome!

## Roadmap
- [ ] watch ctl flag that triggers journalctl log watcher
- [ ] web output ctl flag that spins up web server
- [ ] benchmark to investigate which is more performant (git pull | git fetch -> pull when needed)
- [ ] need to load configs into memory so daemon doesnt need to keep reading file system every iteration
