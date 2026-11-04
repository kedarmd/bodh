# Structural contract slice — issue #9

Recorded 2026-11-03 at `282c06f` with a clean worktree, using pinned Rust/Cargo 1.99.0 on Linux. [Raw validation output](validation.log) records exact system/tool versions, source revision and command results. Headless reproduction:

```sh
./scripts/validate
```

The recorded invocation additionally used `CARGO_BUILD_JOBS=2` and `CARGO_TARGET_DIR=/home/kedarmd/dev/bodh/spikes/native-sequence/target` to reuse dependency builds. Application and spike have distinct Cargo package identities; no performance comparisons were performed. The root runner reports both application and spike targets. No GUI, daemon, AI call or GitHub request ran for this slice.

Passed: five runner regression tests; five Git/gh spike behavior tests; application format/check/Clippy; ten application integration tests (three actual CLI-process tests, seven contract-service/corpus tests); existing spike format/check/Clippy and all headless tests. The contract corpus includes empty results and every supported structural union/enum, nullable/omitted options, unknown fields at each object location, malformed JSON, invalid versions/unions/missing required fields, exact UTF-8 character/collection boundaries and oversized bytes. Valid fixtures and negative mutations pass through canonical schema, strict Rust parsing and shared validation; fixture/negative corpus and envelope checks also invoke the actual CLI. The committed schema is compared to the generated schema.

Red → green observations retained in the session's execution history:

- The first public service test failed because the `bodh` library did not exist; adding shared types/schema/service made the empty-result workflow pass.
- Oversized-string/collection tests initially failed because the service accepted them; schema bounds and strict per-field Rust deserializers now reject them, preserving exact boundary input without truncation.
- Unknown schema-version test expected `UNSUPPORTED_SCHEMA_VERSION` but observed generic `CONTRACT_INVALID`; version discovery now returns the dedicated code and `/schema_version` pointer.
- Malformed-input CLI test initially had no line/column; the protocol now reports parser location without echoing untrusted input.
- Non-Unicode argv test initially observed a Rust panic (exit 101); CLI parsing now emits one `INVALID_ARGUMENTS` envelope and exit 2.
- Updated runner tests initially observed only the spike's four Cargo commands; the runner now validates both application and spike and still stops immediately on a failed lint.

This file records the original #9 structural slice; its scope statements are historical. Issues #10–#12 subsequently added immutable evidence, finding/anchor and complete diagram/follow-up fixture validation. The current frozen fixture contract and clean complete validation run are recorded in [#12 evidence](EVIDENCE-12.md) and [its raw runner log](VALIDATE-12.log). Production preparation persistence, authorization consumption and remote publication are not implemented by these slices. No hosted CI result is claimed. The accepted spikes establish only their recorded Linux/github.com compatibility; these slices add no Enterprise, distro, GPU/input performance or publication guarantee.
