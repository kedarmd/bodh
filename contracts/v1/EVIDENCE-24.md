# Issue #24 — uncertain publication recovery status

## Verified recovery behavior

The publication journals persist immutable request payloads and transition planned → sending → confirmed/failed/unknown. A restart changes a persisted sending operation to unknown; the UI/CLI can reload the saved plan. Confirmed operations retain remote IDs/links and their exact associated draft revisions. Unknown operations block blind resend and protect their payloads from result replacement.

The latest full desktop validation at integration `d1a0627` passed; see [VALIDATE-REVIEW-FIX-RETRY.log](VALIDATE-REVIEW-FIX-RETRY.log). Focused public-service cases include:

- `tests/manual_review.rs::interrupted_send_becomes_unknown_and_cannot_be_started_again`
- `tests/manual_review.rs::rejection_is_definitive_but_timeout_after_request_is_unknown_and_not_retryable`
- `tests/discussion_publication.rs::interrupted_sending_step_becomes_unknown_and_cannot_be_retried`
- `tests/discussion_publication.rs::independent_outcomes_confirm_only_accepted_steps_and_keep_reply_payload_exact`
- `tests/generated_review.rs::unresolved_publication_payload_blocks_replacement_authorization`

## Not implemented or not verified

There is no read-only remote reconciliation command/service that compares an unknown operation against account, PR, body/location, IDs and attempt time. Ambiguous remote matches therefore remain unknown for reviewer inspection; the application does not claim to reconcile them automatically. The full grouped-review/replacement/reply/thread-action timeout, acknowledgment-crash, partial-success, restart and manual-reconciliation matrix has not been run. No live write or remote reconciliation was performed.

All five #24 acceptance items remain open until read-only reconciliation and the complete recovery matrix are implemented and tested.
