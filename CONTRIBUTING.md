# Contributing to Zlorb

Zlorb is licensed under the MIT License (see `LICENSE`). By contributing, you
agree that your contributions are licensed under the same terms.

## Branch model

- `master` is the release branch. Do not open pull requests against it and do
  not push to it directly.
- All work targets `dev`. Open your pull request against `dev`; `dev` is
  merged into `master` by the maintainer.
- Branch from the current tip of `dev` and name the branch for the work,
  for example `fix/<issue-number>-<short-slug>`.

## Before you open a pull request

- Open or find an issue for the change first, so the scope is agreed before
  code is written. Keep each pull request to one focused change.
- Reference the issue in the pull request body (for example `Closes #N`).
  Because pull requests target `dev` rather than the default branch, GitHub
  does not close the issue when the pull request merges into `dev`; the
  maintainer closes it once the change is in `dev`.

## Build and tests

A Rust toolchain is required (via [rustup](https://rustup.rs/)). Before
asking for review, both of these must pass from the workspace root:

```bash
cargo build
cargo test
```

Do not weaken or delete existing tests to make them pass. Add tests for new
behavior.

## Adding a build manager

Support for a new build toolchain goes through the strategy pattern, not
through the daemon's polling or git-update loop. Follow
[Adding a build manager](docs/build-managers.md): it covers the shared
`Manager::exec` contract, the copyable manager template, and the conformance
checklist a new manager must pass. Add a manager only when a real project
needs it — do not add one for a toolchain no watched project uses.
