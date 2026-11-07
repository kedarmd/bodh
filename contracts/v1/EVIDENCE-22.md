# Issue #22 · publish code replacement suggestions

Finish Review can select saved code replacements alongside inline comments. The confirmed review plan stores each replacement's exact source, original text, replacement text, draft ID/revision, and GitHub suggestion Markdown. The publisher sends those suggestions as review threads. Before sending, it rechecks the PR head and permission, validates the pinned head-side anchor and exact source text, and verifies the persisted draft revision. Only a confirmed send marks a replacement revision published; rejected and uncertain outcomes keep it available for reconciliation.

The native Finish Review screen shows replacement drafts, lets users select or unstage them, links back to Changes for editing, and displays the exact suggestion body in the plan. It refreshes replacement counts after confirmed publication and distinguishes a pre-send validation failure from a GitHub rejection or uncertain outcome.

## Verification

- `CARGO_BUILD_JOBS=2 cargo test --locked --test manual_drafts --test manual_review`: 15 + 9 passed. `manual_drafts` covers multiline replacement ranges, renamed targets, changed original text, deleted head-side targets, malformed ranges, binary/unsupported spans and unchanged source/index bytes. `manual_review` publishes an exact multiline suggestion through fake `gh` and asserts body, start/end lines and side; its head-race case now includes a selected replacement and confirms a moved head leaves the operation planned and unsent.
- `./scripts/validate desktop`: passed on the merged integration tip `9c1d6c1` plus the #22 changes. The complete runner output is in [VALIDATE-22.log](VALIDATE-22.log).
- The generated-review stage fixture now supplies an empty `selected_replacements` list for the extended review request type.
- `cargo fmt --all -- --check` and `git diff --check` passed after these fixture additions.
- No live GitHub write or interactive GUI trial was performed.

## Boundaries

Suggestion Markdown is created by the application from stored replacement text; callers cannot provide arbitrary suggestion bodies. Code replacements must target exact pinned head-side source. A send that is rejected or whose outcome is unknown never marks the replacement as published, so it remains available for reconciliation.
