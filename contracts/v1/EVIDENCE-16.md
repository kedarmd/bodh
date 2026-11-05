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
