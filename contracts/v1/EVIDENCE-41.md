# Issue #41 — reply-only and thread-only confirmation

## Public-service evidence

The shared `DiscussionPublicationStore::confirm` service accepts discussion selections without a grouped review operation ID. The public controlled-publisher regression test `reply_only_and_thread_only_plans_confirm_only_real_selected_operations` exercises both cases end to end: it confirms and sends a reply-only plan and a thread-action-only plan, verifies each plan has no associated grouped review, and confirms that only the selected operation is sent. The same test target also covers mixed independent outcomes, permission loss, already-achieved state, stale drafts and interrupted-send recovery.

Run with:

```sh
CARGO_BUILD_JOBS=2 cargo test --locked --test discussion_publication
```

The test target passed 5/5 on integration revision `680c410` plus the local ticket change. No live GitHub write was performed.

## Boundary

The tests verify shared-service planning and transport calls. They do not qualify the full native Finish Review interaction; that remains in the parent #23 / integrated #37 qualification scope.
