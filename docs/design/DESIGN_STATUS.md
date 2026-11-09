# Bodh Design Approval Register

**Purpose:** Track approvals for FigCraft design deliverables only. Current overall project stage and authorization are tracked separately in [PROJECT_STATUS.md](../PROJECT_STATUS.md); approved designs do not by themselves authorize implementation.

**Status:** No design gates have been approved.  
**Owner:** User. Agents cannot self-approve.  
**Updated:** 2026-11-08

| Gate | Deliverable | Status | Approval evidence | Open blockers |
| --- | --- | --- | --- | --- |
| 0 | Requirements/design plan + FigCraft capability audit | NOT STARTED | — | — |
| 1 | Design system + component geometry specimen board | NOT STARTED | — | — |
| 2 | Findings master-detail + interaction states | NOT STARTED | — | — |
| 3 | Changes unified/split diff + inline commenting | NOT STARTED | — | — |
| 4 | Finish Review drawer + error states | NOT STARTED | — | — |
| 5 | Overview + descriptions/diagrams/source inspector | NOT STARTED | — | — |
| 6 | Home + repository PR browser | NOT STARTED | — | — |
| 7 | Cross-screen flow/geometry/a11y audit | NOT STARTED | — | — |

## Update protocol

- Set `IN REVIEW` only when editable FigCraft source, exported renders and an honest geometry/flow QA report exist.
- Set `APPROVED` **only after explicit user approval**. Record date and a link/path to the approved revision/frame.
- Report unverified measurements as OPEN or VISUAL-ONLY, never PASS.
- Revising an approved shared token/component must revalidate affected screens; do not silently invalidate previous approvals.
