# Issue #26 — explicit stale-anchor repair evidence

## Behavior

- Stale text and replacement drafts receive explicit revalidation states for changed, removed, unsupported, ambiguous, and eligible unchanged spans.
- A relocation is proposed only when the exact old text has one eligible match. The proposal shows the previous and proposed source; the reviewer must confirm it. Duplicate or changed text is never moved automatically.
- Matching searches eligible changed ranges across the new path when Git does not report a rename. This supports moved and renamed source while retaining side/range checks.
- Reattachment creates a new draft and preserves the original. The persisted repair linkage makes repeating the action idempotent. Revalidation and workspace revisions reject stale concurrent writes.
- The native Changes inspector exposes repair proposals and explicit manual reattachment. The pinned source keeps the exact selected line as the draft anchor while showing nearby context; long code wraps, the source toolbar stays fixed during vertical scrolling, and narrow windows can horizontally reach each review pane.

## Verification

- `CARGO_BUILD_JOBS=1 ./scripts/validate desktop` passed after the final native UI changes. This covers runner tests, formatting, core/native checks, Clippy, and all locked test suites. Full output: [VALIDATE-26.log](VALIDATE-26.log).
- `CARGO_BUILD_JOBS=1 ./scripts/validate gui` built and launched the release native app after the final UI changes. Build and trial log: [GUI-26.log](GUI-26.log). One earlier build attempt failed on a conditional return type in the narrow-window wrapper; that code was corrected, and subsequent GUI and desktop validation passed.
- Focused Git/SQLite integration tests cover unchanged movement requiring confirmation, changed/duplicate text rejection, replacement reattachment preserving the original, renamed-file lookup, stale revision/concurrent writes, and exact source spans.
- The user reviewed the pinned-source context, long-line wrapping, fixed toolbar, and narrow-window pane boundaries and approved the final layout on 2026-11-05.

## Remaining acceptance evidence

The screenshots and user feedback confirm the native layout. The 2026-11-07 AC-07 isolated GUI replay directly observed the stale-head banner, the **Load Latest** warning that pending work stays on the current review, and that the old preparation's two local drafts remain attached to its old head after switching. It did not yet reach **Confirm proposed relocation** or **reattach here** in the native UI; those interactions remain open. The freshness observation used a copied local database and is not evidence of an external GitHub write.
