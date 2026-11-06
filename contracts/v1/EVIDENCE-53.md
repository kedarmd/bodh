# Issue #53 — retry definitively failed discussion operations safely

## Behavior

- `DiscussionPublicationStore::retry_failed_plan` creates a new immutable plan from only failed and eligible unsent steps. Confirmed and already-achieved steps remain in the original history and are not copied into the retry; unknown or in-flight steps block retry until reconciliation.
- Before creating a retry, Bodh checks the exact saved reply/action draft revisions and payloads, current PR open/head/base state, current thread permissions/state, and any grouped-review dependency. Normal publication repeats the remote checks before each write.
- Retry plans preserve operation ordering and the grouped-review dependency. A unique `retry_of` relationship prevents duplicate direct retries while allowing a later failed retry to be retried as its own operation.
- Finish Review exposes an explicit retry action only for settled plans with failed/unsent work, no unresolved publication, and no existing retry child. The saved history is refreshed whether publication succeeds or returns an error after journaling.
- A plan with confirmed work plus unsent or already-achieved work remains `partial`; it is not reported as fully confirmed.
- On reopen, legacy plan rows previously mislabeled `confirmed` despite failed, unsent, or already-achieved steps are corrected to `partial` before the retry UI reads them.

## Controlled tests

`tests/discussion_publication.rs` uses temporary SQLite state and controlled publishers to cover partial success with a failed action, failed replies across restart, resolve/reopen retries, unsent-only retry with confirmed work excluded, unknown and stale head/permission/draft rejection, duplicate retry prevention, retained exact payload, prior outcome history, and correction of a legacy plan summary on reopen. No GitHub write is made.

The native `publication_recovery_ui_tests::discussion_retry_requires_settled_eligible_work_and_no_existing_retry` test covers settled failed/partial eligibility and suppression while loading, with unknown work, or after a retry child exists.

## Qualification boundary

No live GitHub writes or native GUI/pointer/keyboard trial were performed. The retry transport is exercised through controlled publishers; the button is covered by native state-helper tests only.

## Executed validation

- `CARGO_BUILD_JOBS=2 cargo test --locked --test discussion_publication` — 15 passed.
- `CARGO_BUILD_JOBS=2 cargo test --locked --manifest-path spikes/native-sequence/Cargo.toml --bin bodh-sequence-spike publication_recovery_ui_tests::discussion_retry_requires_settled_eligible_work_and_no_existing_retry` — 1 passed.
- `CARGO_BUILD_JOBS=2 ./scripts/validate desktop` — passed, including root/spike Clippy and full tests. Output is preserved in [VALIDATE-53-final.log](VALIDATE-53-final.log). It ran on revision `53e2977` with the #53 changes in the dirty working tree.
- The first integrated validation attempt stopped at Clippy on a complex stored-plan tuple; it was replaced by a named record before the passing rerun. The initial diagnostic is preserved in [VALIDATE-53-initial-clippy-failure.log](VALIDATE-53-initial-clippy-failure.log).
- Both Cargo manifests passed `cargo fmt`; `git diff --check` passed.
