# Issue #18 native draft failure and concurrency trial

Date: 2026-11-07 (Asia/Kolkata)

## Setup

- Purpose: verify native local draft create/discard, save failure/retry, and protection against a stale GUI editor overwriting a newer CLI revision.
- Product mode: installed Bodh GUI at `/tmp/bodh-37-native-edit/prefix/bin/bodh-gui`, with installed CLI at `/tmp/bodh-37-native-edit/prefix/bin/bodh`.
- Review context: PR #38, pinned base `7f19442b70d09214421ec3424ef7a80222e96b41`, head `00656c38f15201811f2db87c9445608cad814e6b`; preparation `prep-366a11-18e5942dff71b4fb-0`.
- State: copied isolated database and preferences at `/tmp/bodh-37-ac01-concurrency/state/bodh`; no shared Bodh state database was used.
- GUI: PID `3723415`, Hyprland workspace 2, origin `(-1912, 668)`, size `1904x1020`. Captures are cropped to this target window.
- The installed GUI was built after the #18 draft editor implementation and #37 triage recovery fix. The pinned review head above is the fixture being reviewed, not the build revision. Installation output is in [INSTALL-37-TRIAGE-RESTART-2026-11-07.log](INSTALL-37-TRIAGE-RESTART-2026-11-07.log).

## Outcomes

1. A new local inline draft saved to SQLite; CLI inspection showed revision 1 and workspace revision 7.
2. The GUI reopened that saved draft, edited the body, and saved the edit at revision 2. The updated text is visible in [the native capture](../../spikes/native-sequence/evidence/issue-37-ac18-saved-draft-edited-2026-11-07.png) and [CLI snapshot](AC18-DRAFT-STATE-AFTER-EDIT.json).
3. The GUI typed `GUI dirty text that must survive conflict` without saving. CLI updated the same draft ID using [`AC18-CLI-COMPETING-SAVE-INPUT.json`](AC18-CLI-COMPETING-SAVE-INPUT.json), advancing it to revision 3 and workspace revision 9. The GUI Save action returned `WORKSPACE_REVISION_STALE`; the typed text remained in the editor and the error offered recovery as a new draft. See [before](../../spikes/native-sequence/evidence/issue-37-ac18-stale-editor-before-save-2026-11-07.png) and [rejected save](../../spikes/native-sequence/evidence/issue-37-ac18-stale-save-rejected-text-retained-2026-11-07.png).
4. **Recover as new draft** retained the GUI body and saved it under a distinct draft ID, leaving the CLI's newer revision intact. The evidence is [the UI capture](../../spikes/native-sequence/evidence/issue-37-ac18-recover-as-new-draft-2026-11-07.png) and [CLI snapshot](AC18-DRAFT-STATE-AFTER-RECOVERY.json).
5. **Discard draft** removed the recovered copy. The UI reported that nothing was published; the CLI state returned to two drafts. See [capture](../../spikes/native-sequence/evidence/issue-37-ac18-local-draft-discarded-2026-11-07.png) and [snapshot](AC18-DRAFT-STATE-AFTER-DISCARD.json).
6. A temporary SQLite `BEFORE INSERT` trigger forced an insert failure. The native UI showed `STATE_DATABASE_ERROR: Cannot read or write review state`, retained `native save failure remains in editor`, and stated that entered text remained in the editor. See [failure capture](../../spikes/native-sequence/evidence/issue-37-ac18-save-failure-text-retained-2026-11-07.png).
7. After removing the trigger, the same editor saved successfully; the UI reported local save and the CLI state contains the saved text. See [retry capture](../../spikes/native-sequence/evidence/issue-37-ac18-save-retry-succeeded-2026-11-07.png) and [final snapshot](AC18-DRAFT-STATE-AFTER-SAVE-RETRY.json).

Earlier native creation/restart/resume captures are [here](../../../spikes/native-sequence/evidence/issue-37-ac01-inline-comment-saved-2026-11-07.png), [here](../../../spikes/native-sequence/evidence/issue-37-ac01-inline-comment-reopened-2026-11-07.png), with the reopened CLI state in [AC01-NATIVE-INLINE-COMMENT-REOPENED.json](AC01-NATIVE-INLINE-COMMENT-REOPENED.json).

## Limits

This is a controlled local SQLite fixture, not a live review publication. The comments use a broad Head `1–80` anchor for qualification and are not review findings. No GitHub review, comment, approval, merge, or close was attempted. This closes the missing native paths named by #18's final acceptance item when combined with the existing service tests; it does not complete AC-01 or qualify the full #37 V1 workflow.
