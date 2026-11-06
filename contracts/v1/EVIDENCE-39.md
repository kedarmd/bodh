# Issue #39 — Changes workspace visual follow-up

## Implemented

- Changed-file rows now impose a 48 px minimum and center the two-line filename/directory group vertically. The selected row keeps its raised background and now has a persistent accent border.
- Base/Head source buttons use the persistent selected-side background in addition to the accent border, distinct from transient focus.
- An open draft editor changes the source toolbar action to `Draft comment · Base` or `Draft comment · Head` and applies a persistent selected treatment, making its source relationship visible.
- Unified and split diff virtual lists now request the full width of their containing pane. Split rows retain equal flexible side cells and a one-pixel visible divider.

## Validation

`CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/home/kedarmd/dev/bodh/target ./scripts/validate desktop` passed on the dirty issue worktree at base `ecca831e466567f51e66fe9e94e03c92fad5ef3f`, using Rust/Cargo 1.99.0. The complete runner output is in [VALIDATE-39-desktop.log](VALIDATE-39-desktop.log). The runner's application, native GPUI checks and tests passed. One first run failed because the separate isolated `/tmp` Cargo targets exhausted the 3.7 GiB tmpfs; after cleaning only those generated targets and using the existing repository Cargo target, a flaky GitHub CLI preflight test failed once and passed on focused rerun and the subsequent full desktop run.

## Native trial — 2026-11-06

The saved preparation is `kedarmd/kmdot #103`, pinned to base `01664ef42e8768d6add1c70afb5f996e51b19990` and head `85a89dc006d2c7e1bad23ef26a8e92aa96633cf0`. Product mode ran via `./scripts/validate gui` in Hyprland workspace 1. The final process is PID `943539`, at 947×505. `CARGO_BUILD_JOBS=2 ./scripts/validate desktop` passed; its full runner output is [VALIDATE-39-native-desktop-2026-11-06.log](VALIDATE-39-native-desktop-2026-11-06.log). The release GUI runner output is in [VALIDATE-39-gui.log](VALIDATE-39-gui.log).

### Feedback and correction

The first native check showed Base line 40 selected while the changed theme line was line 43; the user's feedback identified that mismatch. The initial source preview covers the hunk, but the Base/Head controls reused its first context line when switching sides. The source-side switch now maps that initial preview to the first changed line on the requested side, preserves explicitly selected context rows, and maps added/deleted counterparts. The minimum window height changed from 560 to 480 so Hyprland can hold the requested 947×505 viewport.

Four focused `source_switch_line_tests` pass. They cover initial-hunk-to-change mapping, context-line pairing, added/deleted counterpart mapping, and the exact line numbers in the theme diff. The full desktop runner passed after these changes.

The pre-fix result is preserved in [light-base-selected-947x560.png](evidence/issue-39-native-2026-11-06/light-base-selected-947x560.png). A pointer-coordinate trial also opened a blank inline-comment editor instead of the Base control; no text was entered or saved, and the clean rerun began after that process was closed.

Hosted validation for the accepted implementation commit `707ca10` passed both core and desktop jobs in [run 42206828872](https://github.com/kedarmd/bodh/actions/runs/42206828872).

### Fresh native results

- At 1904×1020, Base and Head both select line 43, matching the theme change. See [light-base-mapped-43-1904x1020.png](evidence/issue-39-native-2026-11-06/light-base-mapped-43-1904x1020.png) and [light-head-mapped-43-1904x1020.png](evidence/issue-39-native-2026-11-06/light-head-mapped-43-1904x1020.png).
- In Split mode, clicking the Base deletion and Head addition selects the matching line 43 on each side. [Base row](evidence/issue-39-native-2026-11-06/light-split-base-row-43-1904x1020.png), [Head row](evidence/issue-39-native-2026-11-06/light-split-head-row-43-1904x1020.png).
- At exactly 947×505, Light Unified, Light Split, and Dark Split were captured. Split columns retain their divider and aligned line numbers. The Base label updates to `config-install.sh:43`; at this height the source pane's visible excerpt ends at line 41, leaving the selected source row below the viewport. See [Light Unified](evidence/issue-39-native-2026-11-06/light-unified-base-947x505.png), [Light Split](evidence/issue-39-native-2026-11-06/light-split-base-947x505.png), and [Dark Split](evidence/issue-39-native-2026-11-06/dark-split-base-947x505.png).
- The requested size was reached by resizing the floating product window from its maximized state. The compositor showed an intermediate 1127×1020 size during the resize and confirmed stable geometry 947×505 in workspace 1.

No crash or process exit occurred. The source selection, themes, diff modes, and exact viewport have been checked. The selected source label is visible at 947×505, while its highlighted code row falls below the currently visible excerpt. On 2026-11-06, after reviewing the corrected native render, the user explicitly accepted the result with “looked good to me.” This acceptance includes the 947×505 excerpt boundary and is separate from the D2 baseline approval.

### Scope and qualification limit

This product-mode trial concentrated on the #39 Changes workflow. It did not inspect Home/repository navigation, Overview, Findings, or Finish Review in the same run, as requested by the general native-trial procedure in `TESTING.md`. Those surfaces have separate evidence in other ticket records, but this run does not establish a cross-surface product qualification. The user's acceptance applies to the corrected Changes result above.

### Follow-up observation during #16 native trial — 2026-11-06

While reviewing saved `kedarmd/kmdot #103` at a 947×1020 Bodh window, the user reported that the changed-file entries feel too close to the panel border, with too much gap between entries. The target-window capture is [issue-39-changes-spacing-feedback-2026-11-06.png](../../spikes/native-sequence/evidence/issue-39-changes-spacing-feedback-2026-11-06.png). Current source uses a 48 px row minimum, 8 px horizontal row inset (`px_2`), 4 px list gap (`gap_1`), 8 px filename/directory gap (`gap_2`), and 8 px outer panel padding (`p_2`). This records the concern for a visual follow-up; no spacing change has been made or accepted.
