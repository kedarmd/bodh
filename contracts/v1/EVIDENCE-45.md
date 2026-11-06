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

At revision `codex/issue-45-failure-matrix` (`220f076` plus these changes), `cargo test --locked --test freshness` passes 11 tests (3 newly added). The complete `CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/home/kedarmd/dev/bodh/target ./scripts/validate desktop` run passes; output is preserved in [`VALIDATE-45-final.log`](VALIDATE-45-final.log). An earlier run first exhausted the isolated worktree `/tmp` target space; after removing generated target artifacts and using the shared `/home` target, validation passed. A successful approved load-latest switch now verifies that text comments, code replacements, replies, thread actions, finding feedback/disposition and edited alternatives remain attached to the old preparation; the returned new preparation has empty draft/triage views. The protection snapshot is byte-for-byte equal before and after switching. A concurrent text draft created after approval causes `DRAFT_REVISION_STALE`, while both old and new preparation bindings remain loadable and the edited draft remains visible only in the old view. Missing-CLI acquisition failure and a pre-cancelled acquisition both leave the current binding and protected-work count unchanged.

These tests exercise real SQLite stores and switch APIs, not the desktop view widget or a full actual CLI invocation. There is still no native failure/cancellation/permission-loss trial; the injected missing CLI represents acquisition failure, not a live GitHub permission response. No real Git checkout/index snapshot is attached to this store-level fixture. The original acceptance request for end-to-end CLI view consistency and unchanged working tree/index through remote acquisition therefore remains only partially verified.
