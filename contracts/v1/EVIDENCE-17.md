# Issue #17 implementation evidence

Issue #17 adds original PR description reading, changed-file navigation, pinned diffs, and source inspection to the native review workspace.

## Behavior

- Overview presents PR metadata, a compact description excerpt, and the full Markdown description in a separate scrollable area. Markdown is parsed into native text blocks; raw HTML remains inert and links are displayed without becoming actions.
- Changes provides a searchable, virtualized changed-file list and bounded unified or split diffs. Changed and context lines can open the corresponding immutable base/head source range in a resizable inspector. Source reads remain pinned to the prepared commit graph and do not consult or mutate the checkout.
- Navigation stores the selected file, source side, diff mode, and independent scroll positions in recent-review preferences. Older preferences load with defaults.
- Binary, unsupported and oversized files have explicit bounded/error states. Per-selection epochs prevent stale asynchronous file/source loads from replacing a newer selection.
- The no-analysis state remains intact. Sequence diagram/code synchronization is deferred to #30.

## Verification

`CARGO_BUILD_JOBS=2 ./scripts/validate desktop` passed after formatting and lint fixes. The runner covers the core and native GPUI packages, Clippy, unit/integration suites and locked native test targets. Raw output is preserved in [VALIDATE-17.log](VALIDATE-17.log).

Focused coverage includes three real-Git pinned diff tests for literal path scoping, changed and unchanged source reads, complete-line truncation, rejection of paths outside the prepared change, and checkout/index preservation. Native manual review tests cover unified/split line pairing, Markdown safety/readability, and syntax-token spans; preference tests cover old and new navigation state.

No interactive native GUI trial was performed for this issue. The accepted D2 visual baseline is recorded in [APPROVAL-D2.md](/home/kedarmd/.local/state/bodh-implementation/design/APPROVAL-D2.md); the #16 home screen acceptance does not verify #17’s interactive Changes workflow.

## Native review follow-up — 2026-11-05

The first interactive trial exposed that selecting a diff row displayed only its single anchored source line, which made the surrounding code hard to verify. The pinned-source inspector now keeps the comment anchor on the exact selected line while displaying up to four nearby lines on each side; the selected anchor is highlighted. The user confirmed that the displayed Base/Head context makes the changed line visible. Long code lines wrap within the pane; its toolbar stays fixed while the source content scrolls.

The user also reviewed the narrow-window pane behavior and confirmed that the diff controls stay within their pane and all panes are reachable by horizontal scrolling. Final desktop validation after these native UI changes passed; see [VALIDATE-26.log](VALIDATE-26.log). The native GUI build and review record is in [GUI-26.log](GUI-26.log).

## Current-head keyboard/source trial — 2026-11-06

- On the release build at `d89a11f83c99dd3601f0086b92fa574a3459b236`, the existing saved `kedarmd/kmdot #103` workspace was open on the CachyOS/Hyprland Wayland desktop at 947×1020. The PR is now closed upstream; the app showed that state and no remote write was attempted.
- Tab moved focus to a unified diff context row; Enter selected it. The source inspector showed Head `config/quickshell/components/BarModule.qml:51` with surrounding pinned lines 50–55. This verifies keyboard activation and an unchanged context line mapping to its pinned source. Capture: [issue-17-keyboard-context-source-2026-11-06.png](../../spikes/native-sequence/evidence/issue-17-keyboard-context-source-2026-11-06.png).
- Tab traversal then focused the added-file row `LoadingSpinner.qml`; Enter selected it. The unified diff and Head source for the new file loaded, and the unavailable Base side was not offered. Capture: [issue-17-keyboard-added-file-2026-11-06.png](../../spikes/native-sequence/evidence/issue-17-keyboard-added-file-2026-11-06.png).
- Local comment draft count remained zero. No draft, replacement, PR action, or checkout/index mutation was made. The workspace was left open for user inspection.
- This run adds focused current-head evidence for keyboard selection, unchanged context source, and an added file. It does **not** verify selected-file/tab/scroll/pane restoration across restart, a partially visible row, long-description scrolling, PR/context switching, cancellation of an in-flight load, or unsupported-file behavior; the two remaining #17 acceptance criteria therefore remain open. Existing automated and prior native evidence retain their narrower scopes.
- GUI build/launch output is in [VALIDATE-16-ACCEPTANCE-GUI.log](VALIDATE-16-ACCEPTANCE-GUI.log); the runner remains attached to the open app and has not yet emitted an exit status.

### Follow-up: keyboard activation and PR-context resume — 2026-11-06

- After confirming Bodh was the focused Hyprland window, keyboard activation worked: Tab focus plus Enter opened **Back** to the repository list; Tab focus plus Enter on **Open review** opened the saved PR #102 workspace; Home returned to the recent-review list; and keyboard activation of the #103 recent review reopened that workspace. The PR #102 detail used for this context switch is captured in [issue-17-pr102-overview-context-switch-2026-11-06.png](../../spikes/native-sequence/evidence/issue-17-pr102-overview-context-switch-2026-11-06.png).
- Returning to #103 restored the selected `LoadingSpinner.qml` file, unified diff and Head source. Capture: [issue-17-restore-file-after-pr-switch-2026-11-06.png](../../spikes/native-sequence/evidence/issue-17-restore-file-after-pr-switch-2026-11-06.png). The active tab is a global navigation preference: switching to Overview in #102 left #103 on Overview when reopened. Per-review scroll and pane restoration, and restore after process restart, remain unverified.
- PR #102's multiline Markdown description and text code block rendered in Overview at the tested 947×1020 layout; the compact author excerpt showed truncation. This is a realistic multi-line description check, not a maximum-length stress case or a scroll-through test.
- `cargo test --locked --manifest-path spikes/native-sequence/Cargo.toml home_button_activates_after_tab_focus -- --nocapture` passed, followed by `CARGO_BUILD_JOBS=2 ./scripts/validate desktop`; exact outputs are [VALIDATE-17-KEYBOARD-ACTIVATION.log](VALIDATE-17-KEYBOARD-ACTIVATION.log) and [VALIDATE-17-KEYBOARD-ACTIVATION-DESKTOP.log](VALIDATE-17-KEYBOARD-ACTIVATION-DESKTOP.log). The regression test constructs the actual `Product` and shared top-bar Home button, tabs to it, sends the key-up event GPUI uses for keyboard clicks, and asserts the screen changes to Home. This supplements, rather than replaces, the native interaction above.
- The apparent Enter/Space failures from the earlier trial are excluded: Bodh was not the active window, so those virtual keys went to another Hyprland workspace. No changes were made to PR drafts or GitHub, and the workspace was left open on #103 Changes with the restored added file selected.
- Remaining #17 native gaps: partial-row activation, maximum-length/scrolling descriptions, per-review tab semantics if required, scroll/pane restoration, cancelled loads, and unsupported content. Both #17 issue checkboxes remain open.

### Read-only merged-review state — 2026-11-06

A later read-only check confirmed that `kedarmd/kmdot#103` was merged on 2026-11-05. Bodh's open Changes workspace shows “PR is no longer open on GitHub” while retaining the previously selected `CastLauncherView.qml` diff and pinned Head source. This is a stale, now-closed review context; it was not treated as a new open-review trial, and no local draft or remote action was created. Capture: [issue-17-merged-review-context-2026-11-06.png](../../spikes/native-sequence/evidence/issue-17-merged-review-context-2026-11-06.png). The previously recorded restore-after-switch result remains scoped to the earlier session when the PR was open.

### Follow-up: repository-list resume route — 2026-11-06

- A native trial found that reopening a PR from the repository list labeled the row “Saved workspace” but prepared a new review instead of resuming that workspace. The resume path also replaced a saved Base source selection with the default Head side.
- Repository-list mouse and keyboard activation now resume the newest saved workspace for that repository/PR, falling back to preparation only when none exists. Resume restores the saved source side when that side is available for the selected file and persists the restored selection and scroll state after selection initialization.
- Added a preference lookup unit test and a GPUI integration regression test using a temporary Git repository and controlled `gh` fixture. The test verifies the original preparation ID, file selection, Base side, split diff, Changes tab, and absence of duplicate recent entries.
- The focused integration test passed, and `CARGO_BUILD_JOBS=2 ./scripts/validate desktop` passed on the local working tree. Raw full-run output: [VALIDATE-17-RESUME-ROUTE.log](VALIDATE-17-RESUME-ROUTE.log). The subsequent isolated-state native GUI replay has not yet been run; the existing user state was not changed by this fix.
- This addresses saved-workspace routing and source-side restoration. Per-review scroll/pane restoration across restart, long-description stress, cancelled loads, and unsupported content remain unverified; #17 and #37 remain open.
