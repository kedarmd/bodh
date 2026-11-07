# GitHub CLI preflight and repository-path diagnosis

## User report and reproduction

The native app showed `GH_UNAVAILABLE` for the entered repository path `~/dev/nayati`, although GitHub CLI was installed.

The app starts `gh pr list` with the selected checkout as the child process working directory. Rust does not expand `~` in `Command::current_dir`. The old code mapped any spawn failure to `GH_UNAVAILABLE`, so the missing working directory was misreported as a missing executable.

Reproduction with the exact input and same argv shape:

```text
cwd='~/dev/nayati': GH_UNAVAILABLE reproduction; spawn errno=2
cwd='/home/kedarmd/dev/nayati': spawn succeeded; exit=0
```

## Changes

- The home screen asynchronously checks `gh --version` and shows Checking, the installed version, a missing-CLI install prompt, or a check failure. Repository opening is gated while the check is pending or the executable is missing; a retry action is available.
- Repository inputs expand `~` and `~/...`, canonicalize existing directories, and persist the absolute path.
- Invalid/missing checkout directories return `REPOSITORY_PATH_INVALID`. Only failure to spawn an absent executable returns `GH_UNAVAILABLE`; other spawn errors use `GH_START_FAILED`.
- Authentication and repository access failures continue to be reported after `gh` starts, with their own categories.

## Verification

- `cargo test --manifest-path spikes/native-sequence/Cargo.toml --no-default-features gh_prerequisite` passed 5 focused path/preflight tests.
- `cargo test --locked --manifest-path spikes/native-sequence/Cargo.toml tilde_repository_is_expanded_before_the_pr_list_child_is_spawned` passed the exact production PR-list path with a controlled `gh` executable and no network.
- `CARGO_BUILD_JOBS=2 ./scripts/validate desktop` passed. Raw output: [VALIDATE-GH-PREREQUISITE.log](VALIDATE-GH-PREREQUISITE.log).

The preflight verifies the executable and version only. Authentication and per-repository access are checked when the reviewer opens that repository.

## Freshness follow-up — 2026-11-05

The user later reported that opening a PR succeeded but the workspace freshness banner showed `GH_UNAVAILABLE`. The cause was a value mix-up: `GhCliStatus::Available` stored the `gh --version` text, while freshness, load-latest preview and apply-latest treated that value as the executable path. The regression test failed with the version string on the left and the controlled executable path on the right. `Available` now stores an executable `PathBuf` and version separately; focused preflight tests and full desktop validation pass. A release GUI build compiled after this fix. The screenshots reviewed since then covered the Changes workspace, so a direct live check of the freshness action/banner remains outstanding.
