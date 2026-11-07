# Issue #24 — uncertain publication recovery status

## Verified recovery behavior

The publication journals persist immutable request payloads and transition planned → sending → confirmed/failed/unknown. A restart changes a persisted sending operation to unknown; the UI/CLI can reload the saved plan. Confirmed operations retain remote IDs/links and their exact associated draft revisions. Unknown operations block blind resend and protect their payloads from result replacement.

The latest local full desktop validation passed on the `53e2977` integration base with #53 changes in the dirty tree; see [VALIDATE-53-final.log](VALIDATE-53-final.log). Focused public-service cases include:

- `tests/manual_review.rs::interrupted_send_becomes_unknown_and_cannot_be_started_again`
- `tests/manual_review.rs::rejection_is_definitive_but_timeout_after_request_is_unknown_and_not_retryable`
- `tests/discussion_publication.rs::interrupted_sending_step_becomes_unknown_and_cannot_be_retried`
- `tests/discussion_publication.rs::independent_outcomes_confirm_only_accepted_steps_and_keep_reply_payload_exact`
- `tests/generated_review.rs::unresolved_publication_payload_blocks_replacement_authorization`

## Implemented child slices and remaining gaps

- [#42](EVIDENCE-42.md) adds read-only grouped-review reconciliation and controlled exact-match, no-match, duplicate, wrong-actor/repository, incomplete-pagination and unavailable-response fixtures. Its fixture covers a normal right-side inline comment; the native recovery UI is not qualified.
- [#43](EVIDENCE-43.md) adds read-only reply/thread reconciliation, exact authenticated reply matching, no false attribution for already-achieved thread state, permission/head/unavailable safeguards, and controlled restart/ambiguity fixtures. The reader fails closed beyond the first 100 comments instead of paginating.
- [#40](EVIDENCE-40.md) verifies grouped-review ordering and dependent-operation halt after rejection/unknown; [#41](EVIDENCE-41.md) verifies reply-only and thread-action-only plan confirmation.

The complete #24 user flow remains partial. The resumed native workspace now shows persisted publication history and exposes safe retry actions for definitively failed grouped reviews and discussion work through [#50](EVIDENCE-50.md), [#52](EVIDENCE-52.md), and [#53](EVIDENCE-53.md). The complete crash/timeout/acknowledgment/partial-success matrix across grouped review, replacement suggestions, replies and thread actions has not been run. There was no live GitHub write or native GUI recovery trial. In particular, endpoint-specific race behavior and the full integration across confirmed links, safe retry, result replacement and restart remain unqualified.

Keep #24 open until those UI and integrated recovery criteria are verified. Reconciliation is now implemented for grouped reviews and discussion writes; it should no longer be described as entirely absent.
