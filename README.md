# Bodh

Bodh is a Linux native workspace for reviewing GitHub pull requests from an existing local checkout. The build specification is [issue #1](https://github.com/kedarmd/bodh/issues/1). The shared Rust CLI and native application use the same pinned-source, workspace, validation, and publication services.

## Install and run

From a Bodh source checkout, run:

```sh
./scripts/install-local
```

This builds with the pinned Cargo lockfiles and installs `bodh` (headless CLI), `bodh-gui` (native app), and a graphical-session guard under `~/.local`. It adds a `Bodh` desktop entry under `$XDG_DATA_HOME/applications` or `~/.local/share/applications` when that variable is unset. Use `--prefix /absolute/path` to choose another executable prefix. Launch the GUI from the desktop menu or run `bodh-gui`; `bodh-gui --open --repo /path/to/checkout --pr 123` opens a review directly. The CLI works without a display, for example `bodh contract describe` from any directory.

The app stores its local review database and navigation preferences under `$XDG_STATE_HOME/bodh/`, or `~/.local/state/bodh/` when `XDG_STATE_HOME` is unset. It currently creates no Bodh-specific config or cache directory. The CLI takes an explicit `--state` path for workspace commands. Bodh does not store GitHub credentials; GitHub CLI (`gh`) owns authentication. The user-local install can be removed by deleting `~/.local/bin/bodh`, `bodh-gui`, `bodh-gui-launcher`, and `$XDG_DATA_HOME/applications/bodh.desktop` (or `~/.local/share/applications/bodh.desktop` when `XDG_DATA_HOME` is unset, and the corresponding custom-prefix executables).

## Support boundary

The current build environment is CachyOS rolling, x86_64, Linux kernel `7.2.7-1-cachyos`, Rust `1.99.0`, and GPUI `0.2.2`; the desktop session exposes Wayland. This records the development environment, not qualification of every Arch derivative or Wayland compositor. X11, other distributions, Enterprise GitHub hosts, and other native backends remain unqualified. The native app requires Git, authenticated GitHub CLI, Linux desktop runtime libraries (including fontconfig, freetype, xkbcommon, Wayland/X11 and Vulkan libraries as applicable), and an available graphical session. The launcher checks for an active display and reports exact missing shared-library names before starting. The user-local packaging smoke check does not replace a native desktop launch and visual trial.

See [TESTING.md](TESTING.md) for the native dependency list, CI coverage, support limitations, and validation commands. Run `./scripts/validate` for core checks or `./scripts/validate desktop` for native typecheck, lint, and tests.
