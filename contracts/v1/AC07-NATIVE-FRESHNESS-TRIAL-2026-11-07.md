# Issue #37 AC-07 — native freshness replay (2026-11-07)

## Setup and boundary

The installed native app used an isolated copy of prepared PR #38 state at `/home/kedarmd/tmp/bodh-37-ac07-freshness/bodh/reviews.sqlite`. The stale preparation was `prep-9f5d2-18e5ab3c90655a77-0`, with head `fcf2cba63b8dc3d99ad32b0b0e199197f232e81e`; the current GitHub head observed by the UI was `f1dd7f58246ed5dfaa60b0216df260e0a0ed4470`. No review/comment was published and no checkout/index was edited.

## Observations

- Resuming the old preparation showed `PR changed on GitHub · head moved: true, base moved: false`; see [stale banner](../../spikes/native-sequence/evidence/issue-37-ac07-stale-banner-2026-11-07.png).
- A clearly marked local inline draft (`AC07 LOCAL STALE ANCHOR DRAFT`) was saved on the old prep at `src/freshness.rs`, Head lines 1–80. The native **Load Latest** readiness message explicitly said `1 pending work item(s) stay on the current review`; see [warning capture](../../spikes/native-sequence/evidence/issue-37-ac07-pending-load-latest-2026-11-07.png). After loading latest, the old draft remained in its old preparation and was visible as earlier-revision work from the new preparation.
- A second clearly marked draft, `AC07 RELOCATION PROPOSAL DRAFT`, was then stored in the old preparation at `spikes/native-sequence/src/product.rs`, Head line 7381. That exact line moved to 7401 on the new head. The old prep retains both drafts, confirmed by `draft list` readback.

## Limits

The native UI did not yet display the new preparation's earlier-draft recovery panel or reach **Confirm proposed relocation**. Therefore this trial establishes freshness detection, pending-work warning, latest-preparation switching, and preservation of drafts on the old preparation only. It does not establish relocation confirmation, reattachment, changed/ambiguous anchor behavior in the native UI, live publication, or the broader AC-07 matrix. AC-07 remains partial; #37 remains open and PR #38 remains draft/not merge-ready.
