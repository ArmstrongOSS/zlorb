# zlorb 🛠️

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

Prerequisites: a Rust toolchain (via [rustup](https://rustup.rs/)) and Linux with systemd.

Install both binaries with Cargo — no clone and no `just` required:

```bash
cargo install --git https://github.com/ArmstrongOSS/zlorb zlorb-ctl
cargo install --git https://github.com/ArmstrongOSS/zlorb zlorb-service
```

Then set up the daemon:

```bash
zlorb-ctl setup
```

`setup` does the rest in one step:

- creates the config directory and an empty `repositories.toml` in it (existing files are left untouched),
- writes a systemd unit whose `ExecStart` is the installed `zlorb-service` binary and which pins `ZLORB_CONFIG_DIR` to the config directory resolved at setup time,
- runs `systemctl daemon-reload` and `systemctl enable --now zlorb.service` (the `--user` variants for a normal user; run `setup` as root only if you deliberately want a system service), and for user services also tries to enable linger so the daemon starts at boot and keeps running after logout,
- exits non-zero and prints the exact commands to finish by hand when registration does not complete.

Use `zlorb-ctl setup --print-only` to write the config and unit files and print the unit and the `systemctl` commands without running them.

The unit pins the config directory resolved when `setup` runs. If you change `ZLORB_CONFIG_DIR` or `XDG_CONFIG_HOME` afterwards, re-run `zlorb-ctl setup` so the daemon and `zlorb-ctl` keep reading the same directory. Run `zlorb-ctl start` / `watch` at the same scope you ran `setup` (both as your user, or both as root). The checked-in `zlorb.service` in this repo is only a fallback template for the default user layout; the unit `setup` generates is the one that counts.

From a source checkout, the equivalent is `cargo install --path zlorb-ctl` and `cargo install --path zlorb-service`, followed by `zlorb-ctl setup` — or simply `just install`, which runs exactly those steps.

## ⚙️ Configuration

`zlorb-ctl` and the daemon resolve the config directory the same way, in this order:

1. `$ZLORB_CONFIG_DIR`, when set (the generated systemd unit sets it explicitly),
2. `$XDG_CONFIG_HOME/zlorb`,
3. `~/.config/zlorb`.

### Service Config

`<config dir>/service-config.toml` — created by the daemon with defaults on first run:

```txt
sleep_time = 5
username = "NO USERNAME PROVIDED"
token = "NO TOKEN PROVIDED"
```

`sleep_time` is the number of seconds the daemon waits between polling cycles. The daemon re-reads this file and `repositories.toml` every cycle, so edits take effect without a restart.

### Repositories Config

`<config dir>/repositories.toml`:

```txt
[[repository]]
name = "SomeRepo"
path = "/home/you/SomeRepo"
remote = "origin"
branch = "master"
build_strategy = "bun"
build_command = ""
restart_command = "./deploy.sh"
```

Set `build_strategy` (and `branch`/`remote` when they differ) after adding a repo; the daemon picks the change up on its next cycle.

`restart_command` is optional. When it is set, the daemon runs it through the shell in the repo directory after a cycle in which the repo's HEAD changed and the build (when one was needed) succeeded — the hook a long-lived app uses to restart itself on the new code. It also runs in the cycle where a build that failed in an earlier cycle finally succeeds for the current commit. It does not run when nothing changed or when the build failed. The command must return: the project's own script owns stopping the old process and starting the new one in the background (a backgrounded process does not block the daemon, even if it keeps writing to the captured output). Zlorb runs the command, logs its output, and treats a non-zero exit as a failed cycle, retrying the command on later cycles until it succeeds. Valid strategies are `"bun"` and `"rust"` (`"cargo"` is accepted as an alias for `"rust"`). The selected manager owns dependency installation and the build itself, so `build_strategy` is the only build choice to make. An unknown or missing strategy is a config error for that repo: the daemon logs it on every reload, and when a build is triggered for that repo it fails with that error instead of guessing a manager.

Migration from older configs: a repo with no `build_strategy` keeps working only when its legacy `build_command` is `"bun"`, `"cargo"`, or `"cargo build"` (matched exactly, ignoring case and surrounding whitespace). Any other legacy value (for example `"cargo build --release"` or `"bun run build"`) no longer selects a manager by substring; add an explicit `build_strategy` line to that repo instead.

## 🖱️ Usage

Manage repositories with `zlorb-ctl` commands:

```bash
# Add the current git repository
zlorb-ctl add

# List all configured repositories
zlorb-ctl list

# Remove a repository
zlorb-ctl remove --repo-name my-repo
```

## 🚀 Deployment

The daemon is the systemd service registered by `zlorb-ctl setup`: a per-user service for normal installs, running in the foreground with automatic recovery on failure.

## 🌟 Getting Started

1. Install zlorb as described above and run `zlorb-ctl setup`.
2. From inside a git repository, run `zlorb-ctl add`.
3. Edit `build_strategy` for the repo in `<config dir>/repositories.toml` (`"bun"` or `"rust"`).
4. Monitor build logs via `journalctl --user -u zlorb` (`journalctl -u zlorb` for a system service).

zlorb will automatically keep your repositories updated and built, saving you time and effort.

## 🤝 Contributing
Want to contribute? See [CONTRIBUTING.md](CONTRIBUTING.md) for the branch model, the build and test requirements, and how changes are reviewed. Pull requests and feedback are welcome! To add support for a new build toolchain, follow [Adding a build manager](https://github.com/ArmstrongOSS/zlorb/blob/dev/docs/build-managers.md): it covers the strategy pattern, the copyable manager template, and the conformance checklist a new manager must pass.

Zlorb is licensed under the [MIT License](LICENSE).

## Roadmap
- [x] watch ctl flag that triggers journalctl log watcher
- [ ] web output ctl flag that spins up web server
- [x] need to load configs into memory so daemon doesnt need to keep reading file system every iteration
