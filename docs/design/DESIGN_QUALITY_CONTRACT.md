# Bodh — Design Quality & Flow-State Contract (mandatory)

**Status:** V1 design acceptance criteria. **Applies to:** every FigCraft component, screen, interaction state, and exported frame. This document supplements `FIGCRAFT_BRIEF.md` and takes precedence when a vague aesthetic preference conflicts with an explicit quality rule.

## Goal

Produce a **quiet, modern, code-first review workspace** where a reviewer can understand a pull request, inspect a finding, draft a comment, stage it, and continue reviewing **without losing their place**. Precise alignment, consistent spacing, good legibility, and predictable behavior are release gates, not optional polish.

A pleasant screenshot is **not proof** that the design works. The deliverable must remain editable in FigCraft and every required state must be checked at realistic desktop dimensions.

## A. Layout system — no arbitrary placement

1. **Define a scale before drawing screens.** Start with semantic spacing tokens such as `space.1=4`, `space.2=8`, `space.3=12`, `space.4=16`, `space.5=20`, `space.6=24`, `space.8=32` (logical design pixels). Adjust the scale only as a deliberate **design-system** change. Screens may choose different *tokens* for different contexts; repeated instances of the same component must use the same token.
2. **Use FigCraft auto-layout / stacks, constraints, component instances and shared variables** wherever supported. Do not hand-place successive items with independent X/Y coordinates or fake gaps using transparent spacer rectangles. Absolute placement is reserved for justified overlays and diagram geometry; even those must anchor to their owning element/viewport.
3. **Use a shared grid and alignment anchors.** Define consistent screen inset, header alignment, sidebar content inset, text-column start, code gutter, and panel divider placement. Repeated sibling rows align their text/icon columns. A badge must not shift title alignment between rows.
4. **Padding is inside a component; gap is between siblings; margin is outside a component.** Define all three intentionally, and never compensate for wrong parent geometry by tweaking each child individually. Use predictable inset and gap tokens.
5. **Resizable panes use constraints**, not a static screenshot arrangement. Define a minimum width for the findings/file pane, a minimum readable main-code width, and what collapses/truncates at narrow sizes. The right side should get remaining width, not leave unused blank space.
6. **One source of truth per reusable control.** Do not draw a new button, tab, search field, finding row, severity marker, dialog, or comment composer from scratch for another screen. Use component variants and shared typography/colors/spacing.
7. **No cosmetic corrections by magic offsets.** If something is off-center, fix its parent layout, alignment or typography metrics, then update the component instance everywhere. No arbitrary `+3 px` nudges without documented typographic/optical justification.

## B. Component geometry — measurable gates

The agent must inspect exported renders **and**, if the actual FigCraft integration exposes bounds/constraints, inspect those numeric properties too. Do not fabricate measurements or report capabilities the tool does not expose.

| Check | Pass condition |
| --- | --- |
| Button labels | Text is centered both horizontally and vertically in the actual clickable button; icon+label group is centered as **one unit** when applicable; baseline looks optically balanced |
| Icon-only controls | Icon visually centered inside a consistent hit target; hover/focus outline not clipped |
| Repeated list rows | Equal row heights within each density variant, identical padding, consistent title/meta baselines and right-edge actions |
| Form/search controls | Input text, placeholder, leading icon, trailing action, label, and border align within their component variants |
| Tabs and toolbar | Tab labels share a baseline; active indicator aligns to tab bounds; toolbar controls share vertical center |
| Left/right panes | Shared top edge and header baseline; divider has consistent thickness; pane widths meet minimum constraints |
| Code views | Line-number gutter width consistent; code text never overlaps gutter or comments; indentation, highlight and anchors align |
| Inline comment | Composer begins directly beneath the selected anchored line range and has the same horizontal origin as the code content region, accounting for the gutter |
| Cards/panels where necessary | Equal internal insets; avoid nested borders or a full-width rectangle for every small fact |
| Text | No collisions, accidental clipping, baseline drift, stranded icons, or misleading truncation; long file names and PR titles handled deliberately |
| States | Hover, focus, disabled, selected, staged and validation-error variants do not cause neighboring controls to jump or resize unexpectedly |

**Numeric geometry tolerance:** when the tool exposes bounds, target alignment of common anchors and repeated component sizes to within **1 logical pixel**; repeated intentional gaps should match their token values to within **1 logical pixel**. If exact geometry isn't exposed, use rendered side-by-side comparison and explicitly report that the test was visual only. Never declare a numeric pass from visual estimation alone.

**Minimum control sizing:** compact controls should usually have a usable target of **at least 32×32 logical px**, with normal controls closer to **36–40 px**. If a denser code-gutter affordance is required, provide an adjacent or expanded hover/focus target and verify keyboard accessibility. Treat these as provisional design guidelines, not a substitute for actual testing.

## C. Typography, density and polish

- Define text roles (`app-title`, `section-title`, `body`, `metadata`, `code`, `code-line-number`, `button`, `badge`) with a consistent font family, weight, line height and baseline treatment.
- Keep code legible: start around **13–14 px**, and UI body around **13–15 px** at 100% scale; don't solve density problems by reducing everything to tiny text. Controls must still look balanced when labels become two words longer.
- Use simple surfaces, subtle separators and deliberate negative space. Reduce nested cards, ornamental gradients, excessive badges, oversized empty states, and full-width forms.
- Use a restricted color vocabulary. Diff additions/removals and findings severity retain clear semantics; most of the app remains visually quiet. Sequence diagrams remain monochromatic except for the **single semantic accent** used for directly changed interactions.
- Provide visible keyboard focus and non-color status cues. Dark and light modes are separate verified designs, not merely inverted colors.
- No unnecessary animation. Preserve reduced-motion support; transitions should orient the reviewer, not decorate the task.

## D. Flow state — UX acceptance rules

The default review path is **open PR → inspect diff or finding → inspect context → draft/edit → stage locally → continue at the same spot → Finish Review when ready**.

1. **Keep code in view.** Code is the main workspace, not a secondary preview. The Findings detail pane contains the relevant code and suggestions directly below it; Changes contains comments inside the diff, under their anchored lines.
2. **No route change for routine work.** Drafting a line comment, switching a suggested comment, expanding a discussion, editing a draft, and staging it must not take the reviewer to a new page. Use inline expansions, small contextual overlays or the existing pane.
3. **Preserve context.** After staging/canceling, retain selected PR, tab, file, line range, scroll offsets, finding selection, pane widths, and expanded threads where reasonable. Never unexpectedly jump to the top or another tab.
4. **One clear primary action.** Each interaction has an obvious next step (e.g. **Add to review**) and a safe escape (Cancel). Avoid several equal-weight CTAs competing with code.
5. **No accidental publishing.** Staging is explicitly local. Finish Review is a right drawer with a publication preview, distinct decision, source validation and user confirmation. No misleading language that suggests a staged comment is already on GitHub.
6. **Avoid interruption.** Don't open a blocking confirmation modal for reversible everyday actions. Confirm destructive draft loss, irreversible publication, and other genuinely consequential actions. Use subtle non-blocking status feedback for normal operations.
7. **Review without AI.** Missing generated analysis shows a compact honest explanation, not a dead-end application. Manual diff review and comments remain accessible.
8. **Keyboard-aware.** Design a sensible keyboard/focus path between findings list, code, suggestions, and stage action. Focus does not disappear when an inline composer opens/closes; Esc should cancel/dismiss context where appropriate without losing unsaved text silently.
9. **Information proximity.** Severity/title/file in the findings list; actual evidence and code at the point of decision; suggestions directly after evidence; discussion at its code anchor. Do not force memory-based cross-screen comparison.
10. **Progress without pressure.** Actionable findings are prominent, resolved follow-ups and coverage collapsed by default; no artificial need to clear every finding to finish.

### Mandatory end-to-end flow-state rehearsal

In FigCraft, build or simulate a **populated** PR review with at least 3 findings, 5 changed files, and an existing GitHub thread. Demonstrate the following continuous journey:

1. Open a repository PR and enter Changes.
2. Navigate to a changed file and select a 2–3 line range.
3. Open the inline composer at that range; write and stage a comment.
4. Continue reviewing **at the same file and scroll position**.
5. Open Findings; select a High severity finding and inspect its exact code.
6. Switch between AI comment and code-replacement alternatives; edit and stage one; preserve edits or explicitly warn before overwriting them.
7. Jump to the related line in Changes, then return to the selected finding without losing context.
8. Open Finish Review drawer; review staged items and jump to/edit one, then return to the drawer.
9. Close the drawer without publishing; resume reviewing with state intact.

Each transition must be shown with interactive FigCraft states/prototypes if supported, or explicitly documented frame-to-frame. **Do not invent working interactions that FigCraft cannot actually prototype.** Failure at any step means the workflow is not yet approved.

## E. Verification matrix and stop rules

Run an explicit visual+geometry QA pass **before seeking approval at each gate**:

| Required condition | Checks |
| --- | --- |
| Viewport sizes | 1280×800, 1440×900, 1920×1080 |
| Themes | Dark and light; no clipped focus indicators and sufficient readable contrast |
| Content stress | Very long PR title, long nested file path, 20+ findings, long Markdown comment, multiline code replacement, empty/no-AI state |
| Interaction states | Default, hover, focus, selected, editing, staged, disabled, error, stale revision where applicable |
| Geometry | Parent constraints, nested padding, repeated gaps, row alignment, button label/icon centering, truncation and scrolling |
| Flow | Review journey above; state restoration; no unnecessary modal/page hop |

When designing Gate 1 components, demonstrate a **geometry specimen** in FigCraft: rows of identical buttons with text-only, icon-only and icon+text variants, tabs, form fields, severity + finding rows, a compact toolbar, and long-label stress variants. Export the specimen at 100% size for inspection. Avoid only showing pretty overview boards where small alignment errors are hard to spot.

### Required reporting for each gate

Produce `design/bodh/qa/QA_REPORT.md` (or equivalent) with:

- FigCraft file/frame/component names and screenshot exports used for checks;
- token scale and reused components; viewport/theme/state combinations exercised;
- a **geometry audit table**: element/group, expected geometry, actual measured geometry **or “visual-only”**, result, fix;
- any missing or unverified checks marked **OPEN**, not PASS;
- a brief flow-state checklist and exactly where user approval is requested.

**Blockers that require fixing before approval request:** visibly off-center labels/icons, inconsistent recurring padding/gaps, misaligned shared rows, text overflow into another element, overlaid controls, hidden focus rings, code unreadability, uncontrolled pane overflow, wrong comment-anchor placement, or flow-breaking routine navigation.

If FigCraft lacks numeric geometry inspection or interaction prototyping, state that limitation honestly, compensate with screenshot comparisons and documented transitions, and ask for approval only for what has actually been verified. **Never mark a gate approved on your own.**

## F. Design/implementation boundaries

This document primarily governs **FigCraft design artifacts and their QA**, but the approved geometry, readability, accessibility, and flow-state acceptance criteria **continue to apply during UI implementation and regression testing**. During a design-only assignment, do not generate Rust/Rune application code, run migrations, or publish GitHub comments. A separately authorized later implementation task is permitted and must satisfy the approved criteria; see [PROJECT_STATUS.md](../PROJECT_STATUS.md). Preserve the behavior defined by the Bodh PRD/SPEC while treating archived visuals as superseded.

**Priority order when trade-offs arise:** reviewer flow continuity → correctness and code readability → geometry/alignment consistency → information clarity → tasteful visual polish → decorative motion.
