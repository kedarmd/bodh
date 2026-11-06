# Issue #16 native workspace evidence

## Implementation boundary

The native default mode opens a local-only Home, Repository/PR browser, and prepared Overview/Changes/Findings workspace. GitHub CLI listing and the shared `bodh` prepare/source services run outside the GPUI event thread. Stale results are discarded by operation generation; preparation operations use the shared cancellation/deadline control. Preferences store theme, navigation tab, pane width, repository paths and preparation IDs separately from review payloads. `Finish Review` remains visible in every workspace view and reports the honest empty-draft state.

The user-approved D2 baseline is recorded outside the repository at `/home/kedarmd/.local/state/bodh-implementation/design/APPROVAL-D2.md`. Its approved no-analysis behavior suppresses the diagram/code pair. No analysis or sequence diagram exists in the #16 prepared-review model, so the native Overview shows the manual/no-analysis state and the original PR description. The Changes view reads source only through the shared pinned-source service. Diagram-to-source click-through and the equal-height D2 pair remain for the later diagram workflow; they are not claimed as passed here.

## Automated verification

- `CARGO_BUILD_JOBS=2 ./scripts/validate desktop` passed on commit `27a6764` after integrating `e12dbf2` (#14/#15). The run exercised CLI/core, acquisition and discussion tests, GPUI check/Clippy/test targets. Full output is in [VALIDATE-16.log](VALIDATE-16.log).
- Added native crate tests for title/number/author/lifecycle matching and preference save/reload; both passed in the desktop run.
- `cargo fmt --manifest-path spikes/native-sequence/Cargo.toml --all --check` and `git diff --check` passed.

## Native launch and render

- `CARGO_BUILD_JOBS=2 ./scripts/validate gui` compiled the optimized release app and launched it in the graphical session. The app window remains open for inspection; successful runner exit is still pending until the window is closed after the user's trial.
- Runtime: Linux x86_64, Rust 1.99.0, GPUI 0.2.2, Hyprland/Wayland; monitor scale 1, 1920×1080. Native client PID 905174, Bodh class `bodh-review-workspace`, client geometry 947×1020 at compositor coordinates `(-955, 668)`.
- [product-home.png](../../spikes/native-sequence/evidence/product-home.png) is a capture of only the Bodh client window at its live geometry. It shows the repository empty state, checkout path entry, Open Repository action, and recent-review empty state. Visual inspection found readable text and a clear primary path at this viewport.
- No pointer or keyboard automation was run. The native window remains open for the user’s trial and screen acceptance. Home path entry, live GitHub listing, search/lifecycle, workspace resume, pinned source selection and finish entry have not been accepted through native interaction yet.

## Decision status and limits

The D2 visual baseline is approved. Native screen acceptance is separate and remains pending. The screenshot only validates Home; it does not validate the other states or any future analysis/diagram rendering. The no-analysis Overview intentionally does not render a fake diagram. This app has no publication, draft-editing or agent-launch behavior in this ticket.

## User interaction trial — 2026-11-06

The user interacted with the live product window for the #16 navigation and review checks. This was Bodh PID `943539` in Hyprland workspace 1 at 947×1020. The process was launched from a dirty tree based on revision `317ac8324b64b769d883a329460a203c4e1bda75` (`contracts/v1/VALIDATE-39-gui.log`); it is useful interaction evidence for the observed screens, but is not a fresh build of current HEAD `c3a5906287369a41db42a994e956e2fc6164316a`.

- Home and repository listing were shown. The user opened `/home/kedarmd/dev/kmDot`, searched for `102` (only PR #102 remained), and cycled lifecycle All → Open (empty) → Closed (#102 shown) → All; the user confirmed the filters matched.
- The user resumed the saved workspace for `kedarmd/kmdot #103`, “Fix Quickshell loader animations.” Overview displayed the original PR description and manual/no-analysis state. In Changes, the user selected a changed file and confirmed both diff and pinned source loaded. The separate spacing concern on changed-file entries is recorded in [EVIDENCE-39.md](EVIDENCE-39.md); no correction or acceptance is claimed.
- Findings showed the expected empty state, “No committed findings” and no complete generated result. The target window capture is [issue-16-findings-no-result-2026-11-06.png](../../spikes/native-sequence/evidence/issue-16-findings-no-result-2026-11-06.png).
- The user opened Finish Review and confirmed the preview was read-only. No inline comments were saved, and no review was sent or published. Back from that preview returned to Findings, as shown in the user-provided screenshot; Back from Findings then returned to the PR browser, confirmed by the user.
- On a single Tab press after clicking inside Bodh, the user reported no visible focus indicator or unexpected target. Keyboard focus remains unqualified; do not treat this as a pass. A focused keyboard visibility fix is being prepared separately, after which a fresh current-head native run is needed.

These observations establish interaction with the listed screens in the stated dirty runtime only. They do not establish current-HEAD qualification, visible keyboard focus, user acceptance of the complete #16 experience, or publication behavior.

## Fresh native trial after keyboard patch — 2026-11-06

- Integrated keyboard patch: `76dd506f34dce7e04ee668c17ae639822c540c80`; source tree matches the worker's desktop-validated tree. The desktop output and focus test are in [VALIDATE-16-KEYBOARD.log](VALIDATE-16-KEYBOARD.log) and [EVIDENCE-16-KEYBOARD.md](EVIDENCE-16-KEYBOARD.md).
- `./scripts/validate gui` rebuilt and launched the current source in release mode. Runner output: [VALIDATE-16-gui-2026-11-06.log](VALIDATE-16-gui-2026-11-06.log). Live Bodh PID `1755157`, Hyprland workspace 1, geometry 947×1020 at `(965, 52)`. No crash was observed at launch.
- The fresh Home window capture, limited to the Bodh client, is [issue-16-home-current-head-2026-11-06.png](../../spikes/native-sequence/evidence/issue-16-home-current-head-2026-11-06.png). It shows registered repositories and recent reviews at this viewport.
- At capture time, native visual confirmation of keyboard focus was pending. The later Wayland interaction and user confirmation are recorded below; the automated GPUI test alone is not treated as visual acceptance.
- After coordinating with the user and confirming the target PID/geometry, one `Tab` was sent to the fresh Bodh window using Wayland virtual-keyboard input. The selected Home control showed a clear accent focus border; another `Tab` moved the border to Light, and `Shift-Tab` returned it to Home. Captures: [Home after Tab](../../spikes/native-sequence/evidence/issue-16-focus-home-tab-2026-11-06.png), [Light after Tab](../../spikes/native-sequence/evidence/issue-16-focus-light-tab-2026-11-06.png), and [Home after Shift-Tab](../../spikes/native-sequence/evidence/issue-16-focus-home-shifttab-2026-11-06.png). No control was activated. This confirms native focus indication and traversal in the fresh build; the user’s confirmation is recorded below.
- The previous dirty GUI process was stopped deliberately before the rebuild so two Bodh instances would not overlap. Its original runner log, [VALIDATE-39-gui.log](VALIDATE-39-gui.log), ends with status 143 from that SIGTERM; this was an intentional replacement, not an unexplained crash. The current runner remains active for the native check and has not yet produced its final exit status.
- User confirmation after the fresh-build trial: “Yeah, the tab is changing the selectable things. Its working.” This accepts the keyboard traversal behavior observed in the current window. The Home/Light/Home captures above show the matching focus border.

## Expanded focus-control build trial — 2026-11-06

- `./scripts/validate gui` rebuilt the working tree after adding focus stops for link actions, changed-file rows and selectable diff rows. The runner records revision `76dd506f34dce7e04ee668c17ae639822c540c80` plus a dirty working tree in [VALIDATE-16-gui-controls-2026-11-06.log](VALIDATE-16-gui-controls-2026-11-06.log). PID `1971664` launched on workspace 1; Home, Repository search for `103`, and the saved #103 review were reached in the native build. GitHub metadata now reports the PR closed, and the Findings no-result state rendered.
- This run confirms the updated app launches and these navigation states remain reachable; it does not establish the visual focus treatment on the added link/file/diff controls. The process was deliberately stopped after the trial; its runner records status 143, not a crash. The prior Tab/Shift-Tab acceptance remains limited to the Home/Light controls and the user's general confirmation that selectable controls were traversed.
- No remote review/comment/publication action was performed. #16's keyboard criterion and overall native qualification remain open until the newly added row/link focus states and the other unchecked acceptance criteria are directly verified.

## Current-head repository/navigation trial — 2026-11-06

- A second `./scripts/validate gui` run built commit `18aeaca187e99d6deb4c253688faba0fa1e135b8`; see [VALIDATE-16-gui-qualification-2026-11-06.log](VALIDATE-16-gui-qualification-2026-11-06.log). The source was clean at that commit; the runner labels the checkout dirty only because the pre-existing untracked `contracts/v1/GUI-GH-PREFLIGHT.log` remains in the working tree.
- Current-build target-window captures show Home ([capture](../../spikes/native-sequence/evidence/issue-16-home-qualification-2026-11-06.png)), the repository browser ([capture](../../spikes/native-sequence/evidence/issue-16-repository-qualification-2026-11-06.png)), and search for `102` returning only PR #102 ([capture](../../spikes/native-sequence/evidence/issue-16-search-102-current-2026-11-06.png)). The saved workspace route opened; GitHub metadata reported that PR #102 is no longer open ([capture](../../spikes/native-sequence/evidence/issue-16-review-102-current-2026-11-06.png)). No review action was activated.
- The keyboard sequence activated the PR result before I could verify the Open empty state. The lifecycle check is therefore explicitly unverified in this run. The current-build run did not reach Changes or verify the newly added row/link focus states. The user-reviewed #103 source/diff and historical lifecycle observations remain useful, but do not substitute for those missing current-build checks.
- The process was deliberately stopped after the inspection; the runner ends with status 143. This is a planned close, not a crash. No remote write was made.

## Workspace focus handoff and selectable rows — 2026-11-06

- Review-entry actions now return keyboard focus to the product root before asynchronous navigation replaces the initiating Home/repository control. Repository keyboard Enter uses the same handoff. A GPUI test covers tab traversal resuming after the root receives focus; focused and full desktop results are in [VALIDATE-16-FOCUS-ENTRY-FINAL.log](VALIDATE-16-FOCUS-ENTRY-FINAL.log) and [VALIDATE-16-FOCUS-ENTRY-DESKTOP.log](VALIDATE-16-FOCUS-ENTRY-DESKTOP.log).
- In a fresh release build, I opened saved PR #103, sent Tab to focus Back, traversed to Changes, focused a changed-file row, and then focused a selectable unified diff row. Enter on that diff row loaded its pinned source at line 50. Captures: [workspace focus](../../spikes/native-sequence/evidence/issue-16-focus-review-root-2026-11-06.png), [changed-file focus](../../spikes/native-sequence/evidence/issue-16-focus-changed-file-row-2026-11-06.png), [diff-row focus](../../spikes/native-sequence/evidence/issue-16-focus-diff-row-2026-11-06.png). No comment or publication action was activated.
- Runner output is [VALIDATE-16-FOCUS-ROOT-GUI.log](VALIDATE-16-FOCUS-ROOT-GUI.log); it records the working tree and ends with status 143 because I intentionally stopped the app after inspection. The user-provided #103 workspace is closed upstream, and the workspace reported this. No live write was attempted.
- Link-style focus remains unverified because this prepared workspace contains no generated findings/actions. #16 remains open; this check does not cover its remaining preferences, resize, theme, grouped-review, and full navigation acceptance.
