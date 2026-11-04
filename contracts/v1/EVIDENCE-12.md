# Issue #12 validation evidence

The full v1 fixture-context validator is implemented as a shared Rust service (`validate_complete`) and a fixture-only CLI command. This freezes semantic checks while leaving production preparation acquisition and opaque preparation IDs for #13.

## Frozen semantic decisions

- IDs for diagrams, participants and every top-level or conditional step are unique within a diagram. Participant and step IDs share one namespace.
- Every interaction that represents code (`message`, `return`, `self_call`) has a required `SourceRef` resolved against the exact pinned base/head commit. Notes can omit a source; when supplied, it must also resolve.
- Returns reference an earlier message or self-call on the same guaranteed flow path and use reversed endpoints. Branch-local and optional calls do not escape their conditional path. Conditional branches cannot be nested. `alt` has at least two nonempty branches with distinct nonblank labels.
- Follow-up assessments require available prior concerns; omitted/null and an available empty array remain distinct. IDs must match actual prior concerns and each concern is assessed at most once. All assessments include an explanation and current pinned source evidence. Resolved items cannot point to current findings; unresolved and regressed items must link to an existing current finding; uncertain items may do so optionally.
- Empty diagrams/findings/follow-up collections remain valid where structurally permitted. Generated coordinates and renderer markup are rejected by the strict contract.

## CLI and side effects

`validate --input <file|-> --complete-fixture --context <fixture.json> --repository <checkout> [--timeout-ms <0..120000>]` invokes the shared complete validator. Success reports `validation: complete_fixture_context`, `semantic_validation_complete: true` and `preparation_authority: fixture_only`. The CLI does not pretend that fixture `context_id` is an opaque preparation ID. Production `validate --prepared <preparation-id>` remains reserved for #13. Validation reads pinned Git objects only; no database, network, GUI, AI, workspace or remote mutation is performed.

The existing envelope and exit categories remain stable: success 0; input/contract/argument failure 2; stale context 3; access/tool unavailable 4; cancellation/deadline 5; internal Git process/read failure 10. Complete fixture errors use the same `ContractError` envelope as structural, evidence and finding validation.

## Bounds and limits

The accepted contract enforces an 8 MiB result/context input cap, 65,536 Unicode scalar values per string, 1,024 items per result/ordinary collection, 16,384 prepared discussion thread IDs, 4,096 diff files, 65,536 diff hunks, 8 MiB Git stdout/blob output and 64 KiB Git diagnostic output. The CLI timeout defaults to 30 seconds and accepts 0–120,000 ms. The 1,024 interaction collection cap is grounded in the 1,000-interaction renderer spike; other numeric bounds are defensive limits informed by the existing contract/acquisition spike. They are not measured latency or memory support claims. Unsupported binary, symlink/tree/gitlink and LFS source content fails explicitly.

## Verification

`./scripts/validate` in core mode passes formatting, check, Clippy, all Rust tests, the fixture/schema agreement tests, actual CLI child-process tests, and native sequence spike tests. The raw clean-run output is preserved in [VALIDATE-12.log](VALIDATE-12.log). The complete semantic fixtures use temporary real Git repositories and verify real pinned commits/blobs. They cover all supported diagram step variants, self-call returns, alternative and optional flow scoping, participant/step collisions, duplicate branch IDs, missing/invalid source references, unavailable/empty prior results, all follow-up statuses, unknown/duplicate concern links, resolution rules, current finding links, CLI success and CLI rejection. Existing contract tests verify schema/Rust/service/CLI agreement, strict unknown-field/version rejection, collection/string/document limits and all-variant fixtures.

The tests demonstrate contract behavior and bounded input rejection. They do not measure the cost of validating maximum-size inputs, establish production acquisition completeness, or claim semantic truth of generated descriptions/findings.
