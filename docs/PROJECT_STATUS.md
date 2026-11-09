# Bodh — Project Stage & Authorization Register

**Product:** Bodh V1 (entire application)  
**Updated:** 2026-11-08  
**Purpose:** Record what work is currently authorized. This is **not** a product requirements document or a replacement for the PRD/SPEC.

## Current authorization

| Field | Current value |
| --- | --- |
| Active stage | **Design — FigCraft** |
| Authorized scope | **Design Gate 0 (discovery) and Gate 1 (design system + geometry specimen board)** |
| Approval required before proceeding | **Yes — explicit user approval of Gate 1 before Gate 2** |
| Rust application implementation | **Not authorized in the current stage** |
| Rune feasibility prototype | **Not authorized in the current stage** |
| Current design approval record | [design/DESIGN_STATUS.md](design/DESIGN_STATUS.md) — no gates approved yet |
| PRD scope | **Entire V1 product lifecycle** |
| SPEC scope | **Entire V1 implementation, integration, testing and release** |

This records the **initial** user-approved Codex assignment. It does not restrict what Bodh V1 is intended to become. Read the active task and explicit user instructions before carrying out work.

## Stage progression

1. **Design:** Deliver editable FigCraft artifacts, tokens, screens and QA according to [design/FIGCRAFT_BRIEF.md](design/FIGCRAFT_BRIEF.md). Each design gate needs explicit user approval, recorded in [design/DESIGN_STATUS.md](design/DESIGN_STATUS.md).
2. **Framework feasibility:** After design approval **and separate explicit authorization**, test a pinned Rune revision against Bodh's actual Linux/Wayland UX requirements. If the prototype fails, evaluate alternatives without weakening approved requirements.
3. **Core/CLI implementation:** After separate authorization, implement shared Rust domain, Git/`gh`, SQLite, strict contracts and headless CLI without reliance on a running GUI.
4. **Manual review UI:** After authorization, build and verify the approved native interface and manual end-to-end review/comment/publish flow.
5. **AI-assisted integration:** Implement agent review import, diagrams, Findings, follow-up assessment and the `pr-review` skill integration.
6. **Hardening and release:** Test performance, accessibility, stale revisions, publication reconciliation, failure recovery and Linux packaging.

Stages 3–6 are indicative technical order, **not permission** to proceed automatically. Read the authoritative implementation sequence in [SPEC.md](SPEC.md).

## How to update this register

- **Only record a newly authorized stage/gate after explicit user approval or instruction.** Do not infer approval from a completed task, successful test, or a design export.
- Record: date, exact authorized scope, approval source (user message or issue/decision link), remaining restrictions, and any blockers. If no evidence is available, leave the stage unchanged.
- Transitioning out of design changes the active task in this file; **do not rewrite PRD.md or SPEC.md just to change the current work stage**.
- `AGENTS.md` is the permanent agent instruction entry point. It must read this file for current authorization instead of hard-coding a forever-design-only rule.
- Separate design approval from framework selection and implementation authorization. Approving a design does not by itself authorize implementing it.

## Change log

| Date | Stage/scope | Authorization evidence |
| --- | --- | --- |
| 2026-11-08 | Initial FigCraft design Gate 0–1 only; other work gated | User's original request to start the new Bodh FigCraft design process with gate approval |
