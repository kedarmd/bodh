# Issue #23 implementation evidence

## Delivered

- Finish Review includes explicit reply and resolve/reopen selections alongside grouped comments and code replacements. Discussion-only plans do not invent an empty review event.
- The durable discussion journal stores each reply/action payload and outcome independently. A linked grouped review must be confirmed first; replies are attempted before thread actions.
- Before each request, the app fetches the current PR head/base/open state and the thread’s current permissions/resolution state. A changed draft revision or denied permission remains unsent; an action already achieved remotely is recorded as `already_achieved` and never receives a published marker.
- Each request is journaled as sending before transport. Confirmed requests alone receive exact draft-revision markers. Rejected, unknown and unsent steps remain visible, and an interrupted send becomes unknown without an automatic retry.
- Reply transport uses the numeric ID of the top-level review comment. Resolve/reopen uses the thread node ID. All validation fixtures use a controlled publisher and perform no live GitHub writes.

## Verification

- `CARGO_BUILD_JOBS=1 ./scripts/validate desktop` passed in the combined #23/#28 tree. The final shared run is recorded at merge revision `df7575112a85c0c1ecc507facbac8a43f42178f9` with #28 changes dirty in its worktree: [VALIDATE-28.log](VALIDATE-28.log).
- The four `tests/discussion_publication.rs` cases pass for independent outcomes and exact reply payload, permission loss/already-achieved status, local draft races, and interrupted sends.
- An earlier standalone compile stopped on a moved-plan ownership error. The fix is committed in #23 and included in the combined successful run.
- No native GUI trial was performed. No live GitHub writes were performed.
