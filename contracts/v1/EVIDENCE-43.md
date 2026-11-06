# Ticket #43: Uncertain discussion publication reconciliation

## Implemented

- Publication steps persist the start time before invoking the writer. Restart recovery preserves that timestamp for later reconciliation.
- Reconciliation is a read-only operation and never retries a reply or thread action.
- A reply is confirmed only when the fresh GitHub thread observation is complete and exactly one comment matches the authenticated viewer, exact body, target root comment, and attempt time. The remote numeric comment ID is then recorded and the exact reply revision is marked published.
- A desired resolve/reopen state is classified as `already_achieved`; no actor attribution or published action marker is created.
- Missing, incomplete, ambiguous, stale-head, malformed-time, and unavailable observations remain unknown.

## Validation

- Controlled `tests/discussion_publication.rs` cases cover unique reply confirmation, desired state without reviewer attribution, and ambiguous/unavailable evidence remaining unknown. They assert no second writer call during reconciliation.
- No live GitHub writes were made.

## Limits

- Reply reconciliation currently fails closed when the relevant thread has more than the first 100 comments, because the focused reconciliation query does not paginate. It does not broaden matching to other pages.
- Thread action reconciliation observes current state but cannot establish who caused that state; it therefore never marks the action as published by this reviewer.
