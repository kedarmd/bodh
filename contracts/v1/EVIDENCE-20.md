# Issue #20 — discussion workflow evidence

This slice exposes the read-only thread observation in a native Discussions tab and adds durable local reply and thread-action drafts. The shared Rust `DraftStore` owns permission checks, identity checks, revisions, persistence and pending counts; the GPUI adapter calls those same services off the UI thread.

## Verified behavior

- Threads and comments are rendered from the latest acquired observation with author attribution, root/reply content, resolved/outdated state, current and original line information, and diff side. Outdated threads explicitly have no current line.
- Replies are bound to a thread ID and its observed root-comment ID. Reply permission and root identity are checked again against the latest saved observation when saving. Editing cannot silently retarget an existing draft.
- Resolve/reopen intent is bound to the thread ID and current observed state/permission. Pending intent text calls out the observed remote state and that it has not been applied.
- Reply and thread-action records persist in additive SQLite tables and contribute to workspace revisions and `DraftCounts`. Save, edit, discard, stale revision, restart, permission loss and JSON CLI envelopes have integration coverage. The shared version-2 migration test confirms reply/action and code-replacement tables are created idempotently regardless of ticket migration order; the combined #19/#20 tree passes both draft test suites.
- Discussion refresh calls the existing independently paginated, read-only `gh api graphql` acquisition service. Staging or changing tabs does not call a remote mutation.

## Local evidence

- `cargo test --locked --test discussion_drafts`: passed (4 tests), including local CLI envelopes, restart, counts, root binding, stale revision, independent version-2 migration ordering and a later observation with reply/action permission removed.
- `cargo test --locked --test manual_drafts`: passed (4 tests), confirming existing inline draft behavior and shared revisions still work.
- `cargo test --locked --test discussions`: passed (7 tests), including independently paginated nested replies and partial observations.
- `cargo check --locked --manifest-path spikes/native-sequence/Cargo.toml`: passed.
- `CARGO_BUILD_JOBS=2 ./scripts/validate desktop`: passed on final integration tip `1a95b47` plus the #20 merge; output header records local commit `9f0507d` and a dirty tree because the evidence log was being refreshed. Full output is in `VALIDATE-20.log`.
- A coordinated interactive trial of this new Discussions tab was not performed in this slice. Native keyboard/focus behavior is implemented using the existing focused key-handling pattern but remains unobserved; no claim of live interaction acceptance is made.

## Limits

Original GitHub diff sides are not exposed by the existing GraphQL selection, so the UI preserves the service's explicit `not_exposed_by_graphql` status instead of synthesizing one. Partial thread observations remain partial. There is no publish/reply mutation in this slice.
