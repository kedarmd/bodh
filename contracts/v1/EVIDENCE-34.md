# Issue #34 — migration and retained-cache evidence

## Changes

- SQLite connections share bounded WAL setup retry. The initial preparation schema now takes `BEGIN IMMEDIATE` before reading `user_version`; independent draft and publication migration ledgers likewise read their versions while holding the write lock. This prevents two first-open processes from both applying the same version.
- Discussion observations are transient cached reads. At the beginning of a new fetch, a short transaction removes abandoned partial runs while retaining the latest complete observation; after acquisition, another short transaction retains the latest partial attempt and the latest complete observation. A successful complete refresh replaces the old discussion cache history. Both transactions occur outside GitHub pagination. This cleanup touches only `discussion_observations` and its cascading page/thread cache rows.
- Exact Git source remains read from its pinned object. The migration and cleanup paths do not rewrite generated results, compact prior concerns, drafts, protected replacement grants, or publication operations and recovery payloads.

## Focused verification

- Red/green public API fixture: simultaneous `PreparationStore::open` calls initially failed with `STATE_UNAVAILABLE`; the migration now passes with both callers succeeding and `user_version=1`.
- Red/green public API fixture: concurrent first opens through `DraftStore::open` initially failed; the serialized draft and publication migrations now pass.
- Migration lock contention test holds a SQLite writer lock while another store opens; after lock release the saved preparation and description load unchanged.
- Failed migration fixture resets the version marker on a populated schema, verifies the attempted migration fails without deleting the persisted preparation, then restores the marker and reloads the preparation.
- Discussion retention fixture performs a complete fetch, three failed refreshes, and a complete refresh. It verifies the cache retains at most one complete fallback plus the latest partial attempt, then one completed observation after success. It also verifies the latest observation remains available through the public load API.
- `cargo fmt --check` and `git diff --check` pass.
- `CARGO_BUILD_JOBS=2 cargo test --locked --test preparation` passes (7 tests).
- `CARGO_BUILD_JOBS=2 cargo test --locked --test discussions` passes (8 tests).
- `CARGO_BUILD_JOBS=2 cargo test --locked --test manual_drafts` passes (14 tests).
- `CARGO_BUILD_JOBS=2 cargo test --locked --test discussion_drafts` passes (5 tests).
- Integrated `CARGO_BUILD_JOBS=2 ./scripts/validate desktop` passes after resolving a lock-ordering regression in load-latest approval consumption; full output is [VALIDATE-34-final.log](VALIDATE-34-final.log). The exact freshness regression test passed five repeated runs, then the full freshness target passed (5 tests).

## Exact-object and recovery references

- `tests/evidence.rs::historical_source_reads_exact_commit_and_reports_missing_objects` reads source only from the requested immutable commit and returns `GIT_OBJECT_UNAVAILABLE` for a missing object. The evidence service does not fall back to current checkout files.
- `tests/manual_review.rs::interrupted_send_becomes_unknown_and_cannot_be_started_again` and `tests/discussion_publication.rs::interrupted_sending_step_becomes_unknown_and_cannot_be_retried` reload unresolved publication records as `unknown` and reject blind retries. The payload/journal records remain available for explicit reconciliation.
- These existing service tests establish unavailable-object behavior and recovery-record retention. They do not simulate deleting a previously available Git object or run the full prepare/import/follow-up/publication/restart flow under forced eviction.

## Limits

- The bounded policy is scoped to discussion cache history per preparation. Preparation rows and contexts are not globally evicted because drafts, generated results, and publication recovery do not share a complete safe reference/retention index yet. Human drafts, current generated results, compact prior concerns, and unresolved publication payloads remain preserved; this means total database size is not yet globally bounded.
- The cache is bounded by observation count, not by on-disk byte size. Each acquired observation already has an 8 MiB transport limit, but page rows and normalized thread rows duplicate some data. Final numeric storage budgets remain for qualification.
- Load-latest approval consumption now reads immutable preparation bindings before acquiring its write transaction, then revalidates the approval inside the transaction; this avoids nested WAL setup under a held write lock. The interrupted migration test simulates a stale version marker against an existing schema. No process-kill fault injection was performed between SQLite DDL statements; SQLite transactional rollback is exercised through the failed migration path.
- No full repeated prepare/import/follow-up/publication/restart workflow was run under forced Git-object eviction. No native GUI and CLI preference-write race was run. Existing exact-source behavior reports unavailable pinned objects rather than substituting current worktree source; this ticket's cache cleanup does not alter it.
- No live GitHub writes were performed.
