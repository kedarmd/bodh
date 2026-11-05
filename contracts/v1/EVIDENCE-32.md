# Issue #32 · follow-up concerns

## Implemented

- Atomic result replacement keeps one current full result and derives a compact prior-concern index containing each concern's ID, title, summary, and source explanation with the exact commit SHA. The existing generation transaction replaces the result and compact index together.
- The native Findings view maps validated follow-up statuses to prior concerns and current findings. Unresolved, regressed, uncertain, and unassessed concerns appear before resolved concerns; resolved rows are compact and can expand to show evidence. Current findings are marked new when the prior index is available and has no matching ID.
- Prior source inspection reads the saved commit and path through the shared bounded Git reader. Missing historical objects produce an explicit unavailable state; the current source is never substituted. Current assessment evidence remains separately linked to Changes.
- The Findings view states that local dismissal and GitHub thread resolution do not prove a code fix.

## Verification

- `CARGO_BUILD_JOBS=1 ./scripts/validate desktop` passed on the working tree; full output is in [VALIDATE-32-PASS.log](VALIDATE-32-PASS.log).
- Persistence tests verify compact prior text/source metadata across replacement, correct head/merge-base SHA binding, and that the prior full result is not retained. Evidence tests read exact old commits from a temporary Git repository and report unavailable objects. Native reducer tests cover all four statuses, ordering, unassessed concerns, and current-finding relationships.
- A separate local visual fixture at `/tmp/bodh-ticket32-review` contains generation 3 with all four follow-up statuses and one new concern for PR #103. It was committed through the real validation/store path. The historical source object is present in the local checkout at `config/quickshell/menu/WifiDropdown.qml:104`.
- `./scripts/validate gui` built and opened the release app against that isolated state; the run log is [GUI-32.log](GUI-32.log). The app is open for visual review and starts at Home; select the recent PR #103 entry, then open Findings.
- No GitHub writes were performed. Native visual review is pending; keep issue #32 open until the user confirms the rendered flow.
