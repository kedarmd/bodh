# Issue #46 — retained review data bounds

## Policy and behavior

The default retention policy is **50 preparations and 64 MiB of estimated payload bytes per repository**, plus **500 preparations and 512 MiB globally**. The public `enforce_retention` seam accepts a smaller policy for controlled testing and returns before/after preparation counts, estimated bytes, deletions, and protected over-budget counts/bytes.

Ordinary preparations compete by least-recent `created_at`, with preparation ID as a stable tie-breaker. The implementation protects current generated-review contexts, active native-window pins, manual/code/reply/thread drafts, finding triage and alternative edits, anchor-repair state, review intents, unresolved review operations (`planned`, `sending`, `failed`, `unknown`), unexpired publication approvals, active load-latest approvals, and unresolved discussion plans/steps. Confirmed review publication history and settled discussion plans can be removed with an evicted preparation in the same transaction. Expired approvals are removed there; consumed approval tombstones remain until expiry so reuse retains the one-use error behavior.

Native windows pin before resuming a preparation and update the pin when selection changes; `Drop` clears the window pin. Pins include PID plus Linux process start identity, so cleanup removes only provably dead/reused PIDs. If process identity cannot be read, cleanup preserves the reference. A concurrent resume pin and cleanup serialize through SQLite's immediate write lock: a successful pin preserves the preparation, while cleanup winning first makes pinning fail with `ACTIVE_PREPARATION_NOT_FOUND` rather than leaving a dangling reference.

Remote preparation and discussion acquisition complete before cleanup starts. Cleanup opens/migrates local stores first, then uses a short `BEGIN IMMEDIATE` transaction and re-reads active pins, current results, drafts, approvals, and recovery references before deleting. SQLite foreign keys are enabled. Discussion rows cascade from observations; freshness rows and publication records are handled in FK-safe order. Any deletion failure rolls the transaction back.

Cleanup is invoked after preparation, discussion acquisition, generated-review commit, and publication completion. These hooks intentionally do not turn a successful acquisition/commit/publication into an application failure if pruning itself fails; callers that need diagnostics can call the public retention seam and inspect its report.

## Reproducible evidence

Focused real-SQLite tests use a test policy and controlled durable rows:

- 12 repeated preparations with discussion payloads, repeated store reopen/restart, current result, draft, unknown review operation, failed discussion plan, and a settled confirmed publication record: **9 preparations deleted, 3 retained, 1,324 estimated payload bytes retained** under a 3-per-repository/4-global preparation cap and 1/2 MiB byte budgets. The test reloads both protected preparations and verifies recovery/draft records remain; settled publication rows are removed with their evicted preparation.
- Byte/count-floor tests show ordinary LRU rows are removed while current/draft/active references survive. Protected overage is reported per repository and globally.
- An injected SQLite `BEFORE DELETE` failure leaves all three preparations, the failed review payload, and unknown discussion plan intact; WAL remains enabled.
- A real-SQLite byte-budget fixture creates an unassociated 13 KB source/context cache and orphan draft/generated-result payloads. Under 8-byte repository/global limits, the stale cache context is evicted while both generated result and draft survive; protected unassociated bytes and global/per-repository overage are reported.
- A three-cycle integration fixture performs real preparation saves and `GeneratedReviewStore::commit` imports, including a validated follow-up to the prior concern. It stages/confirms/publishes a review through the controlled `ReviewPublisher` seam, reopens the generated store after each cycle, injects a `BEFORE DELETE` failure before final cleanup and verifies all four preparations survive rollback, then removes the trigger and evicts to the two protected floors (published intent and current generated result): 2 preparations and 2,833 estimated payload bytes remain; 2 preparations and 1,306 estimated bytes are deleted.
- A concurrent pin/prune test verifies no dangling active pin or silently missing selected preparation.
- A native test runs 20 retention calls concurrently with 20 atomic `NavigationPreferences::save` writes and verifies the final preference value survives. The root preparation suite also covers `concurrent_workspace_store_open_serializes_feature_migrations` and `migration_lock_contention_waits_without_losing_a_saved_preparation`.
- A UTF-8 estimator test verifies byte counts use SQLite BLOB lengths rather than character counts.
- A consumed replacement-approval fixture verifies its short-lived tombstone and preparation survive retention until expiry.

The current full desktop run is recorded in [`VALIDATION-46-final.log`](VALIDATION-46-final.log). Earlier failed attempts and corrections are retained in [`VALIDATION-46-attempts.log`](VALIDATION-46-attempts.log) and the corresponding attempt logs. The prescribed `./scripts/validate desktop` suite covers the root contract/CLI and native spike checks/tests; no GUI was launched.

## Limits

These are logical retained-payload estimates based on serialized SQLite text/blob lengths. They exclude SQLite page/index overhead, free pages not yet reused by SQLite, WAL file overhead, and Git object packs in checkout repositories; deleting rows does not shrink the database file. A protected floor may exceed either budget, and the report exposes that overage rather than deleting protected work. Confirmed review provenance is eligible for eviction only with an evicted preparation; unresolved publication/recovery rows remain protected. No live GitHub publication or GUI interaction was performed.

Publication in the cycle test uses a controlled in-process publisher and does not contact GitHub. The test exercises three local preparation/import cycles, one follow-up validation, one confirmed controlled publication, store reopen between cycles, injected cleanup rollback, and subsequent eviction; it does not exercise live remote publication or a long-duration application restart across separate OS processes.
