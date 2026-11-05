# Issue #23 — reply and thread-action publication evidence

## Shared publication service

`DiscussionPublicationStore` stores immutable plans and per-step outcomes, checks permission/head/thread state before sending, uses the top-level numeric comment ID for replies and the GraphQL thread node ID for resolve/reopen, and marks only confirmed steps published.

The latest full desktop validation at integration `d1a0627` passed; see [VALIDATE-REVIEW-FIX-RETRY.log](VALIDATE-REVIEW-FIX-RETRY.log). `tests/discussion_publication.rs` covers:

- `independent_outcomes_confirm_only_accepted_steps_and_keep_reply_payload_exact` — separate reply/action outcomes, exact IDs and payload, partial success.
- `permission_loss_and_already_achieved_action_send_nothing_and_never_claim_publication` — fresh permission/state rejection without a send.
- `changed_draft_revision_is_detected_after_confirmation_before_any_write` — stale draft rejection.
- `interrupted_sending_step_becomes_unknown_and_cannot_be_retried` — restart uncertainty and blind-retry refusal.

All transport is a controlled local publisher; no live GitHub write was attempted. The product flow confirms grouped review and discussion plans separately. `DiscussionPublicationStore::publish` refuses dependent discussion work until its grouped review is confirmed, and retains it as unsent/unknown when the grouped operation fails or is uncertain.

## Remaining evidence gaps

There is no focused integration test for grouped-review ordering/halt combined with reply/thread-only plan confirmation, and no live endpoint compatibility write. The separate Finish Review UI was not exercised as one integrated publication workflow. These bundled ordering/plan compatibility clauses remain open until those cases are tested.
