# AC-06 live discussion publication fixture — 2026-11-07

## Scope and authorization

The user explicitly authorized a disposable test PR for live-write qualification. PR #54 (`kedarmd/bodh`, title `Bodh qualification fixture`) was created solely for this test on branch `qualification/37-live-fixture-2026-11-07`, from commit `2b5df55e21aa30ad534dfcfec7d868cb0f14d468`. The fixture contained one root file, `BODH-QUALIFICATION-FIXTURE.txt`. All app state was isolated under `/home/kedarmd/tmp/bodh-37-live-fixture-appstate`.

Only clearly labeled test data was sent:

- Inline review comment: `BODH #37 LIVE FIXTURE COMMENT - temporary qualification test; safe to delete.`
- Reply: `BODH #37 LIVE FIXTURE REPLY - temporary qualification test; safe to delete.`
- Thread action: resolve the fixture thread.

PR #54 was closed after verification and its remote fixture branch was deleted. PR #38 and its status were not changed during the live fixture.

## Results

- Bodh acquired the fixture PR, pinned file/source context, and discussion metadata. The thread exposed `viewerCanReply=true` and `viewerCanResolve=true`.
- The inline comment was confirmed through Bodh's grouped review publication flow. Its review operation journal reached `confirmed` with one comment and a remote review ID/URL. No duplicate was sent.
- The reply and resolve were confirmed through Bodh's shared `DiscussionPublicationStore` and `GhDiscussionPublisher`. The persisted plan recorded both steps as `confirmed`, in order: reply then resolve.
- Read-only GitHub verification showed the root comment ID `4500714976`, reply ID `4500752390` with `in_reply_to_id=4500714976`, and the thread's `isResolved=true`.
- The isolated local workspace showed zero unpublished inline-comment drafts after comment publication. The confirmed discussion plan records the reply/action results.
- The disposable PR was closed and its remote branch deletion succeeded.

## Defects found and fixed

1. GitHub's CLI JSON projection used by the review preflight rejected `viewerPermission` as a requested PR field. The preflight now queries GitHub GraphQL and reads `viewerPermission` from `Repository`, where the field is supported. Discussion PR-state inspection also stopped requesting `viewerPermission` from `PullRequest`. Unit fixtures were updated.
2. Live discussion observations expose a GraphQL comment `id` and a numeric REST `databaseId`. The native composer selects the GraphQL ID, while reply publication requires the numeric REST ID. Draft saving now resolves either selected ID to the root comment and persists its numeric REST ID. Regression coverage checks both the native GraphQL selection and direct numeric selection.

The focused regression suites passed after these fixes. The complete `CARGO_BUILD_JOBS=2 ./scripts/validate desktop` run passed on commit `fa49a53`; a fresh desktop run for the follow-up normalization fix is recorded in [the follow-up validation log](VALIDATE-AC06-ROOT-ID-NORMALIZATION-2026-11-07.log). The runner includes formatting, lint/type checks, shared-service tests, CLI checks, GPUI checks and native sequence-spike checks.

## Native draft-only replay

Using a clone of the isolated fixture database (not the closed PR's live state), the native UI rendered the reply editor, accepted clearly marked test text, and saved it locally. The saved reply used numeric root ID `4500714976` and thread ID `PRRT_kwDOVghYd860zl9O`. Finish Review showed the staged reply and COMMENT event and explicitly stated that nothing is sent until confirmation. After closing and reopening the app against the same clone, the selected reply and COMMENT event were restored. The local intent query recorded one selected reply and event `COMMENT`; no publication/confirmation action was activated. Screenshots: [reply editor](../../spikes/native-sequence/evidence/issue-37-ac06-native-reply-editor-2026-11-07.png), [Finish Review draft](../../spikes/native-sequence/evidence/issue-37-ac06-native-finish-review-draft-2026-11-07.png), and [recovered Finish Review draft](../../spikes/native-sequence/evidence/issue-37-ac06-native-finish-review-recovered-2026-11-07.png).

The discussion heading was also adjusted so its Show/Collapse control remains visible beside a short heading, with the filename on a separate line. This addresses the native UI obstruction observed during the replay.

`CARGO_BUILD_JOBS=2 ./scripts/validate desktop` passed on the final local changes; see [native draft validation log](VALIDATE-AC06-NATIVE-DRAFT-2026-11-07.log). The run includes formatting, tests, lint/type checks and native sequence checks.

## Limits

This establishes live endpoint compatibility and the shared publication services' comment/reply/resolve behavior against a disposable PR. The native reply editor, local save, Finish Review selection and restart recovery were exercised against cloned local state only; the native UI did not publish. Native confirmation/reconciliation has not been exercised. AC-06 remains partial. This fixture does not qualify the other ten V1 acceptance criteria or make PR #38 ready to merge.
