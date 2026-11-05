# Issue #25 validation evidence

The desktop runner validates the application contract/CLI and the native GPUI spike without opening a GUI or contacting GitHub. Validation output is preserved in `VALIDATE-25-final.log`; two earlier attempts remain in `VALIDATE-25.log` and `VALIDATE-25-rerun.log`. The first caught a Clippy type-complexity warning and the second caught formatting in the separate native manifest. Both were corrected before the passing run.

The passing run completed `./scripts/validate desktop` on the #25 worktree before merging integration revision `9c1d6c1`. It included the five freshness tests, native no-default and desktop checks, lint, and tests. The native compilation took about four minutes and forty seconds because GPUI dependencies were built separately for check, test, and Clippy profiles. This was build time; no GUI trial was performed.

The freshness tests cover a moved head while preserving the pinned preparation, separately observed base/branch movement, unknown/offline metadata, review-intent fingerprint changes, and rejection of an approval after a review-intent revision changes. No live remote writes were performed. A final desktop run on the merged #22/#27 integration tip is recorded after merge in `VALIDATE-25-integrated.log`.
