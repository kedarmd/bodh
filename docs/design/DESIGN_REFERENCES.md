# Bodh — Design Reference Board

**Scope:** Design-stage inspiration and reference materials only. This is not a substitute for the product requirements or an implementation plan.

The references below illustrate useful **interaction patterns**, not appearance to reproduce. Do not copy branding, visual assets, colors, or whole layouts. Design original Bodh screens in FigCraft using the product and interaction requirements from [`FIGCRAFT_BRIEF.md`](FIGCRAFT_BRIEF.md) and the local PRD/SPEC.

## S01 Home / workspace

- **Linear UI redesign** — calm dark-mode navigation, strong list/detail hierarchy, compact metadata: [article](https://linear.app/now/how-we-redesigned-the-linear-ui); [reference screenshot](https://webassets.linear.app/images/ornj730p/production/23839a6bacba4a7617728dada3d37a40dc3584aa-2352x1380.png?auto=format&dpr=2&q=95).
- Design goal: repo list, deduplicated recent reviews, zero oversized health/status cards.

## S02 Repository PR browser

- **GitHub's new PR dashboard** — searchable, concise rows, useful review and lifecycle metadata: [GitHub Changelog announcement](https://github.blog/changelog/2026-03-26-new-pull-requests-dashboard-is-in-public-preview/).
- Design goal: single high-signal PR list; select a PR to resume or open review, compact search/filter controls.

## S03 Overview

- **GitHub PR description** — full Markdown presentation and context: [article screenshot](https://www.acrocommerce.com/article/how-to-write-better-pull-request-summaries); [image](https://backend.acrocommerce.com/sites/default/files/styles/content_image/public/2024-05/Bens%20PR%20Example.png.webp?itok=JTVFJhae).
- **Sequence diagram** — conventional participant/message/branch readability: [Mermaid Studio reference](https://mermaidstudio.dev/docs/diagram-types/overview/); [image](https://mermaidstudio.dev/_astro/sequence.D9VVlkA9.png).
- Design goal: brief original-description excerpt; user-invoked full reader; clickable **monochromatic** diagram with single changed-interaction accent and adjacent source inspector; implementation summary and meaningful mismatches only.

## S04 Changes

- **GitHub Files Changed** — file navigator + inline review comments: [design screenshot article](https://dev.classmethod.jp/articles/github-pull-request-files-changed-ui-improvement/); [image](https://devio2024-2-media.developers.io/upload/wp-author-wakatsuki-ryuta/2026-02-23/sVLurIoewJGD.png).
- **Graphite code review** — inline thread, reply, and suggested replacement: [article](https://graphite.com/guides/best-ai-model-for-code-review); [image](https://graphite.com/images/content/guides/best-ai-model-for-code-review/reviewer-comment.png).
- **GitHub official review behavior** — line/range comments, suggestions and staging: [GitHub Docs](https://docs.github.com/en/pull-requests/how-tos/review-pull-requests/reviewing-proposed-changes-in-a-pull-request).
- Design goal: exact-line inline composer and publication state, rather than detached pinned-source form. Dense readable diff and unobtrusive file tree.

## S05 Findings

- **Linear UI** — selection from a calm navigation/detail layout: [article](https://linear.app/now/how-we-redesigned-the-linear-ui).
- **Code review suggestion** — code evidence and commentary together: [freeCodeCamp article](https://www.freecodecamp.org/news/how-to-perform-code-reviews-in-tech-the-painless-way/); [reference image](https://cdn.hashnode.com/res/hashnode/image/upload/v1732880687958/8d0e1ce5-cb23-4c62-b9ba-823f3a59845e.png).
- Design goal: LEFT findings with title/severity/file; RIGHT pinned source code above concrete AI-suggested comments and code replacements. Avoid dashboard cards and unrelated form panels.

## S06 Finish Review

- **GitHub review submission** — review type, summary, final action: [GitHub guide](https://docs.github.com/en/pull-requests/get-started/reviewing-pull-requests-quickstart); [illustrative screenshot](https://www.tianocore.org/tianocore-wiki.github.io/images/github-and-prs/leaving-feedback-in-review.png).
- **Existing GitHub discussion thread** — reply and resolve: [Flower Framework guide](https://flower.ai/docs/framework/contributor-tutorial-contribute-on-github.html); [screenshot](https://flower.ai/docs/framework/_images/resolve_conv.png).
- Design goal: context-preserving side drawer, staged-feedback summary, review decision and explicit confirmation; not a full-page form.

## S07 Shared UX states

Use the above references to design the expanded PR Markdown reader, code inspector, inline composer, stale revision warning, no-AI state, saved/staged drafts, thread replies, and publication validation/partial failure.

### Reference-source policy

These are third-party screenshots used only for design inspiration and analysis. They are not Bodh assets and should not be embedded into the application UI. If an external image is inaccessible, inspect the linked source article or continue using the written behavior descriptions instead of fabricating a source image.
