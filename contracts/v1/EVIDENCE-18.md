# Issue #18 implementation evidence

Issue #18 adds durable, resumable manual text-comment drafts to the local review workspace.

## Behavior

- Drafts are stored in an additive SQLite migration separate from generated review results and navigation preferences. The namespaced migration ledger preserves the preparation database's existing `user_version`.
- New and resumed drafts retain their exact source anchor and immutable preparation provenance. Saving checks the current workspace revision, the individual draft revision, and anchor eligibility against the pinned diff before writing.
- The `draft list`, `draft save`, and `draft discard` CLI commands use structured JSON envelopes. Draft save/discard are local operations and do not publish comments.
- The native editor supports local save, resume, explicit discard, and recovery as a new draft after a stale conflict. Stale save keeps the typed text visible; stale discard leaves the persisted row and editor text intact. Save completion also preserves keystrokes entered after a save began.
- Home/review navigation guards prevent leaving while editor text is dirty or a write is in flight. The workspace status shows local draft counts.

## Verification

`CARGO_BUILD_JOBS=2 ./scripts/validate desktop` passed. Raw output is preserved in [VALIDATE-18.log](VALIDATE-18.log).

Focused tests use real temporary SQLite databases and Git repositories. Four backend/CLI tests cover reopen with pinned provenance, rejection of unchanged-source anchors, conflicting workspace/draft revisions without losing the winning text, and real CLI JSON envelopes for save/list/discard. Seven native editor tests cover stale-save retention, post-submit typing, resumed draft state, stale recovery as a new copy, explicit discard behavior, and source/diff presentation helpers.

No interactive native GUI trial was performed; no pointer or key input was sent to the native application.
