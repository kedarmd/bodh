# Bodh V1 — Documentation Index and Authority

**Updated:** 2026-11-08  
**Scope:** Full V1 product from discovery and design through architecture, implementation, integration, verification and release.

The active Codex assignment is tracked **only** in [PROJECT_STATUS.md](PROJECT_STATUS.md). A design task's restrictions do not reduce the scope of the permanent PRD or SPEC.

## Document map

| File | Purpose | Lifetime/authority |
| --- | --- | --- |
| [PROJECT_STATUS.md](PROJECT_STATUS.md) | Current stage, explicitly authorized tasks, next-gate restrictions | **Mutable workflow register**; never substitute for requirements |
| [PRD.md](PRD.md) | Product goals, complete feature scope, user flows, screen behaviors, accessibility and acceptance criteria | **Permanent product authority** for V1 |
| [SPEC.md](SPEC.md) | Rust architecture, CLI, Git/`gh`, SQLite, versioned AI contracts, publishing semantics, validations and implementation/testing plan | **Permanent technical authority** for V1, refined by approved changes |
| [design/FIGCRAFT_BRIEF.md](design/FIGCRAFT_BRIEF.md) | Initial FigCraft design assignment and approval-gated screen sequence | **Stage-specific instructions**; not an implementation ban after authorization |
| [design/DESIGN_QUALITY_CONTRACT.md](design/DESIGN_QUALITY_CONTRACT.md) | Design tokens, geometry, spacing, usability, review-flow and verification criteria | **Design acceptance authority** and implementation UI QA input |
| [design/DESIGN_REFERENCES.md](design/DESIGN_REFERENCES.md) | Public references for useful interactions, not templates | Nonbinding inspiration |
| [design/DESIGN_STATUS.md](design/DESIGN_STATUS.md) | Design-gate approvals and QA evidence | Design approval record; does not authorize source implementation |

Root [AGENTS.md](../AGENTS.md) defines durable agent procedures; root [README.md](../README.md) introduces the project.

## Precedence and conflict resolution

1. Explicit, current user decisions and approvals govern what may happen **now**.
2. **PRD:** authoritative product scope, behavior and acceptance. **SPEC:** authoritative technical invariants and versioned contracts, subject to product requirements.
3. Approved FigCraft artifacts define **visual layout, components and interaction-state details** for approved designs, without rewriting product or protocol rules.
4. Design Quality Contract provides measurable UX and geometry acceptance criteria; the FigCraft Brief is the design-work task order.
5. External screenshots and reference images are inspiration only.

When these conflict, **record the discrepancy and seek a product/contract decision**; never silently reinterpret the PRD, weaken safe publication, or substitute framework defaults for approved UX.

## Rebuild and delivery boundaries

- The earlier GPUI implementation is archived; do not copy its UI, styling or architecture into the clean-slate implementation.
- **Rust is intended**, but Rune + rune-kit is a **conditional framework candidate** pending a separately authorized Linux/Wayland proof of concept.
- `bodh` is the native app and headless CLI; `pr-review` is the existing external coding-agent skill.
- Design work uses editable FigCraft source and approved previews; keep durable artifacts under repository root `design/bodh/` where actual FigCraft export facilities permit.
- All code development, framework validation, and GitHub remote actions obey the authorization in [PROJECT_STATUS.md](PROJECT_STATUS.md) and the technical safeguards in [SPEC.md](SPEC.md).

## Initial Codex assignment

**Only while [PROJECT_STATUS.md](PROJECT_STATUS.md) lists design Gates 0–1 as active:** Read `AGENTS.md`, this index, the PRD, SPEC and FigCraft design documents. Carry out Gate 0 and Gate 1 in FigCraft, inspect real renders and geometry, present QA and stop for explicit approval. Do not implement application code or start a Rune probe without separate authorization.

After the project stage changes, replace this initial assignment with the new authorized task; **do not continue treating design-only instructions as a permanent restriction**.
