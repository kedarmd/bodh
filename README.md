# Bodh V1 — Fresh Rebuild

**Bodh (बोध)** is a complete native Linux GitHub PR review application. The V1 product includes manual code review, AI-assisted analysis through an external `pr-review` skill, source-linked sequence diagrams, findings, threaded discussions and safe GitHub publication.

This repository begins from a clean slate. The archived GPUI implementation is only historical context; its visual styling and app code are not authoritative. The **PRD and SPEC cover the full V1 product**, including design, implementation, integration, testing and release.

## Documentation

- [`docs/PROJECT_STATUS.md`](docs/PROJECT_STATUS.md) — **current** stage and explicitly authorized work; update this as stages change.
- [`docs/README.md`](docs/README.md) — index and precedence for all documents.
- [`docs/PRD.md`](docs/PRD.md) — complete, long-lived product requirements.
- [`docs/SPEC.md`](docs/SPEC.md) — complete technical/contract specification.
- [`docs/design/FIGCRAFT_BRIEF.md`](docs/design/FIGCRAFT_BRIEF.md) — **stage-specific** Codex design assignment.
- [`docs/design/DESIGN_QUALITY_CONTRACT.md`](docs/design/DESIGN_QUALITY_CONTRACT.md) — design QA/geometry and reviewer-flow acceptance.
- [`docs/design/DESIGN_REFERENCES.md`](docs/design/DESIGN_REFERENCES.md) — design references and sources.
- [`docs/design/DESIGN_STATUS.md`](docs/design/DESIGN_STATUS.md) — design approvals only.
- [`AGENTS.md`](AGENTS.md) — durable coding-agent instructions, including stage checks.

## Starting work

Read [`AGENTS.md`](AGENTS.md) and [`docs/PROJECT_STATUS.md`](docs/PROJECT_STATUS.md) before running Codex. **At this initial stage**, only FigCraft design Gate 0–1 is authorized; finish those gates and request approval. Later prototype, development, QA and release work are covered by the PRD/SPEC but require separate authorization.

Rune is the preferred Rust UI candidate, **pending a separately approved Linux/Wayland feasibility prototype**. No framework is finalized yet.
