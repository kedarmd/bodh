# Issue #23 — reply and thread-action publication evidence

## Shared publication service

`DiscussionPublicationStore` stores immutable plans and per-step outcomes, checks permission/head/thread state before sending, uses the top-level numeric comment ID for replies and the GraphQL thread node ID for resolve/reopen, and marks only confirmed steps published.

`tests/discussion_publication.rs` and `tests/discussion_reconciliation.rs` now cover the following public-service slices:

- `independent_outcomes_confirm_only_accepted_steps_and_keep_reply_payload_exact` — separate reply/action outcomes, exact IDs and payload, partial success.
- `permission_loss_and_already_achieved_action_send_nothing_and_never_claim_publication` — fresh permission/state rejection without a send.
- `changed_draft_revision_is_detected_after_confirmation_before_any_write` — stale draft rejection.
- `interrupted_sending_step_becomes_unknown_and_cannot_be_retried` — restart uncertainty and blind-retry refusal.
- [#40 evidence](EVIDENCE-40.md) covers grouped review ordering, rejection/unknown halt, retained unsent steps and restart without dependent resend.
- [#41 evidence](EVIDENCE-41.md) covers reply-only and thread-action-only plans without a synthetic review operation.
- [#43 evidence](EVIDENCE-43.md) covers read-only discussion reconciliation, including unique authorship and ambiguous/permission/head states.

The controlled publishers are local; no live GitHub write was attempted. The product flow confirms grouped review and discussion plans separately. `DiscussionPublicationStore::publish` refuses dependent discussion work until its grouped review is confirmed, and retains it as unsent/unknown when the grouped operation fails or is uncertain.

## Remaining evidence gaps

The public-service grouped ordering and reply/thread-only plan cases are now covered, but the exact GitHub endpoint compatibility matrix and full Finish Review UI publication workflow remain incomplete. The full remote-race fixture matrix and live endpoint write were not run. Keep #23 open for the remaining acceptance evidence.
