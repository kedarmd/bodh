# Git/gh capability spike — issue #3

This is a disposable, headless experiment for [issue #3](https://github.com/kedarmd/bodh/issues/3), not the production adapter or frozen contract. It lives on the clearly marked `spike/2-native-sequence` branch alongside the renderer experiment. Python 3, installed Git and authenticated gh are sufficient; no daemon or GUI is used.

Reproduce from the repository root:

```sh
python3 -m unittest discover -s spikes/pr-capabilities -v && python3 spikes/pr-capabilities/harness.py --live github.com/cli/cli/14854 --output /tmp/bodh-capabilities.json
```

The default page size is **one** to exercise pagination. Omit `--live` for entirely local fixtures. Output paths must have an existing parent. Network requests have a 120-second timeout per subprocess; failed acquisition raises and does not produce a successful report. Run the same command with `github.com/cli/cli/14858` for the discussion fixture. The committed evidence records the actual tested versions and observations; a later live run can differ as remote data changes.

`./scripts/validate` includes these behavioral checks and Python compilation in addition to the existing Rust checks. Python compilation is syntax validation, not static typechecking. The spike uses only the Python standard library.

## Acquisition experiment

The local fixture creates three disposable repositories: upstream, a fork with changes, and a dirty checkout missing both the later target and fork objects. The fork head is exposed through the upstream's `refs/pull/7/head`, as GitHub exposes fork PRs. Acquisition first tries exact commit objects locally, fetches the exact base SHA if absent, then fetches the PR head ref if needed. Every requested SHA is checked as a commit after fetching. No destination refspec or configured remote is used. Fetch disables tags, submodule recursion, automatic maintenance, hooks and FETCH_HEAD writes. No checkout, reset, merge or branch switch is used on the review checkout.

This deliberately uses full history, not shallow fetches. The merge-base therefore has complete ancestry in these fixtures. A shallow pre-existing checkout needs a separately designed deepen/unshallow policy; multiple merge bases need a product policy. Objects remain subject to normal Git retention: production needs refs/cache retention to keep pinned objects alive. These are not resolved by this spike.

An unavailable base SHA fails explicitly; no fallback to latest target tip is permitted. If the PR ref moves and the original head is absent after fetch, acquisition fails rather than substituting the new head. Some servers refuse fetching unadvertised SHA objects. That is an acquisition error, not permission to reinterpret SourceRef base. Git objects/ref metadata can change even on failure. Authentication/access/network failures are surfaced as failures; the harness does not infer which cause from a generic gh exit.

The local target advances independently. The prepared base tip differs from the merge-base. SourceRef base reads the **merge-base**; context identity includes both tips, the merge-base, normalized explicit repository identity, a versioned comparison description and patch digest. Identical inputs in the same tested environment produce the same identity. Moving a target is a distinct preparation even if the resulting diff is unchanged. This experimental hash is not a proposed canonical contract, and Git-version/config-dependent patch behavior must be specified before contract freeze.

## Anchors and content

Raw NUL-separated Git metadata preserves paths independently of human-readable quoted patch headers. Renames are detected at 50%; unified hunks map old and new line numbers independently. Removed lines only have old coordinates; additions only have new coordinates; context lines have both. Inclusive ranges require every line on the requested side to occur in a hunk. Old-side ranges cannot be replacement suggestions. Head-side hunk ranges are candidate replacement spans only: this experiment does not certify GitHub publication acceptance, stale-head checks or content permissions.

Reading a regular commit blob is separate from anchoring: the fixture reads unchanged line 15 successfully but rejects it as an inline anchor outside the diff. Base line 2 reads the old text even though checkout bytes are dirty. Source locators reject absolute, traversal, missing-tree and non-regular entries. Exact CRLF and no-terminal-newline bytes are preserved. Git external diff and textconv are disabled; a deliberately configured helper must never run.

Binary files and submodule gitlinks are reported without line anchors. Symlinks are explicitly unsupported for source/suggestions. LFS pointer source is rejected rather than treated as materialized content; no LFS fetching occurs. Arbitrary non-UTF-8 path names, combined/merge diffs, cross-side ranges, huge-content bounds and full .gitattributes/config normalization remain unsupported. Ordinary UTF-8 POSIX paths, spaces and renamed paths use literal Git pathspecs. Source bytes are never read from the working copy.

## Discussions and compatibility

GraphQL requests are queries sent as JSON stdin through installed gh. Threads and each thread's comments have **independent** cursor loops; a single outer `gh --paginate` would not prove nested completeness. Each connection records expected count, fetched count, completeness and a failure reason. Missing/repeated cursors, null nodes, duplicate IDs, count disagreement and request errors prevent completeness. A partial result preserves acquired nodes and identifies the affected connection; an inaccessible connection's total is honestly unknown. A changing server can invalidate a paginated observation; this is not an atomic snapshot.

Thread fields preserve node ID, path, current/original ranges, sides, resolved/outdated state and viewer resolve/unresolve/reply permissions. Comments preserve GraphQL and numeric IDs, replyTo IDs (null for top-level), author, body, URL, original/current lines, diff hunk, edit/delete permissions and timestamps. The first/root comment's numeric ID is the REST reply target; thread node IDs serve GraphQL resolution. No publication was tested, and no review/comment/resolve mutation is present. Issue conversation comments and review summary bodies are outside this review-thread experiment.

Primary references: [Git fetch options](https://git-scm.com/docs/git-fetch), [GitHub pull request GraphQL objects](https://docs.github.com/en/graphql/reference/pulls). These describe the fields/options used; executable compatibility is demonstrated only for the versions and github.com host recorded in evidence. GitHub Enterprise Server schema versions, host-specific authentication, unadvertised-object policies, proxies, SSH configurations and fork access restrictions remain **unverified**. Host/owner/repository are explicit inputs; neither origin nor github.com is implicitly selected.
