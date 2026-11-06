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
