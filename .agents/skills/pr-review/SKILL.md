---
name: pr-review
description: >
  Review a GitHub pull request with Bodh's pinned, headless review workflow.
  Use when asked to review a PR in a checkout Bodh can prepare. Preserve the
  existing correctness, security, regression, reliability, testing, and project
  convention review passes while returning a validated Bodh result.
---

# PR review through Bodh

Use Bodh's shared contract and local workspace as the authority for preparation, evidence, validation, and persistence. Keep the desktop closed during analysis. The supported scope is a GitHub pull request with an associated local checkout and authenticated `gh`; commit ranges and unpushed changes are not Bodh review inputs.

## 1. Discover and prepare

Run `bodh --json contract describe` and `bodh --json contract schema`. Read the returned protocol version, supported schema version, limits, and commands before producing a result. Stop if the installed Bodh CLI does not advertise complete prepared-review validation.

Choose the same state database used by Bodh, normally `${XDG_STATE_HOME:-$HOME/.local/state}/bodh/reviews.sqlite`, and pass it explicitly to every command. Prepare the requested PR:

```sh
bodh --json prepare --repo /absolute/path/to/checkout --pr 123 --state "$STATE"
bodh --json inspect --prepared "$PREPARATION" --kind manifest --state "$STATE"
```

Treat the returned preparation ID, context ID, pinned SHAs, and expected generation as a single snapshot. Read `draft_counts`, `previous_result_available`, and discussion completeness from the manifest. Do not substitute another preparation or the working tree if the pinned read fails.

## 2. Inspect the pinned review inputs

Use on-demand `inspect` calls for the original description, changed files, each relevant file's diff, discussions, and prior result. Follow file and diff cursors until the response says the collection is complete. If a discussion connection is incomplete, retain the observed threads and describe the missing connection; never claim full coverage.

Use `inspect --kind source` with the exact path, `head` or `base` revision, and inclusive line range for every source citation. Base source means the prepared merge base. Read surrounding pinned context where it affects behavior. Never cite dirty worktree text as PR evidence.

## 3. Analyze and construct one result

Review correctness, security, regressions, reliability, tests, and project conventions. Trace changed code into relevant callers and tests. Use the original description, acquired discussions, and prior concerns as evidence where available. Avoid repeating active discussion feedback unless the new finding explains a distinct risk.

Produce one schema-complete v1 result. Empty `findings`, `diagrams`, or `follow_up` arrays are valid when evidence supports them. Include a diagram only when it explains a meaningful source-backed interaction. Link every claim to exact pinned source. Assess prior concerns by their acquired IDs; unresolved and regressed concerns need a current actionable finding, while a resolved status needs evidence of the fix. Keep uncertainty explicit. Record checks as `not_run` or `unavailable` unless they were actually observed.

Write the candidate JSON to a file so the same bytes can be validated, authorized, and committed. Do not run commands that modify working files or the index, publish review comments, approve/request changes, resolve threads, merge, close, or start a server. Bodh preparation may fetch objects and update its retained Git refs while leaving checkout contents and index unchanged.

## 4. Validate with a finite correction bound

Validate against the exact preparation and generation from the manifest:

```sh
bodh --json validate --prepared "$PREPARATION" --input "$CANDIDATE" \
  --state "$STATE" --expected-generation "$GENERATION"
```

Allow at most **three correction rounds** after the initial validation. Each round may address only reported schema, evidence, anchor, discussion, or prior-concern errors. Revalidate the complete candidate after every correction. Do not loosen the contract, invent source, or keep retrying after the third correction. On exhaustion, report the error code, pointer, and the evidence needed to continue.

Stop and report actionable errors for unavailable pinned objects, incomplete evidence that prevents a claim, stale preparation/context, stale generation, or a missing/expired approval. A stale result is not permission to silently prepare again and reuse the old analysis.

## 5. Commit safely

If the manifest reports unpublished drafts or staged reviewer intent, explain that replacing the current generated review would clear the exact work shown in the authorization preview. An unattended run stops without replacement. With the reviewer present, run Bodh's interactive authorization against the saved candidate and exact preparation/generation:

```sh
bodh --json authorize-replacement --prepared "$PREPARATION" \
  --input "$CANDIDATE" --state "$STATE" \
  --expected-generation "$GENERATION"
```

The reviewer must type the PR identity shown by Bodh in the interactive terminal. Keep the returned one-use approval ID private to this operation. For a candidate with no protected work, commit those exact bytes and generation:

```sh
bodh --json commit --prepared "$PREPARATION" --input "$CANDIDATE" \
  --state "$STATE" --expected-generation "$GENERATION"
```

For a protected-work replacement, use the same command with the freshly returned approval ID:

```sh
bodh --json commit --prepared "$PREPARATION" --input "$CANDIDATE" \
  --state "$STATE" --expected-generation "$GENERATION" \
  --approval "$APPROVAL_ID"
```

A repeated identical commit is an idempotent no-op. If commit reports a stale generation, changed context, or stale approval, stop; do not discard drafts, obtain a broader grant, or retry against a different snapshot automatically. Failed analysis or validation leaves the current result and drafts intact.

## 6. Report

Report the PR identity and pinned head, whether the result was committed or unchanged, finding/diagram/follow-up counts, checks actually run, incomplete observations, and remaining limitations. Say when no actionable findings were supported. Provide a desktop-opening instruction only when the installed Bodh build advertises one; this workflow does not require a GUI or persistent process.
