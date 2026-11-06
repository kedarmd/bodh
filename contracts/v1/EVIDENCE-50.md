# Issue #50 evidence: publication outcome recovery

## Implemented

- Finish Review loads full review operation and discussion plan histories through public store APIs after restart.
- Each operation/step is presented with its persisted state and payload context. Unknown outcomes have read-only reconciliation actions; these actions inspect remote state and do not send publication mutations.
- Confirmed remote review and discussion objects expose links only when the persisted state is confirmed and the host matches the expected GitHub host. Already-achieved thread actions remain unattributed.
- The UI suppresses a new confirmation action for an unchanged review payload that is already confirmed.
- Store history APIs are read-only and return deterministic oldest-first histories.

## Executed validation

- `cargo fmt --manifest-path Cargo.toml`
- `cargo fmt --manifest-path spikes/native-sequence/Cargo.toml`
- `git diff --check`
- `CARGO_BUILD_JOBS=2 cargo test --locked --test publication_history` — 2 passed, including reopening persisted failed review and discussion records with their payload/state.
- `CARGO_BUILD_JOBS=2 cargo test --locked --manifest-path spikes/native-sequence/Cargo.toml --bin bodh-sequence-spike publication_recovery_ui_tests` — 2 passed.

## Not verified

- Native GUI/pointer/keyboard trial and visual acceptance were not run. Project `TESTING.md` requires coordinating GUI automation with the user.
- No live GitHub writes were performed.
