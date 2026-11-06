# Issue #52 — retry a definitively failed grouped review safely

## Behavior

`ReviewPublicationStore::retry_failed` accepts only a GitHub-rejected grouped review. It rejects local pre-send failures, sending, unknown, planned, and confirmed operations. It rechecks the retained PR identity, open state, head/base, write permission, current selected draft revisions/content, and source anchors. Confirmed draft/replacement provenance is excluded even when a local draft remains after outcome reconciliation.

A retry is a new journal operation whose serialized payload and fingerprint match the failed operation exactly. The prior failure remains in history. A unique `retry_of` relationship permits at most one direct retry per failed operation while allowing a later failed retry to be retried as its own operation. The Finish Review action appears only for failed history without unresolved work and is suppressed once that row has a retry child. Clicking it explicitly checks current remote state, creates the new immutable operation, and submits that same payload. Unknown, sending, and confirmed states never expose the retry action.

## Controlled tests

`tests/review_retry.rs` covers rejection followed by a distinct operation with identical retained payload and preserved history; duplicate retry blocking; unknown outcome exclusion; stale PR head and draft revision rejection; and exclusion of a draft revision already recorded as confirmed. UI helper tests cover failed-only eligibility, unresolved work, duplicate retry children, and retrying a failed retry child. The tests use temporary Git/SQLite fixtures and do not write to GitHub.

## Design and scope

The Finish Review action reuses the approved D2 buttons, cards, status colors, and spacing. No canonical design token or component changed. The work is limited to grouped review publication and its Finish Review presentation; discussion publication is unchanged.

## Qualification boundary

Native GUI/pointer/keyboard acceptance was not verified. No live GitHub write was performed. The `gh` publisher path is covered through controlled tests only.

## Executed validation

- `cargo test --locked --test review_retry` — 4 passed. Covers a rejected review followed by a distinct immutable attempt with the same payload/fingerprint and retained history; duplicate retry blocking; unknown outcome exclusion; moved head, insufficient permission, and edited draft rejection; and confirmed draft provenance exclusion.
- `cargo test --locked --manifest-path spikes/native-sequence/Cargo.toml --bin bodh-sequence-spike publication_recovery_ui_tests::retry_action_is_only_available_for_failed_history_without_unresolved_work` — 1 passed. Covers failed-only eligibility, suppression when a direct retry child exists or unresolved work is present, and allowing a failed retry child its own retry.
- `cargo fmt --manifest-path Cargo.toml`
- `cargo fmt --manifest-path spikes/native-sequence/Cargo.toml`
- `cargo clippy --locked --all-targets -- -D warnings` — passed.
- `cargo clippy --locked --manifest-path spikes/native-sequence/Cargo.toml --bin bodh-sequence-spike -- -D warnings` — passed; Cargo emitted the existing future-incompatibility notice for `proc-macro-error2`.
- `git diff --check` — passed.
