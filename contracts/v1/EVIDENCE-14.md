# Issue #14 acquisition and difficult-content evidence

## Object acquisition

Preparation now acquires exact immutable base and head commits before deriving the merge base or diff. The selected matching remote is read from the checkout's raw remote URL configuration (so URL rewrite rules cannot disguise its repository identity). Remote names are explicit when multiple compatible URLs exist; no `origin` or `github.com` default is assumed. The PR metadata request uses `gh api --hostname <host>` for the selected DNS host.

Fetch order is deterministic:

1. Fetch the base commit from the observed `refs/heads/<base.ref>` on the selected base remote.
2. Fetch the head from `refs/pull/<number>/head` on that base remote, the conventional fork-safe PR ref.
3. If that ref cannot supply the observed SHA, try `refs/heads/<head.ref>` on the API-reported fork repository URL after validating its host and normalized owner/repository identity.
4. If a server refuses an advertised ref, attempt the immutable observed commit SHA directly. Every fetched ref target is compared with the API SHA before use; a changed target is removed and never substituted.

Acquired objects are retained in deterministic Bodh-owned `refs/bodh/prepared/<sha256>` refs. Fetch disables tags, submodule recursion, `FETCH_HEAD` writes, automatic maintenance, commit-graph writes and hooks. It does not check out a ref, switch branches, reset, merge, update the index or write worktree files. Git runs in a cancellable process group with the shared operation deadline and bounded output. The `gh auth git-credential` helper is supplied in-process for HTTPS transport; no token is written to disk or included in process arguments.

Unavailable objects fail explicitly. Shallow histories, ambiguous remotes, moved/unavailable refs and non-unique merge bases are not resolved by using a current branch tip. Compatible host parsing/routing is implemented for DNS hosts, but this ticket did not run live fetches against GitHub Enterprise or other hosts. The production compatibility evidence here is a local bare Git fixture: the checkout selects SSH-form remote `review` at `git.example.invalid`; local URL rewrite rules route the base repository and a separate fork bare repository to local fixture paths; a controlled `gh` executable asserts the host passed to the metadata request. This proves the argument/ref strategy and local Git object behavior, not live SSH authentication, host compatibility, or any server's ref policy. Installed tool versions during validation were Git 2.55.0 and GitHub CLI 2.101.0.

`acquires_missing_fork_head_from_the_explicit_fork_ref_without_touching_checkout_state` starts without either pinned commit in the registered checkout, fetches base and fork-head history from separate bare remotes, reads the pinned head blob, and verifies unchanged raw `HEAD`, symbolic branch, `.git/index`, dirty tracked bytes, staged bytes and untracked bytes. It also exercises a non-`origin` selected remote, a non-`github.com` DNS host and rejection of ambiguous matching remotes. `cancelled_fetch_preserves_checkout_state` holds a controlled Git fetch process and verifies cancellation preserves the same checkout state.

## Diff and content support

The shared `diff_index` service remains the authority for line anchors and readability. It uses NUL-separated raw Git metadata and commit blob IDs; external diff and textconv are disabled. Rename detection is fixed at 50%; textual patch parsing pins Myers, disables the indent heuristic, fixes context width and output ordering. The context identity remains SHA-256 over the versioned `bodh-context-v1` prefix, normalized host/repository/PR identity and exact base, head and unique merge-base SHAs. Git presentation settings, dirty files and current branch tips do not enter this identity.

The real-repository fixtures cover:

- Added, deleted, edited, pure-renamed and edited-renamed files with independent old/new hunk sides.
- CRLF bytes and a missing terminal newline, preserving exact source delimiters.
- Binary blobs, LFS pointer blobs, symlinks and gitlinks as unsupported for inline line anchors/source.
- A file-to-directory transition as a deletion plus a child-path addition, and non-UTF-8 Git paths as explicitly unsupported.
- Hostile source paths, Git pathspec metacharacters, and configured external diff/textconv helpers that must not run.

Readability remains separate from publication eligibility: ordinary UTF-8 commit blobs can be read outside changed hunks, while an inline anchor must fall inside a changed hunk on the requested side. Binary/LFS/symlink/gitlink entries have no supported line anchors.

Run `cargo test --locked --test preparation`, `cargo test --locked --test findings`, and `cargo test --locked cancelled_fetch_preserves_checkout_state` for focused coverage. The final full `./scripts/validate` output is preserved in [VALIDATE-14.log](VALIDATE-14.log). No live PR writes, GUI trial or live remote compatibility check was performed.
