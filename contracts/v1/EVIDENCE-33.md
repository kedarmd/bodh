# Issue #33 agent workflow evidence

## Migrated workflow

The repository-local `pr-review` skill is at [`.agents/skills/pr-review/SKILL.md`](../../.agents/skills/pr-review/SKILL.md). It retains the established correctness, security, regression, reliability, testing, and convention passes, then uses Bodh's public contract, prepared CLI, exact pinned reads, validation, and commit. It documents the supported PR-plus-checkout scope and a maximum of three correction rounds after initial validation.

The workflow uses the normal XDG state database and passes it explicitly to every Bodh command. It treats preparation ID, context, pinned SHAs, and generation as one snapshot. Incomplete discussions and tests that were not run stay explicitly incomplete. Empty findings/diagrams are valid. Existing active discussions are not copied into findings without distinct justification.

Changed results that would replace protected reviewer work require Bodh's interactive, one-use authorization. The reviewer must type the PR identity in a terminal; unattended invocations stop. The skill contains no report generator, action server, Mermaid/SVG output, direct GitHub write, GUI automation, or background service lifecycle.

## Existing behavior exercised through public seams

The existing CLI/service tests cover the workflow seams used by the skill:

- `tests/preparation.rs::prepares_ordinary_pr_from_pinned_local_commits_without_touching_checkout_state` runs real `prepare` and pinned `inspect` CLI processes with temporary Git and controlled `gh`, and checks the checkout/index remain unchanged.
- `tests/generated_review.rs::cli_validate_then_commit_reports_complete_validation_and_persists_result` exercises production `validate` and `commit` through child processes.
- `tests/generated_review.rs::cli_replacement_authorization_requires_a_terminal`, `an_explicit_replacement_approval_is_bound_to_the_candidate_and_clears_only_its_drafts`, and `replacement_approval_is_one_use_and_rejects_a_different_candidate` cover terminal gating and approval binding/consumption.
- `tests/generated_review.rs::replacement_approval_expires_when_a_protected_draft_revision_changes`, `stale_generation_and_invalid_result_leave_the_current_review_untouched`, and `a_newer_preparation_makes_an_older_candidate_stale` cover stale approvals and stale state without replacing the active result.
- `tests/generated_review.rs::replacement_keeps_only_compact_prior_concern_text_and_exact_source_metadata` and `tests/complete.rs::follow_up_requires_real_prior_concerns_and_current_actionable_findings` cover valid follow-up relationships and compact persistence. `tests/discussions.rs::inspection_cli_returns_persisted_observation_and_manifest_summary` and `tests/findings.rs::related_threads_must_be_observed_and_incomplete_absence_is_explicit` cover acquired and incomplete discussion evidence.
- `tests/generated_review.rs::validation_is_read_only_and_commit_is_generation_checked_with_exact_retry_noop` covers validation failure retention, generation checks, and the unchanged-result no-op. `tests/generated_review.rs::an_unresolved_publication_payload_blocks_replacement_authorization` covers in-flight publication protection.
- `tests/complete.rs::complete_validator_accepts_empty_results_and_all_supported_diagram_steps` verifies empty findings and diagrams.

The CLI tests invoke finite child processes; the tested commands return without a daemon. The skill itself does not launch any background process. This change did not execute an LLM-driven review against a live PR. Model analysis quality, the three-correction/exhaustion path under an actual provider, successful human replacement authorization through a live terminal, and a complete prepare-to-commit agent run remain unverified. The skill records the retry and authorization rules but does not claim those trials as evidence.
