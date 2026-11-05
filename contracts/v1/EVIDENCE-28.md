# Issue #28 · authorize replacement of unpublished work

Changed generated reviews can now replace local unpublished reviewer work only after an explicit approval. `authorize-replacement` fully validates the candidate, shows the pull request, candidate fingerprint and work counts, and requires an interactive terminal confirmation phrase. It creates a five-minute, one-use grant bound to the preparation, context, workspace revision, candidate, validation context and complete protected-work snapshot. `commit --approval <id>` repeats validation and consumes the grant atomically with deletion of only the captured draft revisions, clearing of active grouped-review intent, generated-result replacement and generation update.

Published draft revisions remain hidden from unpublished counts and retain their publication markers. A changed draft or intent invalidates the approval. Planned, sending or uncertain publication steps block authorization and commit; publication payloads and marker history are retained. Discussion reply and thread-action publication markers are included in the same snapshot and list filtering.

## Verification

`CARGO_BUILD_JOBS=1 ./scripts/validate desktop` passed on merge revision `df75751` with the #28 worktree dirty. The complete output is in [VALIDATE-28.log](VALIDATE-28.log). This includes the contract CLI checks, Rust test suites, native spike checks, GPUI typecheck and Clippy. No GUI was launched and no GitHub writes were performed.

The new regression coverage checks explicit grant and commit, candidate binding, one-use behavior, decline, stale draft revisions, unresolved publication payload preservation, terminal gating and exact discussion marker filtering. Earlier failures are preserved in logs: [missing draft query helper](VALIDATE-28-FIRST-FAILURE.log), [approval work visibility](VALIDATE-28-SECOND-FAILURE.log), [Clippy borrow warning](VALIDATE-28-THIRD-FAILURE.log), [invalid discussion fixture](VALIDATE-28-FOURTH-FAILURE.log), [stale approval classification](VALIDATE-28-FIFTH-FAILURE.log), [spike formatting](VALIDATE-28-SIXTH-FAILURE.log), and [disk exhaustion](VALIDATE-28-SEVENTH-FAILURE.log). Each was corrected before the successful full run.

## Boundaries

Approval is available through the CLI; the native application can consume the shared store service in a later UI slice. No live GitHub writes are performed by authorization or commit.
