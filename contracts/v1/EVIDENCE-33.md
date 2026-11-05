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
- The remaining generated-review tests cover empty results, prior-concern follow-up, validation failure retention, stale generations, exact-result no-op, changed candidates blocked by protected drafts, and interrupted publication protection.
- `tests/complete.rs::complete_validator_accepts_empty_results_and_all_supported_diagram_steps` verifies empty findings and diagrams.

These CLI operations are finite child processes; no daemon is started by this workflow. This change did not execute an LLM-driven review against a live PR. Model analysis quality, retry behavior under an actual provider, and successful human replacement authorization in a controlled live workspace remain unverified; the skill records the retry and authorization rules but does not claim those trials as evidence.
