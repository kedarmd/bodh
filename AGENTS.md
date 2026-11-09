# Bodh — Repository Instructions for Coding Agents

## Product and durable scope

Bodh (बोध) is a **complete V1 native Linux GitHub pull-request review application**, with manual and externally agent-assisted review. The PRD and SPEC describe the **entire product lifecycle**, not merely a design exercise. The archived GPUI implementation is not a code or visual starting point for the rebuild.

**Before any work**, read [docs/PROJECT_STATUS.md](docs/PROJECT_STATUS.md) to learn which stage and tasks the user has actually authorized. Do not assume the project is perpetually in the design stage.

## Read in order

1. `docs/PROJECT_STATUS.md` — active work authorization and stage.
2. `docs/README.md` — documentation map, ownership and precedence.
3. `docs/PRD.md` — full V1 product requirements and acceptance criteria.
4. `docs/SPEC.md` — full technical architecture, contracts, safety and implementation expectations.
5. Stage-specific instructions **only when relevant to the authorized task**:
   - `docs/design/FIGCRAFT_BRIEF.md` — FigCraft task/gates.
   - `docs/design/DESIGN_QUALITY_CONTRACT.md` — design geometry and flow-quality rules.
   - `docs/design/DESIGN_REFERENCES.md` — interaction inspiration only.
   - `docs/design/DESIGN_STATUS.md` — explicit design approvals.

## Always-on engineering and product rules

- Preserve **manual PR review without AI**, strict versioned agent contracts, local editable/staged feedback, explicit GitHub publication, and safe pinned-revision handling.
- Keep responsibility boundaries: coding agents do semantic analysis, Bodh's Rust core/CLI own authoritative PR state and validation, and the reviewer owns GitHub decisions.
- Do not modify the working tree of a PR being reviewed; use `git` for read-only repository data and authenticated `gh` for GitHub integration.
- Preserve uninterrupted reviewer flow: GitHub-style inline comments in Changes, Findings left-list/right-source-and-suggestions, and Finish Review as a contextual drawer.
- Preserve validated designs and high-quality UI geometry: reusable tokens and components, precise alignment, readable code, real visual checks and accurate test reporting.
- Never reuse the archived app's visual language or assume the old GPUI code is a reusable foundation.
- Rust is the intended implementation language. **Rune + rune-kit is a candidate, not a frozen dependency**, until the authorized Linux/Wayland feasibility gate passes.
- Do not invent approvals, completed tooling actions, test results, numerical measurements or successful publication.

## Stage-specific rules

- **During a FigCraft design task:** Create editable FigCraft artifacts, run geometry/visual/flow QA, present each gate, and stop for explicit approval. **Do not write application code or conduct the Rune feasibility prototype during an unauthorized design-only task.**
- **During an authorized framework prototype:** Validate Rune against the approved UX with a pinned revision and real Linux input/diff/diagram/idle tests. Do not silently turn a failed prototype into a product dependency.
- **During authorized implementation:** Follow the approved FigCraft handoff, PRD, SPEC, and contract-first tests. Do not make new UX/product decisions through arbitrary framework defaults; surface discrepancies for explicit resolution.

Approval to proceed with one gate is **not** permission for the next gate. Only update `docs/PROJECT_STATUS.md` or `docs/design/DESIGN_STATUS.md` after explicit user instruction or approval. If two authorities conflict, explain the conflict rather than silently choosing.
