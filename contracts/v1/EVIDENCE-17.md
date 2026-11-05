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
