# Issue #40 — grouped review ordering before discussion operations

## Public-service verification

The grouped review and discussion plan are independently persisted before the first write. The discussion plan records its grouped review operation ID plus the selected reply and thread-action steps in send order. Its publisher gate refuses dependent sends until the linked review operation is confirmed.

`tests/discussion_publication.rs` exercises the shared publication stores with a real temporary Git repository and SQLite database, a controlled grouped-review publisher, and a controlled discussion publisher:

- `grouped_review_is_confirmed_before_dependent_operations_and_order_survives_reopen` asserts the exact review → reply → thread-action order and that confirmed step states survive reopening the store.
- `rejected_or_uncertain_grouped_review_keeps_dependent_steps_unsent_after_restart` covers definite rejection and timeout/unknown. The discussion steps remain `unsent`, the failed or unknown plan status persists after reopening, and a second publish attempt is rejected without another dependent call.

Run with:

```sh
CARGO_BUILD_JOBS=2 cargo test --locked --test discussion_publication grouped_review
```

This passed (2 tests). `cargo fmt --all -- --check` and `git diff --check` also passed. The tests perform no live GitHub writes and do not establish native Finish Review interaction behavior.
