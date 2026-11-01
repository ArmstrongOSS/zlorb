# Adding a build manager

Zlorb builds projects through the strategy pattern. The daemon's polling and
git-update loop is technology-agnostic: it decides *when* a build is needed,
and a build manager decides *how* one project's build runs. Add a manager only
when a real project needs it. Do not add a manager for a toolchain no watched
project uses.

Bun (`build_strategy = "bun"`) and Rust (`build_strategy = "rust"`, alias
`"cargo"`) are the existing managers. The contract below is the rule for a
*new* manager. The two existing managers predate parts of it and diverge
from it in named places, listed under "Where the existing managers diverge"
— a new manager follows the contract and the template, not those
divergences.

## How selection works

1. A repo entry in `repositories.toml` names its strategy explicitly in
   `build_strategy`. `RepositoryConfiguration::resolved_build_strategy()` in
   `zlorb-lib/src/config.rs` parses that string into a `BuildStrategyKind`.
   For an explicit `build_strategy` the match is exact, after trimming
   and lowercasing: there is no substring detection, and an unknown
   strategy is an `InvalidConfig` error for that repo, never a guessed
   manager. A missing or blank `build_strategy` falls through to the
   legacy path, where a `build_command` of exactly `"bun"`, `"cargo"`,
   or `"cargo build"` still selects a manager; that legacy match is
   exact and case-sensitive.
2. `select_strategy()` in
   `zlorb-service/src/build_system_executor/mod.rs` maps each
   `BuildStrategyKind` to one `BuildStrategy` variant holding one manager.
3. `BuildSystemExecutor::run_build()` matches on that variant and calls the
   shared `Manager::exec(config)` in
   `zlorb-service/src/build_system_executor/managers/manager.rs`. The
   executor, `RepoProcessor`, and the polling loop contain no per-toolchain
   logic beyond that match.

## The `Manager::exec` contract

`exec(&self, config: &RepositoryConfiguration) -> Result<(), ZlorbError>`
receives the repo's full configuration. `config.path` is the repo working
tree every command runs in. `config.name` is the repo's configured name;
the existing managers and the template do not read it in `exec`, and their
build errors identify the failing step and runtime rather than the repo.

A manager must:

- Discover its runtime before use and return an error when the runtime is
  not installed, naming the runtime and how the lookup failed.
- Install the project's dependencies as its first build step, when the
  toolchain has a separate install step.
- Run every toolchain command with the repo path (`config.path`) as the
  working directory.
- Check the exit status of every command it runs. A non-zero exit is an
  error that includes the step name and the command's stderr.
- Return every failure as a `ZlorbError` (`ZlorbError::Io` for spawn and
  I/O failures, `ZlorbError::Other` for runtime-missing, bad-status, and
  other build failures). Log the failure with `Logger::error` before
  returning it, as `BunManager` does.

A manager must not:

- Panic. No `unwrap`, `expect`, or indexing on fallible values in the
  `exec` path; convert failures to `ZlorbError` instead.
- Touch git or update logic. Fetching, fast-forward checks, checkout, and
  the decision to build at all belong to `RepoProcessor`. A manager only
  builds the tree it is given.
- Read or write Zlorb config files, watch lists, or service state.
- Decide whether a build is needed. Missing-output-directory and
  remote-ahead checks happen before `exec` is called.
- Shell out through a shell string. Spawn the runtime binary directly with
  argument arrays, as the existing managers do.

## Where the existing managers diverge

These are facts about the current code, recorded so a new manager does not
copy them. Fixing them is separate work from adding a manager.

- `BunManager` discovers its runtime at a fixed per-user path
  (`~/.bun/bin/bun`), not on `PATH`. Its install and build steps check
  each command's exit status, log failures with `Logger`, and return an
  error carrying the step name and the command's stderr, and both run
  with `config.path` as given as the working directory.
- `RustManager` has no runtime discovery and no install step: it spawns
  the bare name `cargo` (resolved through `PATH` by the operating system).
  Its build step does check the exit status, logs failures with
  `Logger`, and returns an error carrying the command's stderr.

## Adding a manager, step by step

1. Copy `zlorb-service/src/build_system_executor/managers/template.rs` to
   `zlorb-service/src/build_system_executor/managers/<name>.rs`. Rename
   `TemplateManager` to your manager, replace the `template-runtime`
   placeholder with your runtime's binary name, and replace the
   `install` / `run build` argument lists with your toolchain's dependency
   install and build commands. Delete any step your toolchain does not
   have; if it has no separate install step, `exec` calls the build step
   only. The file is self-contained: its helper functions are private to
   the file and its unit tests live in the same file, so the copy carries
   its tests with it and they run under `cargo test` once the module is
   declared. Rename the `template-runtime` literal in the tests to match
   the new runtime name.
2. Register the strategy string in `zlorb-lib`:
   - Add a variant to `BuildStrategyKind` in `zlorb-lib/src/config.rs`.
   - Map the strategy string to it in `resolved_build_strategy()`. Decide
     any alias up front (the way `"cargo"` aliases `"rust"`) and name the
     full accepted set, aliases included, in the error messages for that
     module.
   - Add unit tests there: the new string selects the new kind, the alias
     (if any) does too, and the string still errors when it appears only
     as a substring of a longer command.
3. Register the manager in `zlorb-service`:
   - Declare the module in
     `zlorb-service/src/build_system_executor/managers/mod.rs` as a normal
     `pub mod`, not a `#[cfg(test)]` declaration. This step is what puts
     the manager in the shipped daemon: a module left under
     `#[cfg(test)]` exists only in test builds and the daemon will never
     run it.
   - Add a `BuildStrategy` variant holding your manager in
     `managers/manager.rs`, and add your manager to the `use` block at
     the top of that file.
   - In `build_system_executor/mod.rs`: add your manager to the `use`
     block at the top of the file, add the matching arm in
     `select_strategy()`, add the matching arm in
     `BuildSystemExecutor::run_build()`, and add a selection test next
     to `bun_strategy_selects_bun_manager`. Both matches are exhaustive,
     so the compiler stops you if an arm is missing — a build that
     passes with the variant added has both arms in place.
4. Document the new value in both places `README.md` lists strategies:
   the `build_strategy` paragraph under Configuration and the Getting
   Started step that names the valid values. Include any alias in both.
5. Run the conformance checklist below against the finished manager, then
   `cargo build` and `cargo test`. Both must pass.

## Conformance checklist

Verify each item by reading the manager or by a test, not by assumption:

- [ ] Runtime discovery: the manager locates its runtime itself and
      returns a `ZlorbError` naming the runtime when it is absent. A
      `PATH` lookup accepts only a file that is executable. A test
      proves the absent-runtime case errors instead of panicking (the
      template's `find_in_path` tests are the pattern to copy).
- [ ] Dependency install: dependencies are installed inside the repo path
      before the build step, or the toolchain has no separate install step
      and `exec` calls the build step only.
- [ ] Repo working directory: every spawned command sets its working
      directory to `config.path`.
- [ ] Status checked: every command's exit status is checked, and a
      non-zero status becomes an error carrying the step name and stderr.
      A test with a failing stand-in command proves this (the template's
      `exec` tests, which drive a stand-in runtime that records its
      working directory and fails with marked stderr, are the pattern to
      copy — they also prove the install step runs before the build step
      and that both run in the repo directory).
- [ ] Errors, not panics: the `exec` path contains no `unwrap`, `expect`,
      or panic, and every failure returns a `ZlorbError` and is logged.
- [ ] Build only: the manager contains no git operations, no config-file
      access, and no build-needed decision.
- [ ] No shell strings: commands are spawned as a binary plus an argument
      array, never through a shell.
- [ ] Selection wired end to end: the strategy string resolves in
      `zlorb-lib`, `select_strategy()` returns the new variant, and tests
      cover both.
- [ ] No new manager shipped unused: the strategy exists because a real
      project is being watched with it.

## The template

`managers/template.rs` is a complete manager for a placeholder runtime
named `template-runtime` that installs with `template-runtime install` and
builds with `template-runtime run build`. In this repo it is wired into
`managers/mod.rs` under `#[cfg(test)]` only: `cargo test` compiles it
against the `Manager` trait and runs its conformance tests, while the
shipped daemon gains no new strategy and `template` is not an accepted
`build_strategy` value. The template compiles against the trait as it
stands in the test build; turning a renamed copy of it into a real manager
takes the registration steps above, including declaring its module as a
normal `pub mod`.
