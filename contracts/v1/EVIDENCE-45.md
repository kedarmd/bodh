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
