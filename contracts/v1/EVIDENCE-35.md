# Installation and desktop integration — issue #35

## Installation path

`scripts/install-local` builds the pinned headless CLI and native executable from the repository's lockfiles, then copies them to an absolute user prefix. The default is `~/.local`; `--prefix` selects a custom absolute path. The install exposes `bodh` for headless CLI work and `bodh-gui` for native review. It writes a `Bodh` desktop entry under `$XDG_DATA_HOME/applications` (default `~/.local/share/applications`) with the matching GPUI app ID `bodh`.

The installed GUI launcher checks that a Wayland or X11 display is available, reports linked libraries missing from the installed binary, and forwards its arguments to `bodh-gui`. The home screen checks for GitHub CLI; repository acquisition reports Git, authentication, and remote-access errors through the shared service. Credentials remain under GitHub CLI. No daemon is installed.

Review database and navigation preferences use `$XDG_STATE_HOME/bodh/`, defaulting to `~/.local/state/bodh/`. The app currently creates no Bodh-specific config or cache directory. CLI workspace commands take an explicit `--state` path. Native runtime libraries and build prerequisites are described in `TESTING.md` and the root README; exact support remains limited to recorded environments.

## Verification status

- `python3 -m unittest scripts/tests/test_install_local.py -v` passed. The test exercises the public installer with a fake Cargo build: staged artifacts, desktop entry identity/path escaping, invalid XDG path rejection, CLI execution from another directory, missing-display guidance, missing-library guidance, and GUI argument forwarding. This focused Python test is run separately; `./scripts/validate` does not currently discover it.
- `bash -n scripts/install-local packaging/bodh-gui-launcher` and `git diff --check` passed. The integrated `./scripts/validate desktop` run also passed; see [VALIDATE-32-final.log](VALIDATE-32-final.log). The real release installer completed into `/tmp/bodh-install-review/prefix`; its build/copy output is [INSTALL-35.log](INSTALL-35.log). From `/tmp`, the installed `bodh --json contract describe` reported complete prepared-review support, and `desktop-file-validate` accepted the generated Bodh entry.
- The installed `bodh-gui-launcher --open --repo /home/kedarmd/dev/kmDot --pr 103` started the native app from `/tmp`; PID 919955 and window geometry are recorded in [GUI-32-REVISED.log](GUI-32-REVISED.log). The app was opened for #32 visual review and the user accepted the layout. A click from the desktop menu itself was not automated.
- The current development host is CachyOS rolling x86_64, kernel `7.2.7-1-cachyos`, Rust `1.99.0`, GPUI `0.2.2`, with a Wayland session exposed. This is an environment record, not distro/compositor qualification. X11, other distributions and Enterprise GitHub hosts remain unqualified.

The user-local install, out-of-checkout CLI invocation, generated desktop entry, and installed GUI launcher have been exercised on this Wayland host. X11, other distributions and compositors, and final release support claims remain unqualified; issue #35 stays open under the draft integration PR until the full V1 graph is complete.
