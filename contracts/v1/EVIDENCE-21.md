# Issue #21 · grouped manual review publication

The manual text-comment path now has a persisted Finish Review intent and a separate publication journal. Reviewers can select or unstage saved inline drafts, return to Changes to edit a draft, enter a summary, and choose Comment, Approve, or Request Changes explicitly. Approval is never inferred. The native screen previews the exact payload and fingerprint before its separate Publish action.

Confirmation verifies the selected draft and workspace revisions, revalidates every exact pinned diff anchor, and checks the latest PR node ID, head, base, open state, and viewer permission. The journal stores an immutable payload and fingerprint before network I/O. It records planned → sending → confirmed, failed, or unknown, and records the confirmed remote review ID/link. Requests use `gh api graphql --input -` with JSON stdin. Network I/O runs after the SQLite transaction. A second send is rejected once an operation is sending, confirmed, or unknown. On reopening Finish Review, an interrupted sending operation becomes unknown and its exact plan remains visible for inspection.

Confirmed draft revisions are associated with their operation in the journal. The unpublished draft list and counts omit only those exact revisions; editing the draft later produces a new unpublished revision. A nonempty unsubmitted summary contributes one `review_comments` item to the draft counts, and confirmation clears that count for the exact intent revision. The immutable operation retains the published body, anchor, revision, and confirmation provenance.

## Verification

- `CARGO_BUILD_JOBS=2 ./scripts/validate desktop` passed on integration tip `d35f52c`; raw runner output is in [VALIDATE-21.log](VALIDATE-21.log).
- Seven focused tests in `tests/manual_review.rs` cover explicit events, stale/head/permission checks, CLI intent persistence, exact GraphQL payload and remote link, rejection, timeout uncertainty, restart recovery, duplicate-send refusal, and exact-revision published status.
- The runner also passed the existing #19 replacement and #20 discussion-draft suites.
- All publication tests use controlled local `gh` executables. No live GitHub write was performed.
- The native code typechecks as part of desktop validation. An interactive Finish Review trial has not yet been conducted; it requires the user's coordinated native app session.

## Boundaries

This operation groups newly drafted inline text comments with the explicitly selected review event and summary. Reply drafts and resolve/reopen intents from #20 remain distinct pending work because they use different GitHub mutations; they are not silently folded into a grouped review. Code-replacement drafts from #19 also remain outside this inline-comment payload. A future shared selection model can add discriminated draft kinds and their separate journaled operations without changing the immutable grouped-review payload format.
