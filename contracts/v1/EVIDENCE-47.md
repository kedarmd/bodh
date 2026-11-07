# Issue #47 — agent correction and legacy-path evidence

## Verified behavior

The migrated skill at [`.agents/skills/pr-review/SKILL.md`](../../.agents/skills/pr-review/SKILL.md) documents three correction rounds after initial validation, requires full revalidation after every correction, and directs the agent to stop with the error code, pointer, and missing evidence on exhaustion. It also directs the agent to stop on stale preparation/context/generation or missing approval.

The skill uses the Bodh contract, pinned `prepare`/`inspect`, `validate`, and `commit` workflow. The migrated file contains no custom HTML/CSS/JS report generation, action server, Mermaid/SVG output, direct GitHub publication, GUI automation, or daemon lifecycle. The finite public-service test `tests/generated_review.rs::cli_failed_validation_correction_rounds_preserve_committed_result_and_drafts` passes and verifies that an initial validation plus three invalid correction candidates leaves the committed result and reviewer draft unchanged.

Run with:

```sh
CARGO_BUILD_JOBS=2 cargo test --locked --test generated_review cli_failed_validation_correction_rounds_preserve_committed_result_and_drafts -- --exact
```

No live model/provider run was performed.

## Remaining gap

The repository does not include a provider orchestration runtime for executing the skill as a deterministic program. The test exercises the public Bodh CLI across the documented attempts, but it cannot establish that a model-driven agent obeys the stop-after-three instruction, presents the exhaustion result, or leaves no provider-owned background process. Those provider-level claims remain unverified and are not claimed complete.

## Follow-up: current-head correction and result checks — 2026-11-07

- On integration head `d8ccf0b83e9658e48b137ecc73916886b86bccd4`, the focused correction-round test passed after redirecting `TMPDIR` to `/home/kedarmd/tmp/bodh-build-tmp`; `/tmp` was full and the first isolated rerun failed during linking before the test started.
- The current-head suites passed: `cargo test --locked --test generated_review` (21/21), `cargo test --locked --test complete` (4/4), and `cargo test --locked --test contract` (7/7). Raw output: [generated review](AC47-GENERATED-REVIEW-TESTS-2026-11-07.log), [complete results](AC47-COMPLETE-RESULT-TESTS-2026-11-07.log), and [contract](AC47-CONTRACT-TESTS-2026-11-07.log). The suites cover correction exhaustion/preservation, fresh commits, empty results, follow-up validation, and stale replacement approvals across the public CLI/service seams.
- Independent Standards and Spec reviews of the #47 implementation diff found no standards breach or scope creep. The Spec review confirmed both remaining criteria are still partial: there is no integrated provider-run exhaustion/reporting check and no provider-owned process-residue check. The repo has no provider orchestration runtime. No external review or publication operation was run.
- Therefore #47 remains open and its two unchecked criteria remain unchecked. The passing service/CLI tests do not substitute for the missing provider-level end-to-end evidence.
