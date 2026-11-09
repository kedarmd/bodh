# Bodh — V1 Technical & Contract Specification

**Product:** Bodh (बोध)  
**CLI executable:** `bodh`  
**Coding-agent skill:** `pr-review` (existing name retained)  
**Status:** V1 technical and contract baseline for the complete application lifecycle — proposed technology choices and contracts are refined through validation; delivery gates tracked separately  
**Updated:** 2026-11-08  
**Companion:** [PRD.md](./PRD.md)  
**Audience:** Product/design owners and future implementers of Bodh's Rust core, UI, CLI, contract validation, testing, and `pr-review` skill

> **Invariant:** Bodh owns Git/GitHub context, contracts, validation, rendering, local reviewer state and publication. The coding agent owns semantic review analysis. The human owns comments, review decisions and all remote mutations. A fully functional manual PR review requires no AI.

This document defines V1 architecture, internal data boundaries, CLI behavior, the proposed canonical `1.0` review-result contract, validation, interaction state, and failure recovery **for implementation, integration, testing, and release as well as design**. The previous desktop UI implementation has been archived: domain contract decisions persist, but previous component/layout decisions do not. Current authorization is tracked in [PROJECT_STATUS.md](PROJECT_STATUS.md) and design approvals in [design/DESIGN_STATUS.md](design/DESIGN_STATUS.md); those workflow records do not limit the scope of this specification. Requirements from the PRD take precedence over illustrative command flags, limits, and table names herein.

## 1. Decisions and constraint register

| Concern | V1 contract |
|---|---|
| Platform | Linux only; Wayland-first, supported environments to be verified |
| Desktop | Rust; **Rune + rune-kit is preferred pending Linux/Wayland feasibility prototype and design approval**; native diagram and code views, no WebView |
| Highlighting | Source highlighting mandatory; evaluate Rune-kit syntax/diff built-ins against Tree-sitter for required languages; no LSP dependency |
| Repo operations | Local `git` executable and immutable commit objects; no checkout/worktree editing |
| GitHub | GitHub PRs only, authenticated `gh` and `gh api` REST/GraphQL, including GitHub Enterprise where supported by `gh` |
| Storage | Local SQLite and bounded cache; single mutable current workspace per PR |
| AI | External developer coding agents call existing `pr-review` skill, which calls `bodh` CLI |
| GUI → AI | Not supported V1; GUI does not spawn AI or own a model key |
| CLI | Headless, deterministic, no GUI or resident daemon required; each invocation exits |
| Review comparison | Merge-base of pinned target/base and PR head → pinned PR head, not target-tip → head |
| Artifact contract | Application-owned, versioned, strict; unknown fields rejected |
| Diagrams | Native deterministic renderer; monochrome, single accent for changed interactions; single-level `alt`/`opt` |
| Findings | Separate issue from alternative feedback; one selected suggestion per finding by default |
| Publishing | User-triggered **Finish Review**; local staging first, remote API operations individually tracked |
| Follow-ups | Update same workspace only after successful full validation; compact resolved summary |
| History | No full prior-review snapshot archive; preserve current workspace and published GitHub state |
| Network/privacy | No Bodh backend; `gh` talks to GitHub; agent provider may handle source according to user's configured agent |

### 1.1 Terms

- **Repository identity**: canonical GitHub host + repository owner/name (case normalized for identity); associated with one or more local checkouts.
- **PR identity**: host + repository identity + pull request number, **not** local branch name.
- **Context**: immutable tuple of PR identity, target/base SHA, head SHA, merge-base SHA, and generated diff metadata; identified by `context_id`.
- **Review result**: complete validated AI-produced Overview, diagrams, findings, follow-up assessments and coverage for one context.
- **Workspace**: single mutable persisted PR review state, with an optional current result plus reviewer-owned drafts and preferences.
- **Source reference**: path + immutable revision + file line range. Not itself an inline GitHub comment anchor.
- **Diff anchor**: computed API-compatible location (`path`, side, lines, commit) validated against the pinned/current diff.
- **Suggestion**: agent-proposed wording or code replacement; **draft**: local reviewer-approved/editable payload that may derive from a suggestion.
- **Thread**: actual GitHub PR review discussion with provider IDs and current remote state.
- **Revision freshness**: whether the displayed pinned context still matches the PR's latest remote head.

## 2. System architecture

```text
                       CODING AGENT PROCESS
             (Codex / OpenCode / Claude Code / etc.)
                               |
                     existing pr-review skill
                    methodology + evidence rules
                               |
                               v
                        bodh CLI (Rust)
                  prepare / inspect / validate / commit
                               |
                   +-----------+-----------+
                   |     Shared Rust core   |
                   |                       |
                   | git / gh adapters     |
                   | diff + source mapping |
                   | strict contracts      |
                   | local state + drafts  |
                   | publication journal   |
                   +-----------+-----------+
                               |
                        SQLite + cache
                               |
                       Bodh native desktop (UI framework selected at feasibility gate)
                Overview / Changes / Findings
                source inspector + inline comments + Finish Review drawer
                               |
                     gh CLI for remote actions
                               |
                          GitHub API
```

### 2.1 Module ownership

| Module (illustrative Rust crate) | Owns | Never owns |
|---|---|---|
| `bodh-core` | Domain models, PR identities, invariants, diff mapping, validation interfaces | UI framework, AI reasoning |
| `bodh-contract` | Canonical JSON Schema, parsing, version dispatch, semantic-structure validation, fixtures | View layout |
| `bodh-git` | Read-only source access, object availability, merge-base, diff parsing, line mapping | Checkout/reset/worktree edits |
| `bodh-github` | `gh` subprocess calls, normalization of PR metadata and review discussions, publication steps | Native token management or SDK calls |
| `bodh-store` | SQLite migrations, current result, draft revisions, operation journal, cache metadata | Remote truth |
| `bodh-cli` | Headless commands, stable JSON envelopes, contract discovery, validation/commit orchestration | GUI process dependency |
| `bodh-app` | Native UI adapter, navigation, inline diff annotations, Findings master-detail, diagrams/inspector, review drawer, editing and confirmation | Agent execution, freeform model outputs |
| `pr-review` skill | Review methodology, code exploration/evidence, structured result generation, bounded correction | HTML/JS reports, `gh` mutation or app-specific rendering |

These are *logical boundaries*, not mandatory crate names. Keep shared business rules in Rust core so CLI and GUI cannot diverge.

### 2.2 Process and resource policy

- No background daemon, localhost action server, persistent model connection, or mandatory GUI session.
- CLI does its work, emits machine-readable output, exits with a stable exit code.
- GUI may refresh remote status while **visible and active**; pause when closed/inactive/unfocused according to measured behavior.
- All potentially long Git/`gh` operations run away from the UI thread and support cancellation/timeouts.
- Repository/cache reads are lazy and bounded; large diffs are virtualized and parsed incrementally where practical.

## 3. Git and GitHub data acquisition

### 3.1 Prerequisites and repository identification

- Verify `git` and `gh` are present, executable and compatible with required features.
- Resolve `git rev-parse --show-toplevel` and origin/selected GitHub remote without assuming `origin` always exists.
- Resolve GitHub host and canonical repo via `gh repo view` or API. Do not assume `github.com`; host must be consistent with authenticated `gh` account and remote.
- Reject non-Git checkouts, ambiguous remote identity and PRs belonging to another repository unless the user explicitly selects a matching local repo.
- Local uncommitted edits, index contents and checked-out branch **must not influence pinned PR source or diff**.
- `review prepare` may register an otherwise unseen checkout in Bodh's local registry; "non-destructive" means it does not erase review/draft data or mutate source, not that it performs zero local metadata writes.

### 3.2 Pinned comparison

1. Resolve PR number/URL through `gh` and authoritative `head.sha` and `base.sha`; capture original PR Markdown verbatim as remote content.
2. Confirm the head/base commits are available in local Git object storage. Fetch the specific missing objects or PR ref **without checkout, reset, merge or branch switching**. A Git fetch can update Git object/ref metadata, never the worktree; clearly report permissions/fetch failure.
3. Calculate `merge_base_sha = git merge-base base_sha head_sha`.
4. Compute changes for **`merge_base_sha..head_sha`** (GitHub three-dot semantics). Include renames, additions, removals and both sides' line mappings.
5. Generate `context_id` from a versioned, canonical hash of host/repo/PR/base/head/merge-base/diff semantics; context identity must be stable across repeat prepares for the same input.
6. Capture latest observed PR metadata, existing review discussions and thread IDs, available viewer permissions, plus previous current findings/draft counts for agent context.
7. Expose the diff and file content **on demand**, rather than printing full patches and all threads in the initial agent response.

When the target branch moves without a new head commit, a freshly prepared context can have a different merge-base. Distinguish this from changes solely to remote metadata. Do not silently mix contexts.

**Source retrieval:** read exact blob contents with `git show <commit>:<path>` or an equivalent Git object API; validate paths against the commit tree. Do not read from the working tree for review evidence by default. Disable external diff/textconv hooks; use argv-based process invocation, not interpolated shell commands. Handle CRLF, missing terminal newlines, renames, binary assets, submodules, Git LFS pointers and nonexistent historical paths explicitly.

### 3.3 Remote discussions

- Fetch GitHub review threads with GraphQL (IDs, line/range, diff side, resolved/outdated status, viewer permissions) and comments/replies with their provider IDs; paginate until complete or report a meaningful limitation.
- Preserve original Markdown and author attribution; sanitize/render as untrusted content.
- Thread resolution and reply permissions are checked again before publication.
- An outdated or resolved thread does **not** prove its underlying issue is fixed.

## 4. Bodh CLI contract

### 4.1 Command surface

The following is the **proposed V1 command surface**, to be settled by fixture-backed CLI tests before release. The semantic operations are required; minor spelling/flag variations are implementation decisions.

```bash
# Context and metadata (agent headless path)
bodh review prepare --pr 142 [--repo /path/to/checkout] [--json]
bodh review inspect --context-id ctx_... --section files|diff|threads|source [section-specific flags]
bodh contract describe [--version 1.0] [--json]
bodh contract schema --version 1.0

# Complete AI result validation and replacement
bodh review validate --input review.json [--json]
bodh review commit --input review.json [--approval-id auth_...] [--json]

# Optional desktop entry point, never needed for headless analysis
bodh review open --pr 142 [--repo /path/to/checkout]

# Interactive reviewer-authorized draft replacement, only when needed
bodh review authorize-replace --context-id ctx_...
```

For manual review, the desktop itself calls shared Rust services instead of subprocess-spawning the public CLI for every operation. The CLI need not expose GUI-only draft controls in V1.

### 4.2 JSON envelopes and error behavior

`--json` outputs **one structured JSON envelope** on stdout; diagnostics/progress on stderr. No ANSI/color/progress text in JSON stdout. Suggested envelope:

```json
{
  "protocol_version": "1.0",
  "ok": true,
  "data": { "context_id": "ctx_example", "head_sha": "<40-hex-sha>" },
  "warnings": []
}
```

Errors use `ok:false`, `error.code`, human-readable `error.message`, optionally `error.details[]` with JSON pointers or file/line information; never embed secrets. Stable nonzero exit codes distinguish input/contract failure, stale context/concurrent modification, authentication/access, network/remote uncertainty and internal failure. Exact numeric codes are to be frozen in CLI fixtures.

**Validation error example:**

```json
{
  "protocol_version": "1.0",
  "ok": false,
  "error": {
    "code": "CONTRACT_VALIDATION_FAILED",
    "message": "Review result was not committed",
    "details": [
      {
        "pointer": "/diagrams/0/steps/1/source_refs/0/start_line",
        "code": "SOURCE_LINE_OUT_OF_RANGE",
        "message": "Line 204 does not exist in the pinned head blob"
      }
    ]
  }
}
```

### 4.3 `prepare` output (small manifest)

Must include: repo/PR identity; original description availability; author/title; base/head/merge-base; normalized changed-file index (counts, paths, rename status, binary status); current discussion count/IDs; previous-result availability; draft counts **by type**; workspace revision; contract-supported versions; references or CLI commands for on-demand access. The returned JSON must not unconditionally contain every patch, full file, thread body, or a generated report.

The agent independently explores surrounding code and tests with read-only tools. Review completeness is the agent's responsibility; Bodh may supply authoritative file snapshots and diff anchors so evidence matches the pinned review.

### 4.4 `validate` and `commit`

- `validate`: parse whole input, enforce schema, referential integrity, file/line existence, contract constraints and diff anchoring of suggested replacements. **No review-state mutations.**
- `commit`: re-run all validation; confirm context SHA, workspace generation and authorization state; persist the **entire** new result and any authorized draft resets within one SQLite transaction. Never persist a partial result.
- A failed or interrupted commit retains the previous current review and drafts.
- Rejected data is returned with machine-actionable errors; the skill may perform a bounded number of targeted corrections. Do not auto-resubmit infinitely.
- Idempotent re-commit of the exact current result must not create duplicate local reviews or erase fresh drafts. A changed result cannot bypass concurrency checks.

### 4.5 Human authorization for draft replacement

If the workspace contains **any unpublished local drafts** (new inline comments, edited suggestions, replies, pending thread actions, or review summary):

1. `prepare` exposes typed counts and revision; the skill **must warn** the user that a successful new AI result replaces the old result and will discard all those drafts.
2. Require explicit user approval. Prefer an interactive CLI confirmation or a native GUI approval that persists a narrow authorization record; a headless agent must not silently forge consent with a generic force flag.
3. Authorization binds exact PR identity, workspace/draft revision, intended replacement context and expiry/one-time use (specific token mechanics are implementation details).
4. Validate **again at commit**; if drafts were added/edited or context changed after authorization, reject as stale and require renewed approval.
5. If analysis/validation fails, **do not consume drafts**. If commit succeeds, replace current generated result and delete precisely the drafts covered by explicit reset authorization atomically.

A manual-only update to the latest PR revision also requires explicit confirmation if it would discard unpublished drafts. This authorization rule applies only to destructive replacement, not routine remote-status refresh.

## 5. Canonical AI result contract (`schema_version: "1.0"`)

### 5.1 Governance and trust model

- **Application-owned schema** is the only accepted shape; canonical JSON Schema, prose semantics, valid/invalid fixtures and Rust types must align.
- `additionalProperties: false` throughout externally accepted structures, except expressly documented extension points (none in V1).
- `schema_version` is a string; major-breaking changes require a new supported version. CLI exposes supported versions and current schema. Reject unknown versions.
- Agent-generated IDs are unique **within one complete result**; Bodh assigns its own persistent DB identities. The `context_id` must resolve to a prepared context and match its exact head/merge-base.
- Schema validation checks shape and source existence, **not truth of the agent's interpretation**. Evidence and confidence remain semantic claims that require a careful coding-agent review.
- No agent-provided coordinates, CSS, theme colors, Markdown-embedded executable code, custom widgets, SVG, Mermaid, arbitrary SQL or GitHub API request payloads.
- Text length and array-count safety bounds will be fixed with test fixtures after diagram/diff performance prototypes; V1 rejects clearly unbounded payloads rather than truncating silently.

### 5.2 Top-level shape

```json
{
  "schema_version": "1.0",
  "context_id": "ctx_example",
  "overview": {
    "summary": "...",
    "key_changes": [],
    "intent_alignment": { "assessments": [] }
  },
  "diagrams": [],
  "findings": [],
  "follow_up": { "assessments": [] },
  "coverage": {
    "examined": [],
    "checks": [],
    "excluded": [],
    "limitations": []
  }
}
```

All top-level members are mandatory. **Empty `diagrams`, `findings`, and `follow_up.assessments` are valid.** An agent must not create artificial flow diagrams or findings to satisfy a count. Array ordering has deterministic presentation semantics where noted.

### 5.3 `SourceRef`

```json
{
  "file": "src/auth/session.ts",
  "revision": "head",
  "start_line": 48,
  "end_line": 52
}
```

- `file`: canonical repository-relative POSIX path, valid in the named commit tree. No absolute, parent-traversing, or ambiguous path.
- `revision`: `head` or `base`; here `base` means **the comparison merge-base blob**, not the latest target tip. Use `base` for deleted/old code references.
- `start_line`, `end_line`: positive 1-based inclusive line numbers; `start_line <= end_line` and both exist in pinned blob.
- Source reference can show unchanged context; it is **not** required to intersect a changed line unless it is used as a new inline comment/suggestion target.
- File rename relationships, deletion and side are resolved by core diff mapper, **not** sent as independent arbitrary agent status fields.
- For source-backed diagram messages/findings and claims, at least one concrete ref is mandatory. Non-code limitations are described without invented refs.
- A primary finding ref may legitimately be outside the changed hunk; then a new GitHub inline comment requires reviewer reattachment to a valid diff anchor or a general review comment instead.

### 5.4 Overview and intent alignment

```json
{
  "summary": "Two to four concise sentences explaining the implementation.",
  "key_changes": [
    {
      "id": "change-token-validation",
      "title": "Add token-expiry verification",
      "description": "Authentication now rejects expired tokens.",
      "source_refs": [
        { "file": "src/auth/session.ts", "revision": "head", "start_line": 48, "end_line": 57 }
      ]
    }
  ],
  "intent_alignment": {
    "assessments": [
      {
        "id": "claim-expiry",
        "status": "potential_mismatch",
        "original_claim": {
          "quote": "All expired sessions are rejected.",
          "occurrence": 1
        },
        "explanation": "One fallback route bypasses the new expiry guard.",
        "evidence_refs": [
          { "file": "src/auth/fallback.ts", "revision": "head", "start_line": 15, "end_line": 23 }
        ],
        "finding_ids": ["finding-fallback"]
      }
    ]
  }
}
```

**Interpretation:**

- `summary` describes the actual implementation, **not** a second rewrite of the author's description.
- `key_changes` is a curated list; each item has concrete source evidence. Avoid redundant microchanges.
- `intent_alignment.assessments` contains claims extracted from **original PR Markdown**, retrieved by `gh` and stored by Bodh separately. Do not echo the whole PR description in output.
- `original_claim.quote` must be an **exact contiguous substring** of the retrieved original description; `occurrence` identifies which match when repeated (1-based). CLI verifies quote against the pinned original description snapshot; agent may not invent claims. A quote location is a textual locator, not a claim of semantic verification.
- Allowed `status`: `aligned`, `partially_aligned`, `potential_mismatch`, `unverified`. In V1 UI, **only meaningful partial, mismatch or unverified assessments** are shown prominently. `aligned` need not be generated or displayed.
- `finding_ids` may reference a real finding when an implementation mismatch is an independently justified defect. A mismatch alone must not automatically become a finding.
- If no significant discrepancy is found, an empty assessments array is valid; UI language must not promise complete correctness.
- Developer description preview is a deterministic excerpt from the **original** Markdown, not another AI summary; full Markdown opens in a resizable scrollable reader in Overview.

### 5.5 Sequence diagram contract

**Intent:** represent the main causal/runtime interaction flow, not the complete control-flow graph. Produce zero or more **distinct** diagrams; display only one at a time.

```json
{
  "id": "auth-flow",
  "title": "Authentication request",
  "summary": "How the changed token check affects session lookup.",
  "participants": [
    { "id": "api", "label": "API", "category": "component" },
    { "id": "auth", "label": "Auth Service", "category": "service" },
    { "id": "db", "label": "Database", "category": "storage" }
  ],
  "steps": [
    {
      "id": "m1",
      "type": "message",
      "from": "api",
      "to": "auth",
      "label": "Validate token",
      "description": "The API delegates the new expiry check.",
      "source_refs": [
        { "file": "src/api/auth.ts", "revision": "head", "start_line": 20, "end_line": 24 }
      ]
    },
    {
      "id": "a1",
      "type": "alt",
      "branches": [
        {
          "condition": "Token valid",
          "steps": [
            {
              "id": "m2",
              "type": "message",
              "from": "auth",
              "to": "db",
              "label": "Load session",
              "source_refs": [
                { "file": "src/auth/service.ts", "revision": "head", "start_line": 45, "end_line": 52 }
              ]
            }
          ]
        },
        {
          "condition": "Token expired",
          "steps": [
            {
              "id": "r1",
              "type": "return",
              "from": "auth",
              "to": "api",
              "label": "Reject token",
              "source_refs": [
                { "file": "src/auth/service.ts", "revision": "head", "start_line": 55, "end_line": 58 }
              ]
            }
          ]
        }
      ]
    }
  ]
}
```

**Participant:** unique `id`, concise `label`, optional fixed `category` (`actor | component | service | storage | external`). Category provides accessible semantics only, **not** arbitrary color or layout.

**Allowed step variants:**

| Type | Required data | Rendering/validation |
|---|---|---|
| `message` | `id`, `from`, `to`, `label`, `source_refs` | Distinct participant IDs; solid arrow |
| `return` | `id`, `from`, `to`, `label`, `source_refs` | Distinct participant IDs; dashed arrow |
| `self_call` | `id`, `participant`, `label`, `source_refs` | Loopback on same participant |
| `alt` | `id`, `branches` | At least two labelled nonempty branches; contains only simple steps/notes |
| `opt` | `id`, `condition`, `steps` | One labelled conditional block; contains only simple steps/notes |
| `note` | `id`, `text`, `participant`, `source_refs` | Short contextual note; no freeform layout/content |

`alt`/`opt` may appear at the **top level only**: no nested conditionals. No loop, parallel, freeform grouping, custom participants, custom drawing primitive, markdown injection or agent-defined appearance. Step IDs are unique within a diagram, including inside branches. Participant IDs resolve locally. Normal message/self-call/return steps must contain nonempty source references; note refs anchor the context being explained. Condition text and message content are concise; structure must not assert unverified execution order.

**Changed/unchanged semantics:** Rust core intersects a step's source references with real changed diff lines. Only **directly changed interaction segments** receive the single theme accent; neutral for existing context. A referenced unchanged function that is behaviorally affected remains neutral, with its influence explained in prose. Do **not** color entire participant/lifeline/`alt` frame. Independently selected interactions receive a distinct outline/weight, not a false changed marker. Provide a non-color cue (e.g., heavier stroke) and an accessible description.

**Layout:** deterministic top-to-bottom step order, stable participant order as declared, consistent text wrapping; layout engine controls coordinates, margins, hit regions, arrows and frames. Renderer is native and framework-dependent only in the presentation adapter; strict contract remains framework-independent. A focused early prototype must test collision avoidance, resizable right-side source inspector, conditional frames, hit testing, large labels, theme contrast and performance. Evaluate a suitable pure-Rust layout dependency only if it supports **exact** restricted semantics and native drawing in the approved UI framework; fallback is a small custom deterministic renderer. No WebView/Mermaid-rendered HTML in V1.

**Source navigation:** click a step → open/reuse adjacent resizable inspector showing actual pinned-commit source, syntax highlighting, line numbers and relevant linked finding(s), with navigation to Changes. Multiple source refs are switchable within inspector. Persist flow selection/inspector state for current workspace.

### 5.6 Findings and suggestions

```json
{
  "id": "finding-fallback",
  "title": "Fallback route bypasses expiry validation",
  "severity": "high",
  "confidence": 0.94,
  "summary": "Expired tokens still authenticate via the fallback handler.",
  "reason": "The handler verifies the signature but never invokes the new expiry guard.",
  "impact": "Requests carrying expired tokens can be accepted on this route.",
  "evidence": {
    "explanation": "The fallback handler returns a session before expiry validation.",
    "source_refs": [
      { "file": "src/auth/fallback.ts", "revision": "head", "start_line": 15, "end_line": 23 }
    ]
  },
  "locations": [
    {
      "role": "primary",
      "source_ref": { "file": "src/auth/fallback.ts", "revision": "head", "start_line": 19, "end_line": 19 }
    },
    {
      "role": "supporting",
      "source_ref": { "file": "src/auth/service.ts", "revision": "head", "start_line": 45, "end_line": 58 }
    }
  ],
  "suggestions": [
    {
      "id": "text-comment",
      "type": "comment",
      "comment": "Could this route invoke the same expiration validation as the main authentication flow?"
    },
    {
      "id": "replace-handler",
      "type": "code_replacement",
      "comment": "Apply the expiration check before returning the session.",
      "target_ref": { "file": "src/auth/fallback.ts", "revision": "head", "start_line": 19, "end_line": 19 },
      "replacement_code": "return validateExpiry(token).andThen(loadSession);"
    }
  ],
  "related_thread_ids": []
}
```

*Example values illustrate syntax and must be replaced by evidence from the actual review; the code shown is not a recommendation for a real repository.*

- Required: `id`, `title`, `severity`, `confidence`, `summary`, `reason`, `impact`, `evidence`, at least one `locations` entry with **exactly one `primary`**, `suggestions` (may be empty), `related_thread_ids` (may be empty).
- `severity` enum: `critical | high | medium | low` (issue impact). `confidence`: bounded `0..1` subjective model estimate, **not** a deterministic correctness metric; UI may render a coarser indication instead of false precision.
- Evidence requires an explanation and source references. Primary location is where reviewer starts; supporting refs may be elsewhere in code. Missing tests alone are not sufficient proof of a functional defect.
- Findings must be distinct, evidence-backed, relevant to the PR; avoid formatting/subjective preferences without repository conventions or credible consequences.
- Suggestions are **alternatives**, not automatically staged, and may be empty. V1 variants are `comment` and `code_replacement` only.
- A `comment` is editable prose. It can use the finding's primary location if it maps to an allowed diff comment anchor, otherwise reviewer chooses a valid diff line or general review body.
- A `code_replacement` has editable comment, exact head-side diff target and replacement source text. App, **not agent**, emits valid GitHub fenced `suggestion` Markdown and ensures the replacement range is supported. It never applies code changes locally.
- Only **one selected suggestion per finding is staged by default**. Selecting a different alternative with an edited draft requires a preservation/replace confirmation. Reviewer may create independent additional manual drafts.
- `related_thread_ids` must refer to actual GitHub GraphQL review-thread IDs associated with this PR. Agent identifies semantic relationship; CLI checks ID existence. Prefer replying to existing discussion over duplicating already active feedback.

**GitHub anchoring caveat:** A valid source location may not be valid as a review-comment anchor. The CLI MUST distinguish `SOURCE_VALID` from `INLINE_COMMENT_ANCHOR_VALID`, including source side, line ranges, and the actual diff. Invalid suggestion targets cause contract validation failure; a text finding can remain usable as general feedback without fabricating an anchor.

### 5.7 Follow-up assessments

Follow-up analysis compares previously identified concerns with the newly reviewed head. The agent receives only the current prior findings and remote discussions, **not** a full report history archive.

```json
{
  "assessments": [
    {
      "previous_finding_id": "finding-previous-001",
      "status": "resolved",
      "explanation": "The new validation occurs before the session is returned.",
      "evidence_refs": [
        { "file": "src/auth/session.ts", "revision": "head", "start_line": 48, "end_line": 56 }
      ],
      "current_finding_id": null
    },
    {
      "previous_finding_id": "finding-previous-002",
      "status": "unresolved",
      "explanation": "The error path is still unhandled.",
      "evidence_refs": [
        { "file": "src/api/auth.ts", "revision": "head", "start_line": 70, "end_line": 75 }
      ],
      "current_finding_id": "finding-current-002"
    }
  ]
}
```

Allowed status: `resolved | unresolved | regressed | uncertain`. For `unresolved` and `regressed`, `current_finding_id` must identify an actionable current finding; `resolved` has no current finding; `uncertain` needs an explicit explanation of what cannot be verified. New issues are current findings **not referenced** by a previous assessment; current UI derives "new" from this relationship. A previously dismissed finding is prior context, but dismissal is human state—not proof of resolution—and should not be silently recreated as duplicate actionable feedback without new evidence.

**Critical persistence boundary:** V1 stores the **current** review result plus a compact follow-up display representation (e.g., resolved issue title/previous evidence summary where needed to explain resolution). It must not retain every prior full overview/diagram/coverage/JSON result indefinitely. Where a previous finding ID would otherwise vanish on replacement, its compact resolved assessment is folded into the new current result transaction. Any reference to old code uses the old pinned revision only if that compact evidence is explicitly retained/accessible; otherwise show previous issue text and the **current** fixing evidence, not a false current line link.

Follow-up is not inferred from GitHub thread resolution state: an older "resolved" discussion can still correspond to a live issue.

### 5.8 Review coverage

```json
{
  "examined": [
    {
      "area": "Authentication and token validation",
      "notes": "Reviewed changed handlers and their immediate callers.",
      "source_refs": [
        { "file": "src/auth/session.ts", "revision": "head", "start_line": 1, "end_line": 82 }
      ]
    }
  ],
  "checks": [
    {
      "name": "Authentication unit tests",
      "status": "not_run",
      "detail": "No test execution was available in this review environment."
    }
  ],
  "excluded": [
    { "area": "Generated API client", "reason": "Generated output; checked inputs only." }
  ],
  "limitations": [
    "Did not verify behavior against a running identity provider."
  ]
}
```

`examined`: areas actually read/assessed, with supporting refs when code-backed. `checks`: `passed | failed | not_run | unavailable` and descriptive evidence/limitation; "passed" must correspond to a check the agent actually executed or whose recorded result it reliably inspected, never a fabricated test run. `excluded`: area and reason. `limitations`: honest uncertainties. The GUI presents an expandable, compact section near the bottom of Findings. Do **not** display misleading coverage percentages or imply that omitted limitations mean exhaustive review.

### 5.9 Cross-object validation

Validation passes must run in this order so errors are actionable:

1. Parse and JSON Schema (`schema_version`, enums, required/unknown fields, bounded sizes).
2. Ensure all generated IDs are unique within their declared scope and every reference resolves (`participant`, `finding`, `previous_finding`, `thread`).
3. Load immutable `context_id` and verify PR/review revision, original description and changed-file index.
4. Validate all `SourceRef`s against exact Git commit tree and file length, including base/head semantics.
5. Validate diagram syntax and nesting limits, ordered step sequence, positive branch counts and code-backed steps.
6. Validate finding evidence, exactly one primary location, alternative suggestion variants, replacement targets and diff anchor mapping.
7. Validate original PR description quotes and repeated-quote occurrences, related threads, follow-up status/current-finding consistency, and coverage check enums.
8. Only after full success permit `commit` to check optimistic concurrency and atomically persist the result.

Validation should be deterministic and must never call an AI model. Behavioral truth (real bug, correct fix, accurate severity, accurate execution flow, semantic duplication) remains outside deterministic validation and is governed by skill methodology and human inspection.

## 6. Local SQLite state

### 6.1 Storage objectives

- One current review result per PR identity; **no full historical report archive**.
- Drafts belong to the reviewer and survive app restarts and CLI validation failures.
- Published GitHub comments/threads are remotely authoritative; local state is a cache and operation ledger.
- Explicit revisions prevent concurrent agent run from overwriting GUI edits.
- Schema migrations are versioned, transactional, tested on real persisted fixtures and recoverable on failure.

### 6.2 Logical tables (names illustrative)

| Entity | Core fields |
|---|---|
| `repositories` | repo_id, GitHub host/owner/name, local checkout path(s), last_opened_at |
| `pull_requests` | repo_id, number, title/author cached, latest_head_sha, remote_checked_at |
| `workspaces` | pr_id, pinned_context_id, current_result_id, workspace_revision, draft_revision, selected_tab, selected_flow/file/scroll snapshots |
| `contexts` | context_id, base_sha, head_sha, merge_base_sha, diff metadata/hash, original_description snapshot |
| `current_results` | pr_id, context_id, schema_version, complete validated result JSON/blob, created_at |
| `drafts` | id, pr_id, type, body/replacement, suggested_source, current_anchor, status, created/modified_at, origin finding/alternative |
| `draft_thread_actions` | thread node ID, target state, status, last_seen remote state |
| `publication_runs` | id, pr_id, chosen review event/body, pinned head, state, confirmed_at, last_reconciled_at |
| `publication_operations` | id, run_id, type, payload fingerprint, remote ID, status, attempt details, last_error |
| `repo_cache` | object/diff cache metadata, expiry/size, context references |
| `replacement_authorizations` | approval ID, pr_id, context_id, draft/workspace revision, expiry, consumed flag |

Use **referential integrity**, SQLite WAL when compatible, transaction boundaries, durable save-before-send for publication operations, and a clear policy for deleting expired cache. Avoid copying source files into DB when they can be read from Git; retain only required metadata or bounded caches. Sensitive information and tokens never belong in result data, logs, or SQLite.

### 6.3 Review replacement transaction

Under an exclusive/serialized workspace update:

1. Recheck authorized context, current result revision, drafts and expected workspace version.
2. Insert/replace the complete current generated result and current pinned context.
3. Derive compact previous/current finding relationship information needed for the visible follow-up summary.
4. Clear only appropriately authorized unpublished drafts when a destructive replacement was confirmed.
5. Increment workspace/draft revision and mark replacement authorization consumed.
6. Commit all together, or roll back **all**.

No half-updated Overview vs Findings vs diagram state. GUI reloads the new complete version using the same local store; if it has in-memory edits, its save must fail safely on stale revision rather than overwrite.

## 7. Native application UX and presentation specification

**Design authority and lifecycle:** The application-facing layout specification here supersedes the previous prototype screens and applies to eventual implementation and verification; the current design-only task restriction is held in PROJECT_STATUS.md, not in this technical specification. See [FigCraft design brief](design/FIGCRAFT_BRIEF.md), [design quality contract](design/DESIGN_QUALITY_CONTRACT.md), and [design references](design/DESIGN_REFERENCES.md). Designs are approved individually before desktop implementation. **Rune is a candidate, not yet a frozen dependency**; it must not dictate the design prematurely.

### 7.1 Screens, navigation, and state preservation

**Home:** deduplicated recent PR workspaces and registered local repositories; readable repo identities first, paths/status diagnostics secondary. No analytics or giant tiled cards.

**Repository PR browser:** dense searchable/filterable PR list with title/number/author/lifecycle, saved-review indicator, clear loading/error/no-auth states. Open manual review without AI.

**PR workspace:** compact header and tabs `Overview | Changes | Findings`; one prominent **Finish Review** entry point opens a right drawer; lifecycle **Merge/Close** actions live separately. Persist current tab, selected changed file/finding/diagram step, code-scroll offsets, expanded thread state where practical, resizable pane widths, and unpublished drafts. No ordinary comment action opens a separate route.

**Interaction/state invariant:** switching views, staging/canceling comments and closing a contextual drawer must not erase drafts or jump the reviewer to the top of code. Stale review data remains pinned until explicit user refresh.

### 7.2 Overview: source-linked understanding

1. Compact PR context (title, number, author/branches, freshness state).
2. Deterministic short excerpt from the original PR description; **View full description** opens safe Markdown in a resizable contextual reading pane, with its own restored scroll position. Do not AI-summarize the excerpt.
3. Zero or more focused diagrams, one at a time; compact selector for multiple. An interaction click opens/updates the adjacent committed-code inspector (syntax highlighting, line numbers, related finding links and View in Changes) without leaving Overview.
4. Neutral/monochromatic diagram; single accent for messages directly grounded in changed code, with an independent selection cue; only single-level `alt`/`opt`, no nested conditionals. **Changed classification is derived from validated source/diff intersections, not agent style fields**.
5. Concise agent-generated implementation summary and curated key changes; significant source-grounded intent discrepancies/uncertainties only. When no AI result exists, omit generated sections and allow manual review.

**Renderer design constraint:** Preserve application-owned structured diagram input and strict validation. A library such as `rune-mermaid` may be used as an implementation detail after checking deterministic layout, hit-target mapping and semantic color rules; **never let the agent submit freeform Mermaid** or bypass source validation.

### 7.3 Changes: GitHub-style inline reviewing

- **Layout:** compact changed-file navigator plus primary code/diff viewport using most available width. Resizable/collapsible navigation; real unified/split diff, line numbers, changed-line semantic backgrounds, file headers, folding and large-diff virtualization.
- **Inline comment initiation:** gutter action on an eligible line or explicit multi-line selection. A composer appears **immediately beneath that exact anchored line/range**, not in a detached pinned-source form. Comment text Markdown and optional GitHub-compatible replacement code are editable before staging. Text/code previews, add-to-review and cancel happen in place.
- **Existing GitHub discussions:** render thread history and replies at their actual diff anchors (including clearly indicated outdated anchors), with local reply and resolve/reopen drafting in context. Published state and local staged state must never be visually conflated.
- **Finding references:** subtle markers link changed code to related AI findings; selecting one may navigate to Findings while preserving the Changes scroll and selected file for return.
- **Persistence/flow:** stage/unstage/cancel/expand discussion without forcing navigation or resetting file/range/scroll. Avoid full-page forms and always-visible comment panels. No source-file or worktree editing in V1.
- **Input and a11y:** keyboard-focusable line/comment navigation, range selection, text editing, visible focus, copy/paste, long-line scroll, and adequate hit targets. Invalid GitHub anchors receive actionable explanations instead of silently sending bad payloads.

### 7.4 Findings: master-detail evidence review

- **Left pane:** resizable, vertically scrollable list of compact findings; each shows **severity, title, relevant file path(s)** and a small action state as useful. Order actionable findings by severity; resolved follow-ups collapsed outside the default queue.
- **Right pane:** on selection, show the finding's **actual pinned-commit code block**, syntax highlighting and line numbers, relevant code-range emphasis, concise problem/explanation/impact/evidence, optional supporting source switcher and related GitHub discussion.
- **AI suggestions:** text comments and code replacement alternatives **directly below the code** in the right pane; one selected/staged suggestion per finding by default. A selected suggestion opens an editable contextual composer; switched alternatives must preserve or explicitly warn about edited/staged drafts.
- **Actions:** stage, unstage, dismiss/defer, create independent manual comment, and View in Changes. Avoid additional navigation merely to understand the current finding.
- **Compact extras:** new/unresolved/regressed count, expandable resolved findings and review coverage/limitations; no separate Coverage or Discussions tab. Honest no-AI/manual-only empty state, not an error.

### 7.5 Finish Review: context-preserving right drawer

- Openable from every workspace tab, with the underlying code still visible; not a full-screen review form.
- Compact scrollable publication preview: new inline comments/replacements, existing-thread replies, resolve/reopen actions, local summary and decision selection. Allow editing and navigation to an anchored item and back without losing state.
- Explicit event choice (`COMMENT`, `REQUEST_CHANGES`, `APPROVE`), validation of source/head/permission state, and final user confirmation; never silently default to approval. Merge/Close remain separate actions.
- Show specific partial-success/unknown remote publication outcomes and retain pending work; closing the drawer without publishing is non-destructive.

### 7.6 Design system, geometry and review-flow verification

- **FigCraft first:** Create a reusable semantic token system (colors, typography, spacing, radii, component states) and a **geometry specimen board** before any application screen. Prefer auto-layout/constraints, not hand-positioned repeated elements.
- **Precision:** center labels and icons within real hit regions, align repeated rows and baselines, use predictable padding/gaps and consistent code gutter. If FigCraft exposes numeric bounds, target 1 logical pixel consistency of repeated anchors; if it does not, report visual-only checks honestly.
- **Appearance:** dark-first with a separately verified light mode; muted surfaces, sparse dividers, calm code-first layout, limited color with semantic diff/severity exceptions. No gratuitous gradients, stacked bordered cards, oversized empty forms, or unreadably small code text.
- **Responsive states:** check 1280×800, 1440×900, 1920×1080, long paths, large finding lists, narrow windows, toolbar overflow, keyboard focus, reduced motion and popup/inline-composer positioning.
- **Flow-state rehearsal:** complete a manual diff-line → inline composer → stage → same scroll sequence, followed by Findings list → source → suggestion → stage → View in Changes → return → Finish Review drawer → back without context/draft loss.
- **Approval gates:** Gate 0 discovery; Gate 1 design system/specimen; Gate 2 Findings; Gate 3 Changes; Gate 4 Finish Review; Gate 5 Overview; Gate 6 Home/repository browser; Gate 7 cross-screen audit. Require a real FigCraft export, verified geometry/visual QA report and **explicit user approval** before moving on.
- **No implementation during design:** Approved FigCraft artifacts are handoff data, not authorization to start the application. Implement only after a separate decision and framework viability check.

## 8. Freshness, anchors and publication

### 8.1 Stale review handling

GUI may poll for latest `head_sha` while in active use. On mismatch, show a **nonintrusive stale review badge/message**; do not replace pinned Overview/Changes/Findings, diagrams, inspector source or drafts. The user may explicitly load newer manual code with protected draft-discard confirmation; AI follow-up requires an external skill invocation.

Do not automatically mark old findings as still true, fixed or resolved just because the remote head moved. Show them as associated with the previous reviewed revision.

### 8.2 Inline anchor resolution

`SourceRef` is a code reading locator; `DiffAnchor` is a publication locator. Before draft staging and again immediately before sending:

1. Check that PR is eligible for comment/review and remote current head and target state are known.
2. Map the draft target to correct changed file/hunk, GitHub `RIGHT` (new/head) or `LEFT` (old/base) side, and inclusive line/span; identify rename and removal correctly.
3. For code replacements, require an eligible head-side range and verify exact original text/span and GitHub suggestion compatibility. Construct fenced `suggestion` Markdown in app using validated replacement code.
4. If new commits moved an **unchanged exact code span**, the app may propose an unambiguous deterministic reanchor; **show reviewer the new location and require confirmation**. Never change an anchor silently on materially changed content.
5. If changed/removed/ambiguous, label `REVALIDATION_REQUIRED`, offer open latest code/reselect target, convert text to general comment where appropriate, or discard/defer. Do not send against an invalid location.
6. Recheck as close as possible to send, including after any user editing. No client can make this check fully atomic with remote changes; handle GitHub's rejection gracefully.

Do not use legacy diff `position` as the preferred comment coordinate: prefer REST `line`/`side` and `start_line`/`start_side`, supported by GitHub's current API. A GitHub suggestion may not be valid on every range; validation must be stricter than basic SourceRef validity.

### 8.3 Remote publication plan

After an explicit human confirmation, build a durable **operation journal** before sending requests:

| Operation | GitHub mechanism | Recovery implication |
|---|---|---|
| Review event + new inline comments | `POST /repos/{owner}/{repo}/pulls/{pull_number}/reviews` via `gh api`, with explicit event, summary, commit ID, comments | Usually one request, not a distributed transaction |
| Reply to existing review discussion | `POST /repos/{owner}/{repo}/pulls/{pull_number}/comments/{comment_id}/replies` | Top-level comment ID required; replies to replies not directly supported |
| Resolve thread | GraphQL `resolveReviewThread(input: {threadId})` | Recheck thread and viewer permission |
| Reopen thread | GraphQL `unresolveReviewThread(input: {threadId})` | Recheck current state and permission |

Payloads passed through `gh api --input -` (stdin), not unsafe shell string construction. `gh` handles auth; no token stored by Bodh. Publish outcome is **not atomic** across multiple operations. Order operations deliberately (e.g., comments and review decision first, then thread replies/actions), with UI confirmation of what will happen. If review creation is rejected or unknown, do not blindly proceed as though it succeeded.

**Operation status**: `planned → sending → confirmed | failed | unknown`. Save operation intent and fingerprint before sending, record returned IDs/responses on acknowledgment. On crash/network timeout, reconcile uncertain operations against GitHub by user/account, body/location, review IDs and time window. If uniqueness cannot be proven, ask user to inspect remote status before retry. Never automatically repeat a potentially successful write; never claim exactly-once guarantees from GitHub.

**Post-publication:** only confirmed operations become published locally; retain failed/unknown drafts for retry/reconciliation. Update thread remote state and review counts; show partial-success details and direct links to confirmed GitHub objects where available. Prevent double click during in-flight send.

### 8.4 PR lifecycle actions

Merge and Close, if exposed in V1, are explicitly separate actions in a PR action menu, with permissions, current SHA/lifecycle recheck, clear consequences, and **dedicated confirmation**. They never hitchhike on Finish Review or run from AI output. Repository branch protection and server denial are authoritative. Never provide an implicit "auto merge after approve" workflow.

## 9. `pr-review` skill integration

### 9.1 Skill responsibilities

- Determine PR target and call `bodh review prepare` from matching local checkout.
- Respect draft authorization gate and prepared context revision.
- Read relevant diff and unchanged caller/callee/test context as needed using CLI inspect and read-only source tools.
- Use evidence-driven passes (correctness, security, regression, reliability, tests, project conventions) without mandatory decorative findings.
- Build a single complete `schema_version:1.0` result. Only semantic content, within supported diagram/overview/finding structures.
- Invoke `bodh review validate`, correct specific schema/source errors within bounded attempts, then `bodh review commit`.
- Report review completion, limitations, and optional `bodh review open --pr ...`; no unsolicited GitHub review publication.

### 9.2 Explicitly removed from old skill

No custom HTML/CSS/JS report output, Python action server, generated Mermaid/SVG, direct `gh` publish actions, GUI interaction automation, or extra server lifecycle. The skill can leverage its existing review methodology but must defer CLI/application interface rules to Bodh's versioned schema and actual `bodh contract describe` output.

### 9.3 Agent review discipline

- Cite precise pinned commit source refs; do not infer from dirty local branch.
- Distinguish direct code change from necessary unchanged context.
- Diagrams represent only evidence-backed meaningful execution flow; omit irrelevant steps.
- For developer intent, extract exact claims from the original description rather than summarize and compare entire documents.
- For existing discussions, don't duplicate active feedback unless new evidence or a distinct aspect justifies it.
- Never invent tests run or claim unverified fixes.
- New agent result is not allowed to publish, dismiss human drafts, merge or close PRs without dedicated user actions.

## 10. Security, reliability and privacy

- Process command arguments are constructed as argv arrays; paths, labels, agent text and PR Markdown are untrusted. No command interpolation or arbitrary script execution.
- Constrain Git repository paths to the selected checkout and commit tree; guard against `../`, absolute paths, symlinks/potential path confusion and binary/file size abuse.
- Sanitize Markdown and external links; avoid executing embedded HTML, JavaScript, terminal escape sequences, images/scripts in arbitrary repository content.
- Never log raw access tokens, secrets observed in code, or unnecessarily store full sensitive payloads in diagnostics; `gh` manages GitHub credentials.
- No telemetry/hosted sync/backend is required for V1. App must not incorrectly promise that external coding agents are always local/private.
- Use DB transactions and explicit failure states. Crash recovery must preserve prior result and pending drafts except where a successful authorized replacement transaction is durably committed.
- Retain bounded cache; error clearly when the needed Git object/remote data is unavailable rather than falling back silently to worktree content.
- Rate-limit/paginate `gh` calls and show offline/network/error states; avoid excessive remote polling.

## 11. Design approval, framework feasibility and implementation sequence

**Gate A — FigCraft design and UX approval:** Use FigCraft to complete the approval-gated design system, Findings, Changes, Finish Review, Overview, Home/repository browser and cross-screen UX audit. Deliver editable design source when supported, theme/token maps, component specs, interaction states, exports and geometry/flow QA evidence. **Do not write Rust/Rune application code during this gate.**

**Gate B — Rune feasibility (separate authorization):** With a pinned Rune `dev` commit, prototype **only** an isolated Linux/Wayland slice: `rune-kit::diff` unified/split, clickable diff line/range, line-anchored `Annotation` and inline composer, text editing/IME/clipboard, long-diff virtualization, resizable Findings master-detail, and a diagram step → source inspector. Test on actual Hyprland/CachyOS where available. Assess accessibility, stable keyboard focus, native selection, GPU adapter behavior and focused-input idle CPU. Verify `rune-mermaid` hit-target/source-mapping feasibility, and whether Rune-kit highlighting meets the language requirements; its lenient Mermaid parser is **not** the input contract. Record pinned commit and blockers. **Passing this gate, not excitement about components, selects Rune.** If it fails, evaluate an alternative without weakening approved UX.

**Gate C — Rust domain/core:** Implement canonical PR identities, read-only Git/`gh` adapters, pinned merge-base diffs, SQLite draft/history semantics, strict schemas and valid/invalid fixture tests. CLI remains usable headlessly with no GUI or daemon.

**Gate D — First complete manual review:** Implement approved native shell + repository browser + Changes inline comments/threads + staging + Finish Review drawer and remote publication recovery. Test actual GH review events and invalidated anchors. Manual review must work without AI.

**Gate E — Agent-assisted review:** Implement Overview diagrams/intent alignment and Findings master-detail with alternatives, follow-up and coverage, then migrate `pr-review` skill to strict dynamic contracts and CLI. Do not import old HTML/UI code or snapshot archive.

**Gate F — Full qualification:** Verify PRD acceptance criteria, visual regression/screenshot checks against FigCraft exports, Linux stability, accessibility/keyboard navigation, large PRs, concurrent agent/GUI edits, no worktree mutation, offline/failure behavior and safe partial GitHub publication.

At every gate, change contract/schema semantics **before** coordinated implementation across the Rust core, CLI, app, skill, fixtures and docs. No screen may redefine the contract locally.

## 12. Test and acceptance matrix

| Area | Essential test fixtures |
|---|---|
| PR identity | Same PR from different local branches; fork PR; GitHub Enterprise host; malformed/ambiguous remote |
| Git comparison | New base commit, new head commit, independent branches, renamed/deleted files, binary/submodule, CRLF, dirty worktree |
| Context | Identical inputs yield same context ID; latest remote SHA mismatch; missing commit object/fetch failure |
| Schema | All required fields; unknown field rejection; enums; duplicate IDs; unresolved cross-refs; version incompatibility |
| Source | Valid old/new lines, line beyond EOF, wrong revision, path traversal, renamed/deleted paths, nonexistent commit blob |
| Diagram | Message/return/self-call, simple `alt`/`opt`, attempted nested condition rejection, empty diagrams, hit testing, changed-only accent |
| Overview | Exact quoted original claim with occurrence, repeated text, invalid quote; no discrepancy case; malicious Markdown |
| Finding | Evidence and location validation, one primary ref, multiple alternatives, one default draft, code replacement range and text |
| Follow-up | New/resolved/unresolved/regressed/uncertain, previous/current finding IDs, compact preservation without history growth |
| Coverage | Excluded areas, checks not run/unavailable, no fabricated percentages |
| Concurrency | Agent validates while user edits draft; stale authorization; GUI and CLI simultaneous DB writes; interrupted replacement |
| Freshness | Remote new commit while GUI open, no automatic replacement, stale comment reattachment/missing span |
| GitHub | Review event + multiple comments, reply to top-level, resolve/reopen permissions, partial failure, unknown timeout, duplicate avoidance |
| UI | No-AI manual review; exact line/range inline composers; Findings left-list/right-code-and-suggestions; Finish Review drawer; tab/scroll restoration; dark/light/geometry QA |
| Safety | No worktree checkout/reset/write; argument injection; untrusted Markdown; no agent-initiated remote mutation |
| Design QA | Approved FigCraft exports vs rendered UI; geometry baseline and button centering; 3 viewport sizes; populated interaction-state coverage; reduced motion |
| Rune feasibility | Pinned dev commit, Hyprland/Wayland focus/IME idle CPU, diff annotations, large diffs, clipboard, multiline selection, diagram/source navigation |

**Release gates:** all PRD acceptance criteria AC-01 through AC-16 pass; canonical schema/fixtures/CLI/skill agree; complete manual review works without AI; no daemon; GitHub publication recovery is tested; pinned-source integrity confirmed. Set quantitative latency/memory/diff-size targets after observing representative Linux hardware and large PR fixtures rather than inventing numbers.

## 13. Implementation details not yet frozen

These are implementation selections, not invitations to change approved UX:

1. Actual native UI framework: Rune + rune-kit preferred **conditionally** after an isolated Linux/Wayland proof of concept; fallback evaluated only if the prototype fails. Freeze dependency to a tested commit; track upstream API changes.
2. Rune diff annotations and source anchors, editable multiline comment UI, thread rendering/virtualization, focus/IME and clipboard behavior; any custom wrappers must preserve application-owned GitHub anchor validation.
3. Native diagram rendering approach: controlled `rune-mermaid` subset vs custom Rust layout/hit-testing, maintaining strict schema and neutral + one-accent visual semantics.
4. Syntax highlighting strategy: built-in Rune-kit syntax vs Tree-sitter adapter; no LSP prerequisite.
5. Exact CLI argument spellings, JSON errors, contract JSON Schema organization/count limits, and non-TTY authorization mechanism; freeze via fixtures.
6. Cache/object availability for forks and hosts, large diff virtualization, poll interval, focused-input idle CPU and Linux packaging/support matrix.
7. GitHub API version compatibility, safe comment-anchor reattachment and indeterminate-publication reconciliation; no silent relocation or blind duplicate writes.

**Contract-first rule:** For any new behavior or mismatch, revise canonical schema + semantic specification + fixtures first; then implement Rust core/CLI, native app, skill, tests, migration notes and user documentation together. Reject unsupported data rather than silently widening the contract.

## 14. Authoritative references

- GitHub REST API: [Pull request reviews](https://docs.github.com/en/rest/pulls/reviews) — events, commit ID and grouped review comments.
- GitHub REST API: [Pull request review comments](https://docs.github.com/en/rest/pulls/comments) — `line`/`side`/multiline positions and reply endpoint.
- GitHub GraphQL: [Pull requests, review threads and resolve/unresolve mutations](https://docs.github.com/en/graphql/reference/pulls).
- GitHub CLI: [`gh api`](https://cli.github.com/manual/gh_api) — REST/GraphQL authenticated requests and stdin JSON input.
- GitHub UX: [Review pull requests quickstart](https://docs.github.com/en/pull-requests/get-started/reviewing-pull-requests-quickstart) — code suggestions and submission.
- Existing methodology: [`kedarmd/skills/pr-review`](https://github.com/kedarmd/skills/tree/main/pr-review); methodology source, not future UI/action-server implementation authority.

---

**Definition of V1 success:** A developer opens Bodh on Linux, thoroughly reviews a GitHub PR manually if they choose, optionally imports a structured analysis from their existing coding agent, understands the implementation via source-linked flows, selects only useful findings, and publishes intentional GitHub feedback safely—without leaving a daemon running or handing UI decisions to AI.
