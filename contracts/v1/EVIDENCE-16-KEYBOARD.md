# Issue #16 keyboard focus evidence

Native buttons, link-style actions, changed-file rows and source-selectable unified/split diff rows now participate in GPUI tab traversal. `Tab` and `Shift-Tab` invoke GPUI's next/previous focus traversal across all Bodh screens, including Home, Repository, workspace tabs, and Finish Review. Focused buttons and rows use the approved D2 accent border; link-style actions underline while focused. Search and editor focus handles are tab stops as well, so keyboard traversal can reach text controls when they are rendered. Review-entry actions now return focus to the workspace root before replacing the repository-browser or Home controls.

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

Result: `Validation passed: desktop`. The earlier full runner output is preserved in [VALIDATE-16-KEYBOARD.log](VALIDATE-16-KEYBOARD.log); the expanded run after adding link/file/diff controls is [VALIDATE-16-FOCUS-CONTROLS.log](VALIDATE-16-FOCUS-CONTROLS.log). The workspace-root handoff follow-up also passed the focused GPUI test in [VALIDATE-16-FOCUS-ENTRY-FINAL.log](VALIDATE-16-FOCUS-ENTRY-FINAL.log) and full desktop validation in [VALIDATE-16-FOCUS-ENTRY-DESKTOP.log](VALIDATE-16-FOCUS-ENTRY-DESKTOP.log). Fresh native evidence now confirms the changed-file and unified-diff focus indicators; link-style focus still needs a rendered finding/action to verify. This does not complete the remaining #16 native acceptance criteria.

## Independent Spec review follow-up

The first Spec review found that link-style actions, changed-file rows and selectable diff rows were missing from the Tab order and had no visible focus treatment. The follow-up adds these controls through the shared tab-stop helper, preserves the existing selected file style, underlines focused links, and adds a focused accent border to selectable diff rows/cells. The updated GPUI test now traverses four representative control categories. Fresh native evidence confirms the changed-file and diff-row treatment. Link-style focus remains open because the prepared #103 review has no generated findings to render those actions.

## Workspace-root focus handoff — 2026-11-06

The Home recent-review, repository Open Review button, and repository keyboard-Enter paths now focus the product root before the asynchronous transition replaces the initiating control. The GPUI test checks traversal can resume after focus returns to the root. A fresh release GUI build then opened saved PR #103, showed a visible Tab focus ring on Back, traversed the Changes tab, a changed-file row and a unified-diff row, and loaded pinned source after Enter on the diff row. Target-only captures: [workspace Back focus](../../spikes/native-sequence/evidence/issue-16-focus-review-root-2026-11-06.png), [changed-file focus](../../spikes/native-sequence/evidence/issue-16-focus-changed-file-row-2026-11-06.png), and [diff-row focus](../../spikes/native-sequence/evidence/issue-16-focus-diff-row-2026-11-06.png). No publication/comment action was activated. The exact GUI runner output is [VALIDATE-16-FOCUS-ROOT-GUI.log](VALIDATE-16-FOCUS-ROOT-GUI.log); it ends with status 143 because the GUI was intentionally stopped after inspection.
