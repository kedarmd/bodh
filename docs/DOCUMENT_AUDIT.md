# Bodh V1 — Documentation Scope and Consistency Audit

**Date:** 2026-11-08  
**Scope:** Every Markdown document shipped in this documentation package, including root agent instructions, lifecycle-wide PRD/SPEC, FigCraft brief, design QA, references and approval/status registers.

## Results

- **PRD.md:** Confirmed it defines complete V1 feature/product scope through release. Repointed stage/approval authority to the dedicated status records. GPUI is no longer an implementation requirement.
- **SPEC.md:** Confirmed it covers core Rust architecture, data, CLI, Git/`gh`, schema, native UI, publication and testing. Removed dependency on a fixed design-stage instruction; Rune remains an unapproved candidate pending a feasibility prototype.
- **AGENTS.md:** Replaced a hard-coded forever-design-only rule with durable repository rules plus a mandatory check of the actual authorized project stage before acting.
- **Root README.md:** Reworded as a full application introduction with explicit separation between permanent scope and current initial design assignment.
- **docs/README.md:** Rebuilt the document map and precedence, distinguishing permanent product/technical authority from mutable task instructions and design-only references.
- **PROJECT_STATUS.md:** Added one authoritative register for stage/permission. Initial work is FigCraft Gate 0–1, and implementation or framework-prototype work requires separate explicit authorization.
- **FIGCRAFT_BRIEF.md:** Clearly scoped to design work only, not the project's lifetime. Added current-stage reference and no-app-code language limited to the design task.
- **DESIGN_QUALITY_CONTRACT.md:** Clarified that design QA is required during FigCraft work **and** remains a UI implementation/regression acceptance input after design approval.
- **DESIGN_REFERENCES.md:** Fixed stale link to a removed `CODEX_FIGCRAFT_DESIGN_PROMPT.md`; canonical prompt is `FIGCRAFT_BRIEF.md`. Clarified nonbinding references.
- **DESIGN_STATUS.md:** Explicitly limited to design approval records, not framework or implementation permissions.

## Cross-document invariants checked

1. **Product name:** Bodh (बोध); application/CLI `bodh`; external skill `pr-review`.
2. **Framework:** Rust intent; Rune + rune-kit candidate pending a Linux/Wayland proof of concept; **no required GPUI dependency**.
3. **UI:** Findings left master list + right code and suggestions; Changes GitHub-style inline discussions and local staging; Finish Review contextual drawer; source-linked Overview diagrams.
4. **Design:** FigCraft editable source, geometry/spacing/alignment QA, explicit user approvals, dark/light and viewport checks.
5. **Review safety:** Manual review without AI, strict versioned contracts, pinned revision context, single current workspace, safe local draft handling and confirmed GitHub publication.
6. **Stage separation:** PRD/SPEC remain applicable to implementation/release; `FIGCRAFT_BRIEF` governs design tasks; `PROJECT_STATUS` gates when work may proceed.

**Outstanding by design:** Actual FigCraft approvals, Rune feasibility outcome, precise CLI/schema limits, supported Linux environment matrix and framework-dependent implementation details. None should be misreported as decided.

**Maintenance rule:** Do not modify the PRD/SPEC merely because the current stage changes. Update them only when product or technical decisions change, using contract-first governance. Change `PROJECT_STATUS.md` to record task authorization and `DESIGN_STATUS.md` to record design approvals.
