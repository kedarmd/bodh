# Issue #31 — finding triage and editable alternatives

## Shared triage evidence

`FindingTriageStore` binds triage to a committed finding, persists dispositions and manual feedback separately from the generated result, and stores edits to text/code alternatives with their own revisions. Staging an alternative selects one choice; switching choices retains the previous edited draft. Linking a discussion does not publish or stage duplicate feedback.

The latest full desktop validation at integration `d1a0627` passed; see [VALIDATE-REVIEW-FIX-RETRY.log](VALIDATE-REVIEW-FIX-RETRY.log). Relevant cases include:

- `tests/findings.rs::triage_order_prioritizes_severity_and_keeps_original_order_within_it`
- `tests/generated_review.rs::finding_triage_is_bound_to_a_committed_finding_and_survives_reopen`
- `tests/generated_review.rs::independent_manual_finding_feedback_is_persisted_without_publication`
- `tests/generated_review.rs::switching_a_finding_alternative_preserves_other_edits_and_unstaging_keeps_drafts`
- `tests/generated_review.rs::replacement_keeps_only_compact_prior_concern_text_and_exact_source_metadata`
- `tests/findings.rs::related_threads_must_be_observed_and_incomplete_absence_is_explicit`
- `tests/complete.rs::complete_validator_accepts_empty_results_and_all_supported_diagram_steps`

The user accepted the Findings visual composition under #32 after reviewing the installed screen. That feedback establishes layout acceptance only; it does not establish the semantic correctness of an analysis or acceptance of every triage control.

## Remaining native evidence

The tests above exercise shared persistence/reducer seams; they do not constitute a complete interactive native trial of empty results, edited-alternative switching/restart, related outdated discussions, invalid anchors and concurrent result changes. That cross-workflow native matrix is tracked in [EVIDENCE-37.md](EVIDENCE-37.md) and remains partial.
