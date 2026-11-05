# Issue #22 · publish code replacement suggestions

Finish Review can select saved code replacements alongside inline comments. The confirmed review plan stores each replacement's exact source, original text, replacement text, draft ID/revision, and GitHub suggestion Markdown. The publisher sends those suggestions as review threads. Before sending, it rechecks the PR head and permission, validates the pinned head-side anchor and exact source text, and verifies the persisted draft revision. Only a confirmed send marks a replacement revision published; rejected and uncertain outcomes keep it available for reconciliation.

The native Finish Review screen shows replacement drafts, lets users select or unstage them, links back to Changes for editing, and displays the exact suggestion body in the plan. It refreshes replacement counts after confirmed publication and distinguishes a pre-send validation failure from a GitHub rejection or uncertain outcome.

## Verification

- `cargo test --locked --test manual_review`: 9 passed, including exact suggestion Markdown, target path/line/side, fake-`gh` request body, and removal only after confirmed publication.
- `./scripts/validate desktop`: passed on the merged integration tip `9c1d6c1` plus the #22 changes. The complete runner output is in [VALIDATE-22.log](VALIDATE-22.log).
- The generated-review stage fixture now supplies an empty `selected_replacements` list for the extended review request type.
- No live GitHub write or interactive GUI trial was performed.

## Boundaries

Suggestion Markdown is created by the application from stored replacement text; callers cannot provide arbitrary suggestion bodies. Code replacements must target exact pinned head-side source. A send that is rejected or whose outcome is unknown never marks the replacement as published, so it remains available for reconciliation.
