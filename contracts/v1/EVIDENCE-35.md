# Installation and desktop integration — issue #35

## Installation path

`scripts/install-local` builds the pinned headless CLI and native executable from the repository's lockfiles, then copies them to an absolute user prefix. The default is `~/.local`; `--prefix` selects a custom absolute path. The install exposes `bodh` for headless CLI work and `bodh-gui` for native review. It writes a `Bodh` desktop entry under `$XDG_DATA_HOME/applications` (default `~/.local/share/applications`) with the matching GPUI app ID `bodh`.

The installed GUI launcher checks that a Wayland or X11 display is available, reports linked libraries missing from the installed binary, and forwards its arguments to `bodh-gui`. The home screen checks for GitHub CLI; repository acquisition reports Git, authentication, and remote-access errors through the shared service. Credentials remain under GitHub CLI. No daemon is installed.

Review database and navigation preferences use `$XDG_STATE_HOME/bodh/`, defaulting to `~/.local/state/bodh/`. The app currently creates no Bodh-specific config or cache directory. CLI workspace commands take an explicit `--state` path. Native runtime libraries and build prerequisites are described in `TESTING.md` and the root README; exact support remains limited to recorded environments.

## Verification status

- `python3 -m unittest scripts/tests/test_install_local.py -v` passed. The test exercises the public installer with a fake Cargo build: staged artifacts, desktop entry identity/path escaping, invalid XDG path rejection, CLI execution from another directory, missing-display guidance, missing-library guidance, and GUI argument forwarding. `./scripts/validate` discovers this test automatically.
- `bash -n scripts/install-local packaging/bodh-gui-launcher` and `git diff --check` passed. Full runner/desktop validation, actual release artifact installation, and installed-binary CLI smoke invocation remain pending the integration validation window.
- A native desktop launch through the installed entry point is not claimed. The #32 review app is currently open for the user's visual review; no build or second GUI launch was run so it would not compete for that session.
- The current development host is CachyOS rolling x86_64, kernel `7.2.7-1-cachyos`, Rust `1.99.0`, GPUI `0.2.2`, with a Wayland session exposed. This is an environment record, not distro/compositor qualification. X11, other distributions and Enterprise GitHub hosts remain unqualified.

This ticket must remain open until the pending install and desktop launch checks are recorded; compilation alone does not establish a native launch or support matrix.
