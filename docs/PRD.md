# Bodh — Product Requirements Document (V1)

**Status:** V1 product requirements baseline — applies to design, implementation, testing, and release; individual UI designs await approval  
**Updated:** 2026-11-08  
**Product name:** **Bodh (बोध)** — finalized  
**CLI and executable:** `bodh`  
**Coding-agent skill:** `pr-review` (unchanged)  
**Target:** Experienced developers using Linux, GitHub and locally installed coding agents  
**Companion document:** [`SPEC.md`](./SPEC.md)

> **Product thesis:** A fast, local-first, native PR review workspace that stands on its own for manual reviews and accepts structured, evidence-based reviews from the coding agent the developer already uses. AI helps understand the change; the human controls feedback and GitHub actions.

> **Document scope:** This PRD defines the **complete Bodh V1 product**, from user workflows and UI behavior through review safety, acceptance testing, and release. It is **not limited to the design phase**. The previous implementation is archived and must not dictate the new visual design or code structure. Current delivery-stage authorization is tracked separately in [PROJECT_STATUS.md](PROJECT_STATUS.md), while design approvals are tracked in [design/DESIGN_STATUS.md](design/DESIGN_STATUS.md). Rune is the preferred Rust UI candidate, **pending a practical Linux proof of concept**, rather than a committed dependency.

## 1. Problem and opportunity

Reviewing a PR requires switching between the author's description, changed files, surrounding code, existing discussions, and potential problems. Large or agent-written PR descriptions increase reading overhead. AI review tools can add another interface, another model subscription, or an opaque review process. The existing [`pr-review` skill](https://github.com/kedarmd/skills/tree/main/pr-review) combines analysis with report generation and reviewer-action plumbing, making the skill responsible for too much unrelated work.

Bodh V1 separates responsibilities: coding agents analyze; the native application and headless `bodh` CLI own PR context, review data, rendering, reviewer interactions, and GitHub publication. The user may review manually without any AI.

## 2. Objectives

1. Make it faster to **understand the behavioral flow** of a PR through compact, clickable, source-linked sequence diagrams.
2. Offer a conventional, reliable **read-only code diff review** experience with native performance and clear keyboard navigation.
3. Let users **triage AI findings** and select/edit suggested feedback without blindly accepting it.
4. Make every comment, reply, suggestion, and thread action **local until explicitly published** through a consolidated Finish Review workflow.
5. Keep AI usage **agent-initiated**, independent of the GUI and of any particular vendor or model.
6. Preserve review consistency when a PR receives new commits, without storing a full history of all generated reports.
7. Reuse installed `git` and `gh`, avoid implementing GitHub API/auth or Git operations from scratch.
8. Never require a resident daemon or background agent for a completed review.

### Not goals

V1 is **not** an IDE, code editor, coding-agent management platform, autonomous GitHub reviewer, generic Git client, project-management dashboard, historical PR auditing system, or replacement for GitHub itself.

## 3. Scope and prerequisites

| Area | V1 decision |
|---|---|
| OS | Linux only; Wayland is a primary target; exact distribution/desktop support to validate |
| App | Native Rust; **Rune + rune-kit is the preferred UI candidate**, conditional on a Linux/Wayland proof of concept and approved designs; no desktop framework is final until then |
| Syntax highlighting | Accurate, themeable source highlighting is required; evaluate Rune-kit built-ins vs Tree-sitter adapter with real PR fixtures; LSP is not required |
| GitHub | GitHub pull requests only; existing local repository required |
| Dependencies | Installed/authenticated `gh`; installed `git` |
| Data | SQLite plus bounded local content/cache files |
| AI | External coding agents (Codex, Claude Code, OpenCode, compatible agents) invoke skill and headless CLI |
| AI launched from GUI | **Not V1** |
| GitHub mutations | Explicit human-initiated actions through app + `gh` |
| Review persistence | One mutable current review workspace per repository/PR identity |
| Code editing/branch checkout | Not supported; application must not alter working tree |
| Commit-range and unpushed-change reviews | **Not V1** |

**Privacy boundary:** No separate application-operated AI service or upload of code to the app's own backend. Agents may send code to whichever model provider the user has configured; the app must not claim that all code stays entirely on-device.

## 4. Core concepts

- **Repository:** A registered local Git checkout with a resolved GitHub remote and canonical owner/repository identity.
- **PR workspace:** The single persistent review workspace for a given repository and PR, reusable by manual and agent-assisted review.
- **Reviewed revision:** The pinned comparison context (`merge_base_sha`, `head_sha`) associated with displayed Overview, Changes and Findings.
- **Current AI result:** The latest committed, schema-valid generated overview, diagrams, findings and coverage; replaceable as a unit.
- **Local draft:** Unpublished reviewer-authored comments, selected suggestions, replies, review summary, and pending thread-state actions.
- **Remote status:** Latest observed GitHub PR head, lifecycle, and thread states; may be newer than the pinned review.
- **Follow-up review:** An agent-initiated new analysis of the same PR that replaces the current generated review on successful commit.

## 5. Key user journeys

### 5.1 Open a PR for manual review

1. Launch app; select a recent review, registered local repository, or open an existing local repository.
2. Browse GitHub PRs (search/filter by title, number, author; open PRs prioritized).
3. Select a PR. App obtains metadata and a consistent PR diff from Git/`gh` without checking out a branch.
4. Use Overview (PR description and metadata), Changes (diffs), existing GitHub discussions, and Finish Review. No AI result is required.
5. Draft comments/replies/suggestions and publish only after explicit confirmation.

### 5.2 Agent-initiated initial review

1. From a terminal/agent, invoke the `pr-review` skill for a PR number/URL in a local repository.
2. Skill runs `bodh review prepare`. CLI identifies the PR, obtains authoritative context and returns a lightweight manifest.
3. Agent explores relevant changed and surrounding source code, evaluates findings and flow, and creates a versioned structured result.
4. CLI validates; invalid results return actionable machine-readable errors for bounded correction attempts.
5. On success, CLI commits the entire result atomically; GUI may be opened on demand, but is not required.
6. Agent reports completion and an optional app-open command; its work ends without leaving a resident process.

### 5.3 Inspect overview flows

1. Open Overview. See PR metadata and short excerpt of the **original** PR description, with **View full description**.
2. See one focused sequence diagram at a time; show a flow selector only when more than one exists.
3. Click a diagram message/operation; adjacent resizable right-side inspector shows code **from the pinned revision**, line numbers, syntax highlighting, explanation, and linked findings.
4. Click another message to update the **same pane**, without navigation. Switch flows while preserving context when possible.
5. Distinguish directly changed interactions via one semantic accent color, with a subtle non-color cue; unchanged supporting steps remain neutral.
6. See concise generated implementation summary, key changes, and significant intent discrepancies. Full PR description is available in a side pane without replacing Overview.
7. If no meaningful diagram exists, omit it rather than rendering a fabricated flow.

### 5.4 Triage AI findings

1. Open **Findings** as a **resizable master-detail workspace**: compact **left pane** containing finding **title, severity, relevant file(s), and concise state**; large **right pane** dedicated to the selected issue.
2. Selecting a finding reveals its code **immediately on the right** with syntax highlighting, line numbers, clear changed-line anchoring, and enough surrounding pinned-revision context to understand it. Its concise explanation, scenario/impact, evidence, and related GitHub discussions remain accessible in this detail pane.
3. Show the agent's proposed review comments and GitHub-compatible code replacements **directly below the source excerpt**, not in a detached composer or separate full-page form. Support multiple alternatives but **one selected/staged suggestion per finding by default**.
4. The reviewer can edit and stage the selected suggestion, dismiss/defer the finding, or navigate to the complete diff. Switching suggestions must not silently overwrite edited or staged feedback.
5. The app preserves selected finding, detail scroll, draft state, pane widths, and navigation context when switching to Changes or back. It must not require a new route to evaluate a finding.
6. Resolved previous findings are compact and expandable, not mixed into the main actionable queue. Coverage and limitations appear in a compact expandable section at the bottom of Findings.
7. The Findings tab is optional for manual-only reviews and must degrade gracefully when no generated analysis exists; reviewers can still use Changes and Finish Review independently.

### 5.5 Work with GitHub discussions

1. **Changes is a GitHub-style, code-first review experience:** select a file and line/range; a composer opens **inline directly below the anchored diff lines**, with text/replacement modes as appropriate.
2. Read existing GitHub threads and replies **inline at their code anchors**; distinguish local drafts from published replies, and resolved/outdated status from genuinely fixed code.
3. Draft and edit replies and optional resolve/reopen actions locally. A finding may link a related existing thread, avoiding duplicate AI feedback.
4. **Stage** locally without publishing; the inline composer closes/compacts without resetting the selected file, scroll offset, highlighted line, or surrounding context. Do not use a permanently visible detached comment form.
5. Publish through Finish Review, allowing multiple GitHub requests when necessary and preserving recovery state on partial outcomes.

### 5.6 Finish review / publish feedback

1. Open **Finish Review** from any workspace tab in a **right-side drawer**, leaving the underlying code workspace visible and retaining its state. It is **not a full-screen page or long administration form**.
2. See a compact publication preview: locally staged inline comments/replacements, replies, thread actions, draft counts, and an editable review summary; inspect/edit each item contextually.
3. Choose exactly one review event: **Comment / Request changes / Approve**. Merge and Close are separate PR lifecycle actions with separate confirmations.
4. Before publication, validate the latest remote head, comment line/range anchors, replacement compatibility, and thread state. Flag stale/ambiguous anchors with clear actions to inspect, edit, reattach, or discard.
5. Explicitly confirm publication. The application tracks successful/failed/unknown GitHub operations, avoids blind retries, and retains unfinished work after partial failures.
6. Closing the drawer without publishing must restore the previous tab/file/finding, scroll, and selection without dropping drafts.

### 5.7 New commits arrive while reviewing

1. While the app is active, lightweight remote checks detect a new PR head; no background monitoring when closed.
2. Show a nonintrusive **review is older than remote PR** indicator.
3. Do **not** silently replace current diffs, diagrams, findings, code inspector, or drafts.
4. The reviewer may continue inspecting the pinned review; publishing always performs fresh validation.
5. For a fresh **manual-only** review, the user explicitly loads the latest PR code, replacing current displayed manual context while invalidating/hiding older AI claims. Unpublished drafts require a clear destructive confirmation if replacement would discard them.
6. For a fresh **AI** review, the user re-invokes the skill in their coding agent; app does not launch AI.

### 5.8 Explicit follow-up AI review

1. `review prepare` identifies an existing PR review and any unpublished drafts, including new comments, replies, summary, and pending resolution actions.
2. If drafts exist, skill warns that **successful replacement will erase them** and obtains explicit confirmation before proceeding; unattended invocation must stop rather than assume consent.
3. Existing review remains intact throughout analysis and validation. If analysis fails, no drafts are discarded.
4. Agent uses prior findings and existing GitHub threads to assess new/unresolved/resolved/regressed issues and avoid duplicates.
5. On successful final commit, result and the confirmed drafts are replaced/cleared **atomically**. If drafts change after confirmation, commit must reject and require renewed permission.
6. Only one current generated review is retained; no full historical snapshot browser.
7. GUI observes committed data and updates safely, without displaying partially generated results.

## 6. Information architecture and screen layout

### 6.1 Home

- A focused, scannable landing screen with **deduplicated recent PR review workspaces** and **registered local repositories**.
- Display human-readable repository identity prominently, with local path secondary; expose pending drafts and stale revision status only when useful.
- One clear **Open Repository** path; repository registration also occurs when the headless CLI is used in a new checkout.
- No activity dashboard, charts, duplicated review tiles, large diagnostics panels, or decorative card grid.

### 6.2 Repository PR browser

- Compact, searchable/filterable PR rows with title, number, author, lifecycle, and existing local review state; distinguish open/closed/merged.
- Enter or resume a manual workspace immediately, with no AI prerequisite; support clear loading/empty/unauthenticated/offline states.
- No remote auto-cloning in V1.

### 6.3 PR workspace: Overview | Changes | Findings

**Shared:** compact persistent PR context in header; tabs `Overview | Changes | Findings`; **Finish Review** entry point opens a right drawer; separate PR lifecycle action menu; nonintrusive revision freshness indicator. Restore per-tab file/finding/diagram selection, scroll, pane widths, and in-progress drafts.

**Overview:** short, deterministic **original** PR description excerpt with **View full description** in a contextual reading pane; one interactive focused sequence diagram at a time with adjacent resizable committed-source inspector; concise implementation summary and curated key changes; **significant** intent discrepancies/uncertainties only. No permanently side-by-side full descriptions.

**Changes (primary manual-review workspace):** changed-file navigator plus a large, readable **unified/split Git-style diff**. Hover/select valid lines or ranges to create a review composer **inline under the selected lines**; display published discussions and staged drafts at source anchors. Support replies, resolve/reopen intent, AI finding markers, syntax highlighting and source-context expansion. **No detached, always-visible 'Pinned source / Draft comment' control panel.** Code occupies most of the workspace.

**Findings (primary AI triage workspace):** **left pane** is a dense, resizable list displaying severity, finding title, and affected file(s). **Right pane** displays the selected finding's exact committed source, supporting explanation/evidence, and **AI comment/replacement alternatives below the code**, with contextual edit/stage/dismiss actions. Avoid cards and form pages for each finding. Actionable issues by default; resolved follow-ups and coverage collapsed.

### 6.4 Finish Review drawer

- Open from any workspace tab without navigating away; display compact staged-feedback summary, editable items and summary, review decision, validation warnings and final confirmation.
- The underlying review remains visible; closing/reopening restores context. Real GitHub publication errors or partially successful operations remain recoverable.

### 6.5 Visual design and interaction authority

- FigCraft artifacts approved at each gate are the **visual and interaction authority** for the rebuild, within this PRD and the strict semantic contracts in SPEC.md.
- Apply [design quality and flow-state rules](design/DESIGN_QUALITY_CONTRACT.md): shared tokens, auto-layout/constraints, reusable components, baseline/button-centering checks, real geometry/visual QA, and realistic populated states.
- Design order: system/specimen board → Findings → Changes → Finish Review → Overview → Home & repository browser → cross-screen audit. Gate approval is required before continuing.
- Verify desktop sizes **1280×800, 1440×900, 1920×1080**, dark and light themes, long paths/text, keyboard focus, and compact window behavior.
- Delivery sequence: approve the FigCraft designs, complete the independent Rune/Linux feasibility gate, then implement and verify the application against the approved product and interaction requirements. This PRD continues to apply throughout all stages.

## 7. Overview and diagram rules

- Agent produces **zero or more** focused diagrams; multiple only for genuinely distinct execution flows.
- Each shows main behavioral interactions and only unchanged context necessary to explain relevant changed behavior.
- Standard sequence semantics: participants/lifelines; call/message; return; self-call; single-level `alt`/`opt`; short notes only.
- No nested conditionals in V1; no loops, parallel blocks, freeform diagram edits, arbitrary shapes, SVG, HTML, Mermaid, or layout code in agent output.
- A native Rust renderer (potentially through Rune or its diagram support, after prototype validation) computes positions and clickable regions from a strictly validated application-owned contract. Visual language is monochrome/neutral, with one accent reserved for directly changed interactions; selection appearance is independent of changed status.
- Source references are to immutable commit-bound code, and actual excerpts are read by the app rather than trusted from agent output.
- Unknown element types, nonexistent references, unsupported relationships and excessive complexity are rejected deterministically; semantic claims remain the agent's evidence responsibility.
- Summary should prioritize what the PR **actually implements**, not paraphrase the original PR body.
- Compare meaningful claims from developer description to implementation; surface only significant mismatches/uncertainties with source evidence, not exhaustive checklists or automatic defect labels.

## 8. Findings and suggestion rules

- Severity: `critical | high | medium | low`. Confidence is separate from severity.
- One actionable concern per finding; include title, explanation, concrete failure conditions, impact, evidence, and one or more source locations. Exclude speculative/style-only issues unsupported by project rules.
- Multiple suggested alternatives per finding may include **text comment** or **GitHub-style code replacement**. Reviewer stages one alternative by default; can author additional manual comments.
- Code suggestions are feedback rendered into GitHub suggestion Markdown by the app, **not changes applied to source files**.
- Reviewer triage (`pending`, `staged`, `dismissed`/deferred) is application-owned, not dictated by AI-generated finding status.
- Related GitHub review threads are explicit links, validated against actual fetched threads; semantically duplicate comments should not be suggested by default.
- Follow-up findings: new/unresolved/regressed visible by default, resolved summarized and optionally expandable.
- Review coverage: examined areas, relevant tests/checks, excluded paths with reasons, limitations; don't invent precision percentages.

## 9. Consistency and publication safety

- A visible generated review has a single pinned comparison context: merge-base -> PR head. No cross-tab mixing of different heads.
- The reviewed PR diff follows GitHub-style three-dot/merge-base comparison. Latest remote target may additionally be inspected for integration risks, without contaminating the PR diff with unrelated target-branch changes.
- GUI may check **remote metadata/head status only** while active. It does not silently replace pinned review content.
- Before publication, validate current head, source/target line anchors, diff side, code-replacement span, and thread state; if code moved/changed/disappeared, require manual action instead of silently attaching to a wrong location.
- Exact code equality/line matching can support deterministic validation, but cannot guarantee semantic validity after external behavior changes; communicate uncertainty.
- Batch publish **as a user workflow**, not necessarily as one API request; new inline comments + review event can use a single GitHub review API call, existing thread replies/resolutions require separate calls.
- Treat transport failures as possibly **unknown** outcomes; reconcile with GitHub before retrying. Never claim multi-request atomicity.
- A fresh AI run never automatically cancels a user's existing draft: explicit confirmation is required, then successful validated replacement clears it.

## 10. AI and contract governance

- The agent reads a **small authoritative context manifest** from the `bodh` CLI, retrieves full diffs/discussions on demand and explores surrounding repository code independently using read-only tools.
- Do not flood agent context with full patches by default; still require adequate coverage of relevant changed code.
- The agent submits a complete structured result for overview, diagrams, findings, follow-up assessments and coverage.
- Application owns versioned schemas, source validation, persistence and rendering; skill owns review methodology, evidence standards and bounded retry instructions.
- Reject unsupported fields and elements; return machine-readable errors. Never partially commit a result.
- Any discrepancy requiring contract changes must follow: **document proposal → update canonical contract → implement Rust app/CLI → update skill → tests/examples → documentation/release compatibility**. No silent unilateral extension by agent or app.

## 11. Non-functional and design-quality requirements

- **Design process:** Every FigCraft screen requires the user's explicit approval; geometry specimen board is approved before screen design, and populated Findings and Changes take priority. A static screenshot or generic generated layout is not evidence of completed UX behavior.
- **Layout precision:** Shared spacing/padding tokens, reusable auto-layout components, deterministic alignment, centered icon/text groups and repeated-row geometry. Test numeric bounds where tooling exposes them; never fabricate a numerical QA result. Block known spacing, clipping, baseline, and button-centering defects.
- **Flow state:** Preserve file/finding/line/diagram selection, visible code and scroll offsets across ordinary comment, reply, stage, tab and drawer operations. No route changes for routine feedback or confirmation modals for reversible actions.
- **Readability:** Source code is visually dominant, readable at typical desktop widths, with a scalable monospace size; no tiny font workaround for narrow panes. Dense but calm, minimal borders/cards, subtle semantic colors, keyboard-visible focus and dark/light consistency.
- **Responsiveness:** File tree, diff navigation and source inspector remain responsive on realistically large PRs with lazy/virtualized rows and bounded caches. Set numeric latency/memory targets after profiling real Linux hardware.
- **Reliability:** Persist drafts transactionally; preserve active review if agent validation fails; safe partial publication recovery.
- **Accessibility:** Keyboard navigation, accessible hit targets, resizable panes, high-contrast-compatible text, non-color changed-diagram cue, reduced-motion support.
- **Security:** No shell interpolation of user-provided paths/text; process argument arrays; no app-managed GitHub tokens; untrusted PR Markdown and AI output never executable.
- **Resource usage:** No resident daemon or polling when closed; pause remote polling when inactive; CLI terminates after its operation.
- **Integrity:** Never modify checked-out branch, index, or working files during review; code shown in inspector is from immutable pinned commits.
- **Privacy:** No Bodh-owned cloud service for repo or review data in V1.

## 12. Out of scope (explicit)

- App-initiated AI reviews, agent selection/management, continuous AI review or automatic re-review.
- Source code editing, applying code suggestions to local files, LSP, general IDE functions.
- Standalone commit-range and unpushed local-change review; remote-only PR review without a local checkout.
- Generic GitHub provider abstraction or native GitHub SDK/auth; non-GitHub forges.
- Full PR revision history/snapshot browser, full audit trail, analytic dashboards.
- Full UML or arbitrary Mermaid rendering, freeform diagram creation/editing, nested conditional/parallel/loop diagrams.
- Background services, automatic cloning, modifying local branches, automatic merging or AI-generated GitHub actions.

## 13. Acceptance criteria

### AC-01 Manual review
A developer with a local checkout and authenticated `gh` can inspect a PR and publish an explicitly confirmed review without a coding agent.

### AC-02 Headless agent review
A compatible agent runs `bodh review prepare → analyze → bodh review validate → bodh review commit`, produces source-grounded artifacts and exits without requiring an open GUI or persistent background process.

### AC-03 Diagram navigation
A valid structured sequence diagram renders consistently in the approved native UI framework; a click opens the corresponding pinned-source excerpt in the adjacent inspector; unsupported graph structures are rejected.

### AC-04 Multiple flows
A PR with distinct meaningful flows can present multiple selectable diagrams without displaying all diagrams simultaneously; an unsuitable PR can have none.

### AC-05 Findings triage
A finding with multiple alternative suggestions allows one default staged selection, editing, dismissal and source navigation; no suggestion is published without user confirmation.

### AC-06 GitHub discussions
Existing threads and replies are visible; a user can stage a reply and a resolve/reopen action, then publish them through Finish Review.

### AC-07 Commit freshness
New remote commits trigger a visible stale indicator but do not silently mutate pinned review UI. Publication rejects or flags invalid/stale inline anchors.

### AC-08 Draft overwrite guard
An agent follow-up encountering unpublished feedback obtains explicit user approval; failed analysis or stale permission leaves data unchanged; successful commit replaces review and clears authorized drafts atomically.

### AC-09 Partial publication
If one of several required GitHub writes fails or has unknown outcome, successful actions aren't blindly resubmitted and failed/unknown drafts remain recoverable.

### AC-10 Contract compliance
Unknown fields, invalid enums, nonexistent source references, incompatible versions and invalid diagram structure cause structured validation errors with no partial persistence.

### AC-11 No worktree mutation
Opening/reviewing a PR never checks out the branch or overwrites working-tree files.

### AC-12 Inline commenting without navigation
A reviewer can select a line or multiline range in Changes, open an inline composer **beneath the selected range**, stage the draft and continue at the same code location without a forced route change, detached form or lost scroll position.

### AC-13 Finding workspace and code proximity
Findings uses a persistent resizable **left title/severity/files list** and **right actual-code/evidence/suggestions detail pane**. Selecting a finding updates the detail in place and comments can be edited/staged below relevant code.

### AC-14 Contextual Finish Review drawer
Finish Review opens as a right-side drawer; staged feedback and decision are scannable, and closing it returns to the original selected code/finding without draft loss.

### AC-15 Design system and approval gates
Before any application UI implementation, the FigCraft token system and reusable geometry specimen board pass measured/visual QA; every subsequent populated screen is explicitly approved at its gate, including dark/light and viewport checks. No known alignment, overflow or unreadable-code blocker is accepted.

### AC-16 Framework feasibility gate
A pinned Rune revision must be tested on Linux/Wayland for actual `DiffView` annotations, multi-line comments, text editing and clipboard, resizable panes, interactive diagram mapping, idle CPU, and large virtualized diffs. A failed probe requires reconsidering the UI framework, **not weakening the accepted UX**.

## 14. Outstanding implementation decisions (not blockers to product scope)

1. Choose the native desktop UI framework after the **Rune + rune-kit proof of concept**; the archived framework is not authoritative. Verify Linux/Hyprland compatibility, text input, accessibility and high-refresh idle behavior.
2. Determine whether `rune-kit` diff/syntax components and `rune-mermaid` meet the validated UX/contract needs or whether a custom Rust renderer / Tree-sitter integration is necessary. No agent-authored raw Mermaid is permitted.
3. Freeze precise CLI/schema count/size limits, migrations and persistent draft versioning using fixtures and contracts.
4. Finalize active-only polling behavior, large diff caching and binary/submodule fallback handling after testing.
5. Establish safe GitHub publication reconciliation for indeterminate responses and SHA changes without blind retries.
6. Packaging, desktop ID and Linux support matrix, after the framework decision.

**Naming rule:** Use **Bodh** for product-facing text and `bodh` for CLI commands/binaries. Keep the existing `pr-review` coding-agent skill name; avoid renaming it solely for branding. GitHub repository naming and Linux desktop identifiers are subject to availability checks.

**Change control:** Requirements here are the agreed product baseline. If development reveals a contract discrepancy, update this PRD and the canonical spec before implementing behavior; synchronize CLI, GUI, skill, fixtures and documentation in the same change.
