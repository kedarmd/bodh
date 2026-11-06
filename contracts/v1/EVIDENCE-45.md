# Issue #45 acceptance evidence

Implementation: the existing latest-context flow acquires a separate preparation before a switch can be authorized. `apply_latest_switch_approval` rechecks the approval binding, workspace revision, and protected-work fingerprint inside an immediate SQLite transaction, consumes the approval once, then returns the candidate binding. The desktop retains its current selected binding until this operation succeeds.

`protected_work_in` fingerprints unpublished inline comments, replacements, replies, thread actions, review intent, all non-confirmed review publication payloads, and non-confirmed discussion publication plans with their complete step payloads and states. Finding triage disposition, selected suggestion, manual feedback, revision, and alternative-edit metadata are now included as well. Failed and partially completed publication records remain protected because they can still contain retained/retryable work.

## Validation

- `cargo fmt --check` — passed.
- `CARGO_BUILD_JOBS=2 cargo test --locked --test freshness` — passed (8 tests, rerun after merging integration tip `70c0e0a`).
- The new finding-feedback test was first run against the old fingerprint logic and failed because the before/after fingerprint was identical; it passes with triage included.
- SQLite regression coverage confirms a failed discussion publication's count and payload changes affect the protection fingerprint.
- These tests use local SQLite and deterministic fixtures; they do not write to GitHub or alter a checkout/index.

## Remaining acceptance gaps

- No new CLI test exercises every draft category through actual acquisition and view switching. The tests added here cover the public protection/approval store boundary, while existing UI flow retains the old selection until successful completion.
- Failure, cancellation, permission-loss, and old/new displayed-view behavior were not exercised in a native GUI trial during this change.
- Expiration is implemented as a five-minute check, but the test suite does not wait for the TTL to elapse.

## Additional SQLite regression coverage

At revision `codex/issue-45-failure-matrix` (`220f076` plus these changes), `cargo test --locked --test freshness` passes 11 tests (3 newly added). The complete `CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/home/kedarmd/dev/bodh/target ./scripts/validate desktop` run passes; output is preserved in [`VALIDATE-45-final.log`](VALIDATE-45-final.log), and the follow-up run after these additional assertions is preserved in [`VALIDATE-45-followup.log`](VALIDATE-45-followup.log). An earlier run first exhausted the isolated worktree `/tmp` target space; after removing generated target artifacts and using the shared `/home` target, validation passed. A successful approved load-latest switch now verifies that text comments, code replacements, replies, thread actions, finding feedback/disposition and edited alternatives remain attached to the old preparation; the returned new preparation has empty draft/triage views. The old view assertions now include all six draft classes: text comments, code replacements (including original and replacement payload), replies, thread actions, finding feedback/disposition, and edited alternatives. The new preparation view is empty for each class. The protection snapshot is byte-for-byte equal before and after switching. A concurrent text draft created after approval causes `DRAFT_REVISION_STALE`, while both old and new preparation bindings remain loadable and the edited draft remains visible only in the old view. Missing-CLI acquisition failure and a pre-cancelled acquisition with a pre-existing text draft both leave the current binding and complete protected-work snapshot unchanged; the old view still returns the exact draft body.

These tests exercise real SQLite stores and switch APIs, not the desktop view widget or a full actual CLI invocation. There is still no native failure/cancellation/permission-loss trial. No actual CLI view-switch operation exists. The original acceptance request for CLI-level old/new view consistency therefore remains unverified.

## Follow-up: controlled permission-loss observation

The failed/cancelled acquisition fixture also invokes both latest-candidate acquisition and the public freshness service with a controlled `gh` executable that returns HTTP 403. A real temporary checkout with the matching GitHub remote ensures candidate acquisition reaches the denied metadata request; the call returns `AUTH_ACCESS` while the selected preparation, protected-work fingerprint and populated text draft remain unchanged. Freshness records `Unknown` with `AUTH_ACCESS`. This is a deterministic process fixture, not a live permission race. At this point the checkout/index snapshot had not yet been added; see the later checkout-integrity section.

- `CARGO_BUILD_JOBS=2 cargo test --locked --test freshness` — 11 passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.

The first desktop validation rerun failed in the existing `gh_prerequisite::tests::preflight_reports_the_installed_cli_version` test because its temporary fake executable could not be started; the immediate complete rerun passed. The failure was transient and did not involve the new freshness test. The final passing output is in `VALIDATE-45-permission-loss-acquisition.log`.

## Checkout and index integrity

The `failed_or_cancelled_latest_acquisition_keeps_the_current_binding_selected` fixture now uses a real Git checkout with a committed HEAD, a symbolic `main` branch, an unstaged tracked-file edit, and an untracked file. It captures and compares the raw `.git/index` bytes, HEAD, symbolic ref, tracked file bytes, and untracked file bytes across missing-CLI acquisition failure, controlled HTTP 403 during candidate acquisition, freshness permission denial, and pre-cancelled candidate acquisition. The raw index is verified unchanged, but this fixture does not contain a staged modification. The focused regression passed:

```sh
CARGO_BUILD_JOBS=2 cargo test --locked --test freshness failed_or_cancelled_latest_acquisition_keeps_the_current_binding_selected -- --exact
```

Result: 1 passed, 0 failed. This establishes checkout/index preservation for those acquisition/service outcomes. It does not establish CLI old/new view consistency or native UI behavior.

The complete `CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/home/kedarmd/dev/bodh/target ./scripts/validate desktop` runner passed after this addition. Its full output is preserved in [`VALIDATE-45-checkout-snapshot.log`](VALIDATE-45-checkout-snapshot.log).
