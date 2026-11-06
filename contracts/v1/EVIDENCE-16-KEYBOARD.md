# Issue #16 keyboard focus evidence

Native buttons, link-style actions, changed-file rows and source-selectable unified/split diff rows now participate in GPUI tab traversal. `Tab` and `Shift-Tab` invoke GPUI's next/previous focus traversal across all Bodh screens, including Home, Repository, workspace tabs, and Finish Review. Focused buttons and rows use the approved D2 accent border; link-style actions underline while focused. Search and editor focus handles are tab stops as well, so keyboard traversal can reach text controls when they are rendered.

## Automated interaction check

`product::keyboard_focus_tests::tab_and_shift_tab_move_focus_between_visible_controls` mounts a GPUI test view representing a button action, link-style action, changed-file row and selectable diff row. It uses the same `keyboard_tab_stop` helper as the product controls, dispatches Tab through all four and Shift-Tab back, and asserts the focused control after each event. The controls use the same GPUI focus styling as the corresponding product controls. The original red run without Tab bindings fails at the first focus assertion; the expanded focused test passes with the bindings.

Focused command:

```text
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/home/kedarmd/dev/bodh/target cargo test --locked --manifest-path spikes/native-sequence/Cargo.toml keyboard_focus_tests -- --nocapture
```

Result after the follow-up: 1 passed, 0 failed. `cargo fmt --manifest-path spikes/native-sequence/Cargo.toml --all --check` and `git diff --check` passed.

Full desktop validation was run after the final change:

```text
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/home/kedarmd/dev/bodh/target ./scripts/validate desktop
```

Result: `Validation passed: desktop`. The earlier full runner output is preserved in [VALIDATE-16-KEYBOARD.log](VALIDATE-16-KEYBOARD.log); the expanded run after adding link/file/diff controls is [VALIDATE-16-FOCUS-CONTROLS.log](VALIDATE-16-FOCUS-CONTROLS.log). That run did not launch a GUI. A fresh native interaction is still required to visually confirm focus on link, changed-file and diff controls; the earlier Home/Light captures only verify the shared button focus treatment. This change does not complete the remaining #16 native acceptance criteria.

## Independent Spec review follow-up

The first Spec review found that link-style actions, changed-file rows and selectable diff rows were missing from the Tab order and had no visible focus treatment. The follow-up adds these controls through the shared tab-stop helper, preserves the existing selected file style, underlines focused links, and adds a focused accent border to selectable diff rows/cells. The updated GPUI test now traverses four representative control categories. The review finding is addressed in code; native visual confirmation on those row/link controls remains open.
