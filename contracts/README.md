# Bodh review contract (provisional v1)

Issue #9 delivers **structural validation only**. It does not certify source evidence, claims, publication anchors, scoped references, prepared state or semantic truth. Those checks arrive in #10–12; the complete v1 agreement gate remains open. This package launches no GUI, daemon, AI or Git/gh process, and writes no workspace or remote state.

Run from the checkout:

```sh
cargo run --locked -- --json contract describe
cargo run --locked -- --json contract schema
cargo run --locked -- --json validate --input contracts/v1/fixtures/all-variants.json --structural
cargo run --locked -- --json validate --input - --structural < contracts/v1/fixtures/empty.json
./scripts/validate
```

`v1/review.schema.json` is the canonical generated JSON Schema 2020-12 artifact. The shared `validate_structure` service validates it and then parses the strict Rust types; direct Rust deserialization also rejects unknown fields and per-field size excesses. A regression check rejects any committed schema drift. Regenerate the artifact by taking the `data` object from `bodh contract schema`; the schema command uses the same generator as validation. No schema network resolution is enabled.

All required result sections are present even when their collections are empty: `context`, `overview`, `diagrams`, `findings`, `follow_up`, `coverage`. `schema_version` is exactly `1.0`. Every object rejects unknown fields. See the complete fixture and schema for exact fields and enums. Optional fields are nullable or omitted: participant `category`, step `source`, follow-up `finding_id`. All other fields are required, including arrays that may be empty. Source references use `revision: head|base`, repository-relative `path`, and inclusive `start_line`/`end_line`; `base` will mean the merge-base, never latest target tip. Reading references and publication-eligible anchors remain distinct.

Steps are tagged `message`, `return`, `self_call`, `note`, `alt` or `opt`. Conditional children use the nonconditional union, so nested conditionals are structurally rejected. Code source is optional to support non-code notes/context; #12 checks when real source is necessary and validates references/branch counts. The application owns layout and change cues. Findings have severity separate from `high|medium|low` confidence, evidence, primary/supporting locations, alternatives and discussion relationships. Alternatives are tagged `text_comment` or `code_replacement`; replacement text stays structured data. Follow-up status is `resolved|unresolved|regressed|uncertain`. Coverage status is `passed|failed|not_run|unavailable`. Structural acceptance of a fixture is not evidence that its illustrative claims are true.

## Resource policy

- Encoded input: at most **8 MiB**, checked before parsing. CLI reads at most the limit plus one byte, then rejects; it never silently truncates accepted content.
- Every free string: at most **65,536 Unicode scalar values** (`maxLength` in JSON Schema), including strings inside arrays and nullable fields. Rust counts characters rather than UTF-8 bytes to match schema semantics.
- Every collection: at most **1,024 items** (`maxItems`). The accepted renderer spike exercised 1,000 interactions; this permits that fixture size without requiring fabricated minimum content. These bounds also apply to findings/evidence/discussions referenced in results, not remote acquisition completeness.
- JSON parsing retains serde_json's recursion limit (128 nested parser levels). The finite schema vocabulary has no recursive conditional/layout escape hatch. Byte, string and collection limits bound accepted complexity; they are resource policies, not latency/memory guarantees.

The 1,000-step native fixture and Git/gh spike's explicit unsupported large-content behavior motivate bounded input rather than unlimited acceptance. The 8 MiB/65,536 text caps are conservative provisional engineering choices, not experimentally demonstrated production support limits. #12 must qualify/freeze them alongside semantic fixtures; release measurement can impose lower operational targets with evidence rather than silently changing accepted data. Larger results fail explicitly. Arbitrary graph/layout code, binary source, LFS, Enterprise support and publication acceptance are not inferred from these limits.

## CLI protocol and continuation contract

`--json` may appear before or after command arguments, once. All current output already uses JSON envelopes, including without that flag. Stdout contains exactly one JSON object and newline; stderr contains diagnostics. Success: `{"protocol_version":"1.0","ok":true,"data":...}`. Failure: `{"protocol_version":"1.0","ok":false,"error":{"code":"...","message":"...","pointer":"..."}}`. Errors may add one-based `line` and `column` for malformed JSON. A pointer is an RFC 6901 instance location (empty means the document or a strict-parser error); schema union failures may point at the containing object. Messages do not echo submitted content, credentials or command arguments.

Current symbolic failures: `INVALID_ARGUMENTS`, `INPUT_UNREADABLE`, `INPUT_TOO_LARGE`, `MALFORMED_JSON`, `UNSUPPORTED_SCHEMA_VERSION`, `CONTRACT_INVALID`. File input `-` means stdin. UTF-8 command arguments are required. The `--structural` flag is mandatory until complete validation is implemented, preventing a caller from mistaking structure acceptance for completed review validation. Success explicitly reports `validation: structural_only` and `semantic_validation_available: false`.

Exit categories reserved across CLI operations:

| Exit | Meaning |
| --- | --- |
| 0 | Success |
| 2 | Input/contract/arguments |
| 3 | Stale context/generation/draft state |
| 4 | Authentication/access |
| 5 | Network/uncertain outcome |
| 10 | Internal failure |

Only 0/2 occur in this slice. The remaining categories must be used by later shared-service adapters rather than reporting their failures as success or inventing adapter-specific exit numbers.

The continuation vocabulary reserves `prepare`, `inspect`, full `validate`, `commit`, `authorize-replacement` and optional `open`. Planned transport, to fixture-freeze at #12:

```text
prepare --repo <checkout> --pr <number> [--remote <name>]
inspect --prepared <preparation-id> --kind <manifest|description|files|diff|source|discussions|previous> [...locator flags]
validate --prepared <preparation-id> --input <file|->
commit --prepared <preparation-id> --input <file|-> [--approval <grant-id>]
authorize-replacement --prepared <preparation-id> [...human interaction]
open --repo <checkout> --pr <number>
```

These commands are reserved, not implemented or advertised as available. Repository/host selection and source locator flags must remain explicit where ambiguous. Prepare must return an opaque preparation ID with the expected workspace generation and draft revision bound to that individual preparation. The result's deterministic `context.context_id` is separate and cannot act as a concurrency token. Later `validate`/`commit` use `--prepared` to retain that binding; repeated prepares cannot overwrite another preparation's expected generation. Human grants bind the same context/revisions with expiry and one-time use; no `--force` may fabricate approval. State-location options and exact inspect locators remain part of #12's agreement gate.

## Executable evidence

`cargo test --locked` exercises strict schema/Rust/service/CLI acceptance of the empty and all-variant fixtures, every object location's unknown-field rejection, unsupported versions/unions/enums/required fields, exact size boundaries, JSON/file/argument errors and protocol envelopes. The CLI tests use actual child processes. `./scripts/validate` additionally runs formatting, check/Clippy and existing spike checks. See `v1/EVIDENCE.md` for the recorded run and scope.
