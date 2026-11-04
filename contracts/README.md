# Bodh review contract (v1 fixture validation)

Issues #9–#12 freeze the v1 result shape and implement complete structural and semantic validation against a caller-supplied fixture preparation. Issues #13–#14 add headless prepare/inspect, SQLite persistence, and safe acquisition of exact base/head commits, including fork heads. Preparation selects an explicit matching DNS-host remote and supports advertised base branches, GitHub-style `refs/pull/<number>/head`, and explicit fork branch refs; it rejects shallow histories, moved/unavailable refs and non-unique merge bases. Discussions and previous results remain explicitly unavailable; see [#13 evidence](v1/EVIDENCE-13.md) and [#14 evidence](v1/EVIDENCE-14.md).

Run from the checkout:

```sh
cargo run --locked -- --json contract describe
cargo run --locked -- --json contract schema
cargo run --locked -- --json validate --input contracts/v1/fixtures/all-variants.json --structural
cargo run --locked -- --json validate --input - --structural < contracts/v1/fixtures/empty.json
cargo run --locked -- --json validate --input result.json --complete-fixture --context fixture.json --repository /path/to/checkout
./scripts/validate
```

`v1/review.schema.json` is the canonical generated JSON Schema 2020-12 artifact. The shared `validate_structure` service validates it and then parses the strict Rust types; direct Rust deserialization also rejects unknown fields and per-field size excesses. A regression check rejects any committed schema drift. Regenerate the artifact by taking the `data` object from `bodh contract schema`; the schema command uses the same generator as validation. No schema network resolution is enabled.

All required result sections are present even when their collections are empty: `context`, `overview`, `diagrams`, `findings`, `follow_up`, `coverage`. `schema_version` is exactly `1.0`. Every object rejects unknown fields. See the complete fixture and schema for exact fields and enums. Optional fields are nullable or omitted: participant `category`, step `source`, follow-up `finding_id`. All other fields are required, including arrays that may be empty. Source references use `revision: head|base`, repository-relative `path`, and inclusive `start_line`/`end_line`; `base` will mean the merge-base, never latest target tip. Reading references and publication-eligible anchors remain distinct.

Steps are tagged `message`, `return`, `self_call`, `note`, `alt` or `opt`. Conditional children use the nonconditional union, so nested conditionals are structurally rejected. Message, return and self-call steps require a `SourceRef` that resolves to a regular UTF-8 blob at the exact pinned base/head commit; notes may omit source. Diagram, participant and step IDs share a diagram-local uniqueness scope. Participant references must resolve. Returns link to an earlier message or self-call on that exact execution path and reverse its endpoints; branch-local calls do not escape their branch. `alt` requires at least two branches with nonblank labels; conditional children may not nest. Findings have severity separate from `high|medium|low` confidence, evidence, primary/supporting locations, alternatives and discussion relationships. Alternatives are tagged `text_comment` or `code_replacement`; replacement text stays structured data. Follow-up status is `resolved|unresolved|regressed|uncertain`: every assessment names one actual prior concern exactly once, includes an explanation and current evidence, and resolved assessments cannot name a current finding. Unresolved/regressed assessments must link to an existing current finding; uncertain may link to one optionally. Prior concern availability is explicit (`null`/omitted means unavailable, `[]` means successfully acquired and empty). Coverage status is `passed|failed|not_run|unavailable`. The application owns layout and change cues. Generated coordinates/markup are not part of the strict schema. Structural or semantic acceptance of a fixture is not evidence that its illustrative claims are true.

## Resource policy

- Encoded input: at most **8 MiB**, checked before parsing. CLI reads at most the limit plus one byte, then rejects; it never silently truncates accepted content.
- Every free string: at most **65,536 Unicode scalar values** (`maxLength` in JSON Schema), including strings inside arrays and nullable fields. Rust counts characters rather than UTF-8 bytes to match schema semantics.
- Every result and ordinary nested collection: at most **1,024 items** (`maxItems`). The accepted renderer spike exercised 1,000 interactions. Prepared discussion thread IDs separately allow **16,384** IDs to match the planned acquisition envelope; discussion failure lists remain capped at 1,024. These bounds constrain supplied data, not remote acquisition completeness.
- JSON parsing retains serde_json's recursion limit (128 nested parser levels). The finite schema vocabulary has no recursive conditional/layout escape hatch. Byte, string and collection limits bound accepted complexity; they are resource policies, not latency/memory guarantees.

The 1,000-step native fixture motivates the 1,024-item interaction cap. The 8 MiB input and 65,536-character string caps are conservative engineering bounds, not demonstrated latency or memory support guarantees. The 16,384 discussion-ID allowance matches the planned acquisition envelope and is likewise a limit, not a performance claim. Larger results fail explicitly. Arbitrary graph/layout code, binary source, LFS, Enterprise support and publication acceptance are not inferred from these limits.

## CLI protocol and continuation contract

`--json` may appear before or after command arguments, once. All current output already uses JSON envelopes, including without that flag. Stdout contains exactly one JSON object and newline; stderr contains diagnostics. Success: `{"protocol_version":"1.0","ok":true,"data":...}`. Failure: `{"protocol_version":"1.0","ok":false,"error":{"code":"...","message":"...","pointer":"..."}}`. Errors may add one-based `line` and `column` for malformed JSON. A pointer is an RFC 6901 instance location (empty means the document or a strict-parser error); schema union failures may point at the containing object. Messages do not echo submitted content, credentials or command arguments.

Current symbolic failures: `INVALID_ARGUMENTS`, `INPUT_UNREADABLE`, `INPUT_TOO_LARGE`, `MALFORMED_JSON`, `UNSUPPORTED_SCHEMA_VERSION`, `CONTRACT_INVALID`, plus evidence/semantic errors in their service modules. File input `-` means stdin. UTF-8 command arguments are required. A stage flag is required. `--structural`, `--evidence-only` and `--findings-only` report partial completion. `--complete-fixture` runs all contract checks against the supplied fixture context and reports `validation: complete_fixture_context`, `semantic_validation_complete: true` and `preparation_authority: fixture_only`. It does not resolve an opaque preparation ID or make the fixture authoritative for a live PR.

Exit categories reserved across CLI operations:

| Exit | Meaning |
| --- | --- |
| 0 | Success |
| 2 | Input/contract/arguments |
| 3 | Stale context/generation/draft state |
| 4 | Authentication/access |
| 5 | Network/uncertain outcome |
| 10 | Internal failure |

Structural and complete fixture validation use 0/2 for success/input-contract errors. Evidence validation also uses 3 for context mismatch, 4 for unavailable Git/tool/objects, 5 for cancellation/deadline and 10 for process/read failures. Unsupported content, bounds and invalid evidence remain input/contract failures (2). Later adapters must preserve these categories.

The continuation vocabulary includes `prepare`, `inspect`, production `validate`, `commit`, `authorize-replacement` and optional `open`. Fixture-frozen transport:

```text
prepare --repo <checkout> --pr <number> --state <database> [--remote <name>]
inspect --prepared <preparation-id> --kind <manifest|description|files|diff|source|discussions|previous> --state <database> [...locator flags]
validate --prepared <preparation-id> --input <file>
commit --prepared <preparation-id> --input <file|-> [--approval <grant-id>]
authorize-replacement --prepared <preparation-id> [...human interaction]
open --repo <checkout> --pr <number>
```

`prepare --repo <checkout> --pr <number> --state <database> [--remote <name>]` requires exactly one matching supported DNS-host remote unless selected explicitly. It uses installed `gh` for read-only REST metadata and Git credential-helper access. It fails on shallow history, unavailable or moved pinned objects, and ambiguous merge bases; it never substitutes a newer tip. Exact acquired commits are retained under Bodh-owned refs without checkout, branch, index or worktree changes. Each invocation stores its own preparation ID and metadata snapshot with an expected generation; deterministic SHA-256 context IDs identify immutable comparison inputs. `inspect` requires the same explicit `--state` path. Files are returned in pages of at most 100; diff patches are returned in 64 KiB UTF-8 pages for an indexed path; source reads use the stored checkout and pinned context. Discussions and previous results report unavailable. Host syntax and `--hostname` routing are tested locally; no live Enterprise compatibility is claimed. Publication and replacement authorization remain later work.

## Executable evidence

`cargo test --locked` exercises strict schema/Rust/service/CLI acceptance of the empty and all-variant fixtures, every object location's unknown-field rejection, unsupported versions/unions/enums/required fields, exact size boundaries, JSON/file/argument errors and protocol envelopes. The CLI tests use actual child processes. `./scripts/validate` additionally runs formatting, check/Clippy and existing spike checks. See `v1/EVIDENCE.md` for the recorded run and scope.

## Immutable evidence fixture workflow — #10

The public shared `validate_evidence(input, prepared, repository, control)` service extends structural validation with exact context binding, normalized PR identity, commit/merge-base consistency, overview evidence, exact original-description quote occurrences and explained coverage. `read_source(repository, context, source, control)` returns exact UTF-8 commit-blob lines with their original LF/CRLF delimiters; an absent final newline creates no extra empty line. Unchanged source can be read even when it is outside an eligible diff hunk; reading is not permission to publish there.

```sh
cargo test --locked --test evidence
bodh --json validate --input result.json --evidence-only --context fixture-context.json --repository /path/to/checkout
```

The second command needs a **trusted fixture**, not an agent-authored authority for a real PR. Its strict context file is `{ "context": <the exact ReviewContext object>, "original_description": <original text or null> }`; `original_description` may be omitted. This service does not acquire/verify remote PR authority or derive a production context hash. It verifies full lowercase SHA-1/SHA-256 IDs identify actual commits and that the stated merge-base is the unique merge-base of pinned base/head. Production acquisition/persistence and `--prepared <id>` follow in #13; this fixture command never modifies active review state. `context_id` is a binding to the supplied fixture, not a concurrency token. Exact quotes use case-sensitive contiguous original text and one-based character-boundary occurrences (including overlapping occurrences).

PR host/owner/repository are normalized lowercase ASCII identifiers and PR number is positive. Host is a DNS-style sequence, owner uses letters/digits/hyphens, repository additionally permits `_`/`.`. Host/API compatibility is not proven by identifier parsing. Missing commits fail explicitly; shallow histories and a non-unique/mismatched merge-base are rejected. No fallback to dirty source or latest target occurs. Git replace refs, legacy grafts and ambient `GIT_*` redirection are ignored. Git argv uses literal pathspecs, immutable object IDs, no shell and no working-tree/index operation; no diff/textconv hook is invoked by blob reads.

Only regular UTF-8 blobs have supported source lines. Binary NUL, invalid UTF-8, symlink/tree/gitlink entries and LF/CRLF LFS pointers fail explicitly. Absolute, traversal, empty, backslash and control-character paths fail; literal spaces and Git pathspec metacharacters are supported. Positive inclusive line ranges must exist in the exact blob. Each Git stdout stream is capped at 8 MiB; diagnostic streams at 64 KiB. Source bounds and supported content are provisional policies, not performance guarantees. Original description uses the existing 65,536-character bound.

`OperationControl` is cloneable; cancellation of one clone stops the operation. It defaults to a 30-second overall deadline, checked before/during Git calls and before return. The CLI permits `--timeout-ms <0..120000>` for bounded fixtures. Git runs in a dedicated Unix process group; a cancellation/deadline kills the group, reaps its leader and drains its pipes. A parent that exits while descendants retain pipes cannot evade that deadline. Cancellation through this service does not establish GUI or terminal interrupt handling; later orchestration must bind its cancellation input to the control. This Linux-native implementation uses Unix process groups.

`--findings-only` extends fixture validation with anchor, suggestion and thread checks; `PreparedEvidenceContext.discussions` carries `{ "thread_ids": [...], "complete": bool, "failures": [...] }`. Omitted discussion data means incomplete/unacquired. A partial observation may link an observed thread; an unknown thread ID fails with an explicit incomplete-observation error. A complete observation with an absent ID fails as an invalid reference. `PreparedEvidenceContext.prior_concerns` uses `null`/omission for unavailable and an array (including empty) for an acquired prior result. The application must acquire and accurately report all discussions and prior findings before claiming completeness. No GitHub mutation is implemented here. Review-result schema/types are unchanged by #12. See [#10 evidence](v1/EVIDENCE-10.md), [#11 evidence](v1/EVIDENCE-11.md) and [#12 evidence](v1/EVIDENCE-12.md).
