# Issue #19 — editable code-replacement drafts

## Shared-service and CLI evidence

The production `DraftStore` saves replacement source, exact original text, replacement text, provenance, and optimistic revision separately from generated suggestions. The public CLI exposes replacement list/save/discard operations.

The latest full desktop validation at integration `d1a0627` passed; its complete output is [VALIDATE-REVIEW-FIX-RETRY.log](VALIDATE-REVIEW-FIX-RETRY.log). Relevant public-service/CLI cases in `tests/manual_drafts.rs` include:

- `code_replacement_drafts_validate_exact_head_spans_and_keep_checkout_unchanged`
- `code_replacement_accepts_inclusive_multiline_head_range_and_preserves_final_newline`
- `replacement_source_round_trips_crlf_and_missing_final_newline_exactly`
- `edited_renames_are_eligible_by_head_path_and_binary_spans_are_rejected`
- `code_replacements_reject_old_side_changed_source_and_stale_concurrent_saves`
- `replacement_cli_uses_separate_list_and_save_envelopes`
- `cli_save_list_and_discard_use_json_envelopes_without_publishing`
- `concurrent_workspace_and_draft_revisions_reject_stale_writes_without_losing_text`

`tests/manual_review.rs` additionally covers application-generated suggestion Markdown, exact source revalidation and removal only after a confirmed send. Rejected and uncertain sends keep the replacement available. No live GitHub write was performed.

## Native UI boundary

The product UI displays replacement drafts in Finish Review, provides edit/unstage routes, and renders the exact structured suggestion preview. The ticket evidence does not contain an interactive native trial of replacement editing, keyboard/focus/theme behavior, or Finish Review publication. The #32 visual acceptance covered the Findings composition and did not exercise replacement controls. These native interaction clauses remain unverified.
