# Issue #42 — reconcile uncertain grouped review outcomes

## Public-service evidence

`ReviewPublicationStore::reconcile_unknown` accepts only an operation in the durable `unknown` state and a `ReviewOutcomeReader`. `GhReviewOutcomeReader` performs a read-only GraphQL query for the authenticated viewer, target pull request, reviews, review threads, and comments. A complete listing must identify exactly one review matching the viewer, repository and pull request, event, summary, head commit, comment bodies and locations, and a bounded submitted time after the persisted send-start time.

Zero or multiple exact candidates, a mismatched repository or PR, and incomplete pagination leave the operation unknown. Missing required identity or unavailable GitHub data returns an error while preserving the unknown state. A unique candidate atomically confirms the operation and records its published draft provenance; its serialized payload is never rewritten.

## Controlled regression coverage

`tests/publication_reconciliation.rs` exercises a unique match, no match, duplicate matches, wrong actor/repository, incomplete pagination, and unavailable GitHub response through the public store and reader seam. It uses an isolated Git/SQLite fixture and a fake `gh` executable. No live GitHub write is performed.

Run with:

```sh
CARGO_BUILD_JOBS=2 cargo test --locked --test publication_reconciliation
```

## Qualification boundary

The fixture covers an ordinary right-side inline comment. It does not qualify all GitHub diff-side variants or the native GUI reconciliation interaction. Evidence is read-only; this ticket does not retry or resend an uncertain operation.
