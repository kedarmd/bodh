# Ticket #30: sequence diagram navigation evidence

## Implemented

- Adapts validated `ReviewResult` diagrams to the native renderer without synthesizing flows. Message, return, self-call, note, alternate branch and optional-step semantics and order are preserved.
- Stable semantic interaction IDs connect diagram hits to their exact `SourceRef`, explanation and linked findings.
- Selecting an interaction loads pinned source context beside the diagram. The selected source range is highlighted; the inspector can open the file in Changes or navigate to a linked finding.
- Per-flow selection and scroll state are retained. Multiple flows have a selector; a single flow does not. Keyboard arrows move through interactions and reveal the selection.
- Direct change emphasis is derived from exact changed-line intersections, with color plus a heavier stroke. The source diff is loaded asynchronously for changed files referenced by diagrams.
- Participant and interaction label geometry accounts for longer text; existing virtualization/culling supports large diagrams.

## Automated validation

`CARGO_BUILD_JOBS=1 ./scripts/validate desktop` passed on the dirty `ticket/30-sequence-navigation` worktree. The preserved output is `VALIDATE-30.log`. This includes formatting, headless typecheck, Clippy and the full test suites. Sequence navigation tests: 6 passed. Adapter tests: 3 passed (including Product preflight coverage). A transient process-spawn failure during an earlier full run passed when isolated and the final full run passed.

`git diff --check` passed after implementation. Native release compilation passed through `./scripts/validate gui`.

## Native trial

The GUI runner's first launch was intentionally stopped before interaction because the isolated recent-review fixture had an invalid serialized revision value; this is recorded in `GUI-30.log`. The value was corrected and the runner was relaunched successfully. The app is open with an isolated temporary state database containing a validated and committed PR #103 result for `config-install.sh`. No user input has been synthesized. Direct interaction and visual acceptance are pending user review.

The seeded result was checked with production `bodh validate --prepared` and committed only to the temporary database. It has one flow with request/return, self-call, note, `alt` and `opt` steps, each source-linked to the pinned changed line 43. The user's normal Bodh state was not modified.

## Review checklist

- Open PR #103 from Recent reviews and confirm the diagram flow and participant/step order.
- Select the request, return, self-call, each branch step, and the optional step. Confirm the pinned source inspector follows each selection and highlights line 43.
- Use arrow keys to move selection and verify the source excerpt updates.
- Confirm the directly changed source cue is distinct and corresponds to line 43.
- Check whether diagram and source panes remain usable at the current window size, and report any clipping or incorrect source association.

This evidence records automated results separately from the pending direct user interaction.
