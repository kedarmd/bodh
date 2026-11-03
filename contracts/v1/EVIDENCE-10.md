# Immutable evidence validation — issue #10

Recorded 2026-11-03 at clean revision `273db475eb1c6bdc8eb7b50652e236583cf82b7f`. [Raw core runner output](validation-10.log) records Linux, pinned Rust/Cargo 1.99.0, revision, target directories and results. Reproduce with `./scripts/validate`; the recorded run used `CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/home/kedarmd/dev/bodh-worktrees/ticket-9/target` to reuse dependency builds. Application and renderer spike have distinct Cargo package identities. No timing/performance comparison was attempted.

Passed: five runner tests, five acquisition-spike behavior tests, application format/check/Clippy, all twenty application integration tests (ten new shared-service/CLI evidence tests), and existing renderer headless format/check/Clippy/tests. No GUI, gh request, remote mutation, daemon, workspace persistence or replacement ran. Hosted CI is not claimed.

The new public-service fixtures create real temporary Git histories with independently advanced target and head. They read base from the merge-base, preserve CRLF and missing-final-newline bytes, and compare HEAD, symbolic branch, raw index, dirty tracked file and untracked file before/after. Valid reading includes unchanged source; no inline publication eligibility is inferred.

Positive/negative fixtures cover exact context binding, canonical PR identity, missing commits, mismatched merge-base, shallow history, exact description quotes and one-based occurrence, source-backed overview evidence and explained coverage. Unsupported/malicious source cases include absolute/traversal/backslash/empty components, missing committed entries despite local files, invalid ranges, symlinks, binary NUL, invalid UTF-8, CRLF LFS pointers, empty blobs and oversized committed content. Literal Git pathspec metacharacters resolve the intended entry. Actual CLI processes test hostile `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE` and `GIT_CONFIG_COUNT`; actual replace refs and legacy grafts cannot substitute source or ancestry.

Cancellation/deadline fixtures preserve checkout/index, reject an already cancelled service control, enforce an immediate deadline, and run a controlled Git process whose parent exits while a descendant retains output pipes. The timeout kills the dedicated group, reaps the leader and returns protocol exit 5. The CLI also demonstrates stale context exit 3 and missing Git exit 4; input/contract errors remain exit 2. Messages do not expose raw Git diagnostics.

Red → green observations:

- The first service test failed because immutable source/preparation APIs did not exist; pinned commit-blob resolution made the real-Git dirty-checkout fixture pass.
- Canonical identity was initially accepted despite uppercase preparation identity; canonical preparation validation now rejects it and verifies all commits plus the actual unique merge-base.
- An invalid case-sensitive original quote was initially accepted; exact original-text/occurrence checking now rejects it. A source range error inside an intent assessment has the exact JSON pointer.
- Empty source evidence and unexplained coverage were initially accepted; semantic content checks now require source-backed key changes and nonempty explanations.
- Cancellation/timeout APIs were initially absent. The first deadline implementation then waited for a descendant's inherited pipes beyond the deadline; the failing process test now passes because polling includes pipe-reader completion.
- The actual CLI initially rejected the evidence workflow as invalid arguments; it now routes to the shared service and reports explicitly partial success.
- Shallow history initially surfaced a generic missing-merge-base failure; it now produces actionable `GIT_HISTORY_UNSUPPORTED` before interpreting a partial graph.

This slice does not verify finding relationships, diff anchors, replacement eligibility, diagram semantics or follow-up concerns. Review schema/types remain unchanged. `--evidence-only --context` is a trusted fixture transport, not authoritative remote PR preparation or a concurrency binding. Full semantic acceptance/freeze remains #11–12; production acquisition/persistence remains #13. Coverage checks validate enums/explanations and do not certify execution or semantic truth. Linux/Unix process groups are the tested execution environment; Enterprise, publication and additional host/distro support remain unverified. See [contract guidance](../README.md) for precise caps, cancellation boundaries and unsupported content.
