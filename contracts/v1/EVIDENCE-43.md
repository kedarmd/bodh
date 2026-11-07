# Ticket #43: Uncertain discussion publication reconciliation

## Implemented

- Publication steps persist the start time before invoking the writer. Restart recovery preserves that timestamp for later reconciliation.
- Reconciliation is a read-only operation and never retries a reply or thread action.
- A reply is confirmed only when the fresh GitHub thread observation is complete and exactly one comment matches the authenticated viewer, exact body, target root comment, and attempt time. The remote numeric comment ID is then recorded and the exact reply revision is marked published.
- A desired resolve/reopen state is classified as `already_achieved`; no actor attribution or published action marker is created.
- Missing, incomplete, ambiguous, stale-head, malformed-time, unavailable, or permission-lost observations remain unknown.

## Validation

- Controlled `tests/discussion_publication.rs` cases cover unique reply confirmation, desired state without reviewer attribution, ambiguous/unavailable evidence remaining unknown, lost acknowledgments and interrupted-send recovery. They assert no second writer call during reconciliation.
- `tests/discussion_reconciliation.rs` exercises the concrete read-only GitHub adapter through fake `gh` responses: a unique authenticated reply is confirmed; wrong-author and duplicate exact matches remain unknown; lost reply permission remains unknown without reading comments; and stale-head/unavailable observations do not confirm an outcome.
- `CARGO_BUILD_JOBS=2 cargo test --locked --test discussion_publication` passed (8 tests); `CARGO_BUILD_JOBS=2 cargo test --locked --test discussion_reconciliation` passed (4 tests). Formatting and diff checks passed.
- No live GitHub writes or native GUI trials were performed.

## Limits

- Reply reconciliation currently fails closed when the relevant thread has more than the first 100 comments, because the focused reconciliation query does not paginate. It does not broaden matching to other pages.
- Thread action reconciliation observes current state but cannot establish who caused that state; it therefore never marks the action as published by this reviewer.
