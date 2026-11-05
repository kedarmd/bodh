# Changes screen visual follow-up

User feedback from the native PR review trial on 2026-11-05. The user confirmed repository/PR opening, Overview description rendering, changed-file rendering, diff rendering, and Base/Head/Draft comment controls. These screenshots record three remaining visual issues:

1. Changed-file labels sit high within their rows instead of aligning vertically with the row.
2. The selected Base/Head source revision and the open Draft comment editor do not have a clear active state.
3. Split diff columns leave most of the available diff pane blank and have no visible separator.

## Planned acceptance

- Vertically center changed-file labels in each file row while preserving the selected row treatment, truncation, click target, and scrolling.
- Give the selected source revision a clear persistent state, distinct from transient keyboard focus. When the draft editor is open, make its relationship to the source controls clear.
- Make split diff use the full width available to the diff pane, with equal base/head columns and a visible divider. Keep line numbers aligned and maintain source-selection behavior on both sides.
- Verify at the captured 947×505 window size, during resize, in light/dark themes, with unified and split modes, and with keyboard focus/selection. Capture a fresh native render and request user acceptance separately from the D2 design-baseline approval.
- Keep the approved D2 visual baseline; treat these as screen-local layout/state corrections unless the user later asks to change canonical tokens or components.

## User screenshots

- [Changed file row alignment](evidence/changes-screen-feedback-2026-11-05/changed-files-alignment.png)
- [Pinned source active state](evidence/changes-screen-feedback-2026-11-05/pinned-source-active-state.png)
- [Split diff width and divider](evidence/changes-screen-feedback-2026-11-05/split-diff-width-and-divider.png)
