# Issue #16 keyboard focus evidence

The shared native button helper now participates in GPUI tab traversal. `Tab` and `Shift-Tab` invoke GPUI's next/previous focus traversal across all Bodh screens, including Home, Repository, workspace tabs, and Finish Review. Focused buttons keep the approved D2 accent border as their visible focus treatment. Search and editor focus handles are tab stops as well, so keyboard traversal can reach text controls when they are rendered.

## Automated interaction check

`product::keyboard_focus_tests::tab_and_shift_tab_move_focus_between_visible_controls` mounts a GPUI test view with two focusable controls, dispatches `Tab`, `Tab`, and `Shift-Tab`, and asserts the focused control after each event. Its controls use the same GPUI focus and accent-border APIs as the product buttons. Removing the Tab key bindings makes the test fail at the first focus assertion; restoring the bindings passes it.

Focused command:

```text
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/home/kedarmd/dev/bodh/target cargo test --locked --manifest-path spikes/native-sequence/Cargo.toml keyboard_focus_tests -- --nocapture
```

Result: 1 passed, 0 failed. `cargo fmt --manifest-path spikes/native-sequence/Cargo.toml --all --check` and `git diff --check` passed.

Full desktop validation was run after the final change:

```text
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/home/kedarmd/dev/bodh/target ./scripts/validate desktop
```

Result: `Validation passed: desktop`. The full runner output is preserved in [VALIDATE-16-KEYBOARD.log](VALIDATE-16-KEYBOARD.log). No GUI was launched: the live Bodh window was left untouched as requested. The new focus appearance and navigation still need visual confirmation in a fresh build, and this change does not complete the remaining #16 native acceptance criteria.
