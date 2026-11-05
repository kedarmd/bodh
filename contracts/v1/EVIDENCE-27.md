# Issue #27 · validate and atomically commit a generated review

Production validation and commit now use the shared Rust service with the existing preparation, discussion, draft, and publication state. `validate --prepared` checks the complete result against the current pinned context, acquired discussion observation, and previous committed concern index without changing state. `commit` repeats those checks and atomically stores one current full result plus the derived compact concern index while advancing the shared workspace generation.

Commit rejects a stale preparation or generation. Exact current-result retries are read-only no-ops, so newly saved drafts remain intact. A changed result is blocked while any unpublished inline, replacement, reply, thread-action, or grouped-review work exists, or while a publication operation is still planned, sending, or uncertain. SQLite failures roll back both the result replacement and generation increment. The CLI reports current full results through `inspect --kind previous` and marks availability in the preparation manifest.

## Verification

- `CARGO_BUILD_JOBS=2 ./scripts/validate desktop`: passed on the #27 worktree based on integration `4933f86`; full output is in [VALIDATE-27.log](VALIDATE-27.log).
- Ten tests in `tests/generated_review.rs` cover read-only validation, CLI envelopes, atomic initial/replacement rollback, generation and preparation staleness, exact retry with draft preservation, and guards for replacement drafts and staged grouped reviews.
- The full desktop runner compiled and checked the native application. No interactive GUI trial or remote GitHub write was performed.

## Boundaries

Changed generated results remain blocked with `APPROVAL_REQUIRED` when unpublished reviewer work exists. This slice does not implement the human authorization/replacement workflow; it prevents changed candidates from bypassing that workflow. The service owns one current full result per PR, while historical result retention is not added here.
