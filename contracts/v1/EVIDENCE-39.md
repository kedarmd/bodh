# Issue #39 — Changes workspace visual follow-up

## Implemented

- Changed-file rows now impose a 48 px minimum and center the two-line filename/directory group vertically. The selected row keeps its raised background and now has a persistent accent border.
- Base/Head source buttons use the persistent selected-side background in addition to the accent border, distinct from transient focus.
- An open draft editor changes the source toolbar action to `Draft comment · Base` or `Draft comment · Head` and applies a persistent selected treatment, making its source relationship visible.
- Unified and split diff virtual lists now request the full width of their containing pane. Split rows retain equal flexible side cells and a one-pixel visible divider.

## Validation

`CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/home/kedarmd/dev/bodh/target ./scripts/validate desktop` passed on the dirty issue worktree at base `ecca831e466567f51e66fe9e94e03c92fad5ef3f`, using Rust/Cargo 1.99.0. The complete runner output is in [VALIDATE-39-desktop.log](VALIDATE-39-desktop.log). The runner's application, native GPUI checks and tests passed. One first run failed because the separate isolated `/tmp` Cargo targets exhausted the 3.7 GiB tmpfs; after cleaning only those generated targets and using the existing repository Cargo target, a flaky GitHub CLI preflight test failed once and passed on focused rerun and the subsequent full desktop run.

## Remaining native qualification

No GUI was launched. Per `TESTING.md`, native acceptance still requires a coordinated product-mode trial at 947×505 and during resize, light/dark themes, unified/split modes, keyboard focus and selection, with a fresh render and the user's explicit acceptance. This implementation and desktop validation do not claim that visual criterion passed.
