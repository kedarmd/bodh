# Codex task — design Bodh V1 in FigCraft (stage-specific, approval-gated)

You are acting as a **senior product designer and design-system engineer** for **Bodh (बोध)**, a Linux-native GitHub pull-request review application. Your job is to produce **original, editable designs and interactive prototypes in FigCraft**, not to write the app, refactor Rust, or recreate an existing screenshot.

## Authority and current stage

This document governs **FigCraft design tasks**, not the lifetime scope of Bodh or the technical SPEC. For current authorization and transition to later development phases, consult [`../PROJECT_STATUS.md`](../PROJECT_STATUS.md). Do not treat design-only rules below as a permanent ban on implementation once a later stage is explicitly authorized.

- Read root `AGENTS.md` first, then `docs/PROJECT_STATUS.md`, `docs/README.md`, `docs/PRD.md`, `docs/SPEC.md`, this brief, the quality contract, and the references.
- **This assigned task is design only while authorized by PROJECT_STATUS.md.** Do not edit application code or begin a Rune proof of concept during this design task. The archived implementation is not visual design material.
- **No design has been approved yet.** Begin Gate 0 discovery and Gate 1 design system; never infer approval from completed output.
- Treat the reviewer’s minimal interruption-free workflow as the top priority. The left-findings/right-code-and-suggestions layout, inline code-anchored discussions, and Finish Review drawer are non-negotiable UX constraints.

## First actions

1. Read `docs/PRD.md` and `docs/SPEC.md` for current product behavior and strict contracts. They have been revised for a clean rebuild. Rune is under evaluation, not yet formally approved. Do **not** use framework constraints to invent or limit the visual design.
2. Read `docs/design/DESIGN_QUALITY_CONTRACT.md` **in full** and treat it as a mandatory design acceptance checklist. Do not build or present any screen without passing its geometry, component consistency and flow-state gates. Read `docs/design/DESIGN_REFERENCES.md` supplied with this prompt. Inspect the linked images/sources where available. They are **pattern references** only: no pixel-for-pixel reproduction, no copying branded visual assets, no mimicking entire page layouts.
3. Discover the **actual installed FigCraft integration** in this environment. Consult the tool's current docs and available MCP/CLI commands before using it. The upstream FigCraft project documents `figcraft-cli mcp`, its command registry, editable files, render/export tooling, auto-layout, components, variants, and design variables; do not assume a particular executable path or unsupported API. If FigCraft cannot be reached, report the exact blocker and stop instead of generating HTML, static SVG, screenshots, or code as a substitute.
4. Inspect available working design documents. **Do not import or reuse the previous Bodh screens, styles, or layout as inspiration**. You may read the product requirements, interactions, or backend contracts, but consider the old UI a failed exploratory attempt.
5. Establish a **design-first, approval-gated workflow**. Do not implement application code; do not create all screens in one unattended batch. Work in small, reviewable deliverables, render each, inspect its geometry, and present it to the user for approval. Never claim a design is approved until the user says so.

## Product and interaction principles (non-negotiable)

- Bodh is **a code-review workspace, not an IDE, chat client, enterprise dashboard, or CRUD form**. Code is the primary visual element; surrounding UI must be quiet and purposeful. **The reviewer must not lose flow state** while inspecting, commenting, staging or navigating between evidence and findings.
- The app works **fully for manual PR review without AI**. An external coding agent invokes the `pr-review` skill and produces structured reviews; Bodh itself does not start agents in V1.
- Reviewer feedback is staged locally before explicit publication. Do not imply inline comments, thread replies, replacements, or thread state changes are posted immediately.
- **GitHub-style inline code review** is the interaction baseline for Changes: anchored line/range comments, rich Markdown drafts, code replacement suggestions, existing threads, replies, resolution actions, and unified/split diffs.
- **Findings is a master-detail workspace**: narrow resizable left pane listing title, severity, file path and review state; large right pane with exact source code, explanatory evidence and AI comment/replacement alternatives **directly below the code**. One suggested alternative is staged per finding by default; independent manual comments are allowed.
- **Finish Review is a right-side drawer**, not a dedicated full-page form. Show a compact, scannable publication plan, explicit Comment / Approve / Request Changes selection, editable summary, validation warnings, and final confirmation.
- Overview emphasizes the behavioral story: brief author-description excerpt with **View full description** in a reading pane; compact implementation summary; focused interactive sequence diagram with adjacent code inspector; meaningful intent mismatches only.
- One current mutable workspace per PR, not a timeline of snapshots. If the PR head has advanced, show an unobtrusive stale-revision warning, retain pinned code, and never suggest that old findings automatically reflect new commits.
- **Zero generated AI results is a normal state**, not an error: manual review and diff commenting must still function.

## Visual direction

Create an **original, cohesive visual identity** for Bodh: dark-first and equally intentional in light mode, sophisticated without being decorative. Favor high-quality type, precise spacing, consistent density, clear focus states, subtle separators, restrained borders, excellent code legibility, and a small semantic palette. Avoid giant rectangular cards, large blank forms, tiny code typography, excessive badges, colored panels, gratuitous gradients, glassmorphism, and flashy animations.

Alignment and spacing are **non-negotiable acceptance criteria**, not subjective polish. Use shared auto-layout, grid anchors, consistent spacing and padding tokens, and reusable components. Button labels must be genuinely centered in their hit targets, including icon+text combinations; repeated lists/tabs/toolbars must share consistent baselines and edge anchors. **Never eyeball an approval without checking the actual exported geometry.** Follow `docs/design/DESIGN_QUALITY_CONTRACT.md` for explicit checks and blocker rules.

Use no more visual color than required for important semantics. Standard diff addition/deletion tints are permitted; findings severity must remain scannable without turning the interface into a rainbow. Sequence diagrams are neutral/monochromatic with **one semantic accent for code interactions directly changed by the PR**; selection has a separate visual state. Provide meaningful non-color cues.

Treat dimensions as starting hypotheses, to be verified in FigCraft rather than blindly fixed:
- Primary desktop reference at **1440 × 900**; also test **1280 × 800** and wide **1920 × 1080**.
- Comfortable default code font around 13–14 px equivalent, independently scalable; UI body around 13–15 px equivalent. Use consistent typography roles.
- Resizable file/finding left rail approximately 240–320 px at common desktop widths; main code region gets the available space.
- Spacing MUST use a named token scale, with reused auto-layout/constraints/component variants. No manual per-instance offsets or hand-positioned sibling rows. Check parent padding, sibling gaps, baseline alignment and overflow using FigCraft geometry tools when available; inspect exported renders regardless.
- Dark/light design tokens should be semantic. Do not blindly copy Tokyo Night, OpenCode, GitHub, or Linear palette; propose a fresh palette grounded in legibility and adjust after review.
- Interactions should respect reduced motion and keyboard focus; motion only where it improves orientation.

## Screen inventory — design all of these, one approved stage at a time

### S01 — Home / review workspace
- Minimal top bar, app identity, registered repositories, recent review workspaces, Open Repository entry point.
- Repo display name/owner prominent; local path secondary. Clearly indicate pending drafts and stale context when relevant.
- Deduplicate recent reviews by canonical PR identity. No large diagnostic status block; put `gh` status unobtrusively in a status/diagnostic affordance.
- States: populated, first-run empty, loading, missing checkout/`gh` error, long repo list.

### S02 — Repository pull-request browser
- Search by PR title, number or author; lifecycle filter; distinguish open/closed/merged PRs, saved review indicator, and changes requiring reviewer attention.
- Dense but readable row layout, not giant tiles and repeated 'Open review' buttons on every full-width card unless justified.
- States: many PRs, no matches, no open PRs, remote error, previously saved review.

### S03 — PR Overview
- Compact PR identity and context, status and revision freshness, tabs `Overview | Changes | Findings`, persistent Finish Review action.
- Original author description: concise excerpt plus **View full description**; full Markdown opens a resizable scrollable reading pane (no duplicative full text on main page).
- One focused sequence diagram at a time; compact selector if multiple. Clicking a message opens/updates the adjacent pinned-source inspector, including line numbers, readable syntax, linked findings and View in Changes.
- Monochromatic diagram; accented changed messages ONLY, neutral unchanged; single-level `alt`/`opt`, no nesting.
- Concise generated implementation summary and key changes, evidence-linked intent mismatches only; omit empty blocks.
- States: generated analysis present, manual-only (no AI), diagram selected with inspector open, full description open, stale PR revision, multiple diagrams.

### S04 — Changes (highest-priority interaction screen)
- Changed-file navigator, file search/filter, readable syntax-highlighted diff, line numbers, unified/split control, scroll and file-navigation affordances.
- Hover gutter +; single-/multi-line selection; inline composer appears **below the anchored code range**, with Markdown editing, preview, add to review and cancel; optional code-replacement editor with before/after preview.
- Existing GitHub threads appear directly beneath anchored lines, support read/reply/resolution draft; distinguish published, local draft, outdated and resolved states.
- AI findings appear as subtle indicators tied to relevant lines; navigation to the finding is non-disruptive.
- Avoid a permanently visible right-hand 'Pinned source' comment form; extra inspector only if needed for cross-file context.
- States: diff at rest, single-line composer, range selection, replacement suggestion, open published thread, staged local draft, split mode, huge/long file, binary/renamed file and empty diff.

### S05 — Findings (highest-priority layout screen)
- **Left:** compact list of actionable findings ordered/filterable by severity; each shows severity, title, source file/path and pending/staged/dismissed indicator. Search/filter and counts remain small.
- **Right:** chosen finding heading, reason/impact/evidence; primary pinned code excerpt with enough surrounding lines and actual source location; supporting references with easy switching; one or more AI alternative comments, and code replacement where supplied, immediately BELOW the code; editable composer and stage/dismiss/defer controls.
- Existing related GitHub thread link and View in Changes action. One selected suggestion per finding by default; switching alternatives must preserve unsaved edits until confirmed.
- Resolved follow-up findings summarized in a collapsed area; review coverage/limitations compact and collapsed near bottom.
- States: finding selected, alternative changed, editable comment, staged, dismissed, linked GitHub thread, no findings, no AI review, resolved summary.

### S06 — Finish Review drawer
- Workspace stays visible behind an overlay/right drawer. Show count + short previews of staged comments, code replacements, thread replies and pending resolution/reopen actions (omit empty groups).
- Every staged item links to its code location and can be edited/unstaged. Present review event selection `Comment / Approve / Request changes`, optional/required summary as applicable, clear publish confirmation.
- Stale/invalid code-anchor warning with actionable paths to reattach/edit/discard; partial publication state and retry/reconcile behavior should be representable.
- States: empty drafts, 3–5 staged entries, request-changes selected, warning blocking publication, confirmed publishing, partial failure.

### S07 — Shared contextual interaction states and overlays
- Full PR description reader; sequence-step source inspector; inline draft editor; existing discussion with replies; replacement code suggestion preview; confirmation before destructive refresh; stale revision indicator; load/error/no AI states.
- These are **component/state designs**, not more page-level tabs.

## Design system deliverable (must come before page screens)

Create an editable FigCraft design-system section/file with:
- Type roles and code typography; spacing, radii, border, divider, elevation, focus, surface, overlay and semantic color tokens; both light and dark modes.
- Reusable components: app shell, tabs, icon button, toolbar action, status pill, search field, filter control, repository/PR row, file tree item, severity indicator, finding row, diff toolbar, code line, inline comment/thread, suggestion before-after block, editor, empty state, drawer, confirmation modal, skeleton state, diagram participant/message/alt frame.
- Component variants: default, hover, focus, selected, disabled, pending, staged, error where relevant.
- A clearly defined minimum readable size and responsive behavior. Include the anti-clutter rule: no giant bordered containers around every line of content.
- A **geometry specimen board** showing centered button labels, icon-only controls, icon+text buttons, toolbar, tabs, aligned finding/file rows, long labels, hover/focus variants, with explicit spacing/padding token usage. Must be verified at 100% zoom/export.
- Design rationale: explain the hierarchy choices, why the accent is used, and what behavior each component communicates.

## Gate sequence (required order)

0. Discovery and reference/requirements audit.
1. Design system, reusable tokens, component variants and **geometry specimen board**; request approval and **stop**.
2. **Findings** populated master-detail layout and suggestion/composer states; approval and stop.
3. **Changes** populated diff + exact-line inline comments/replies/replacements; approval and stop.
4. **Finish Review** drawer, including partial publication state; approval and stop.
5. **Overview** descriptions, interactive diagrams, source inspector; approval and stop.
6. **Home + Repository browser**; approval and stop.
7. Cross-screen UX, geometry, accessibility, flow preservation audit; final approval and stop.

Use the process instructions below only insofar as they are consistent with this order. The later screen inventory describes requirements, **not** permission to build all screens immediately.

## Process and sign-off gates

**Gate 0 — Audit and plan:** Summarize requirements found, describe proposed information architecture, enumerate screens and interaction states, verify FigCraft tooling, and propose visual direction. Do not modify application code.

**Gate 1 — Design system:** Build and render the dark design tokens/component sheet **and geometry specimen board** in FigCraft; include a small light-mode counterpart. Inspect exports for center alignment, equal repeated gaps/padding, baseline consistency, overflow, contrast, focus and legibility; record findings in `QA_REPORT.md`. Present the file, exports and quality report for user approval. **Stop.**

**Gate 2 — Findings:** Once Gate 1 is approved, build populated Findings with real-looking evidence/code and suggested feedback; render selected, edited and staged states; demonstrate left/right pane geometry and navigation. Present exports and wait for approval.

**Gate 3 — Changes:** After approval, build populated Changes plus the complete inline comment → stage → review interaction, in unified and split views. Render and show interaction states; wait for approval.

**Gate 4 — Finish Review:** After approval, create the populated drawer and error states, review decision/summary, links back to staged code; wait for approval.

**Gate 5 — Overview:** After approval, create diagram + source-inspector interaction, author-description preview/full reader, summary/mismatches and manual-only state; wait for approval.

**Gate 6 — Home + PR browser:** After approval, design navigation, search/filter and recent-review/repository flows; wait for approval.

**Gate 7 — Cohesion pass:** Only after individual screens are approved, verify all screens at 1440×900, 1280×800, 1920×1080; test dark/light, focus, scrolling, pane resize, truncation, empty states and motion rules. Export a final handoff board and summarize outstanding UX questions.

Do not skip gates, start a later gate without explicit approval, or send a 'final design' with unapproved assumptions. During each gate, use actual FigCraft files/frames/components and exported renders; **no screenshot-only or HTML-only substitutes**. Keep the design editable.

## Visual quality verification (every gate)

**Full acceptance criteria are in `docs/design/DESIGN_QUALITY_CONTRACT.md` and MUST be applied to every gate and every required interaction state. A screen with clearly misaligned controls or broken review continuity is not complete, even if it renders without errors.**

1. Render the design using FigCraft and inspect the real exported image, not merely the layer tree.
2. Validate each major alignment, spacing and geometry; check text clipping/truncation, scroller bounds, focus contrast and file/path overflow.
3. Ensure the main content uses available width; sidebars should be resizable and never crowd code.
4. Verify all design-token/component definitions are reused instead of independently hardcoded per screen. Audit exact repeated gaps, component dimensions, center alignment of button labels/icons, left/right edges, baseline consistency and code gutter alignment. Use numeric FigCraft bounds if supported, otherwise mark checks visual-only. Never fabricate measurements.
5. Fix visible issues before asking for user approval. Identify any uncertainties honestly.
6. Provide a short changelog of what was designed, file paths, exported PNG paths, a geometry/flow QA report listing open failures, and exactly which approval is requested. Do not seek approval when known blocker-level geometry issues remain.

## Handoff requirements

Keep FigCraft source files and exports in a clear design workspace such as `design/bodh/` (or user-specified location). Suggested organization: `design-system.figcraft`, `screens.figcraft`, `exports/`, `DESIGN_NOTES.md`. Adapt to actual FigCraft capabilities, and don't invent file extensions if this installation uses something different.

Include a screen/state inventory, semantic token mapping, component variants, interactions, keyboard/focus behaviors, minimum/ideal sizing, and developer handoff notes. No app runtime code and no speculative 'Rune implementation' until designs are approved and a separate implementation task is authorized.

### Start now

In Gate 0, state the design-system token choices and the planned **geometry QA + flow-state validation**. Read and obey `docs/design/DESIGN_QUALITY_CONTRACT.md`. Complete **Gate 0** first. Then begin **Gate 1 (design system)** only. Use FigCraft directly and return the real design file and rendered screenshots for approval. Do **not** proceed to Findings or any other screen until I approve the design system.
