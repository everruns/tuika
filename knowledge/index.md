# tuika Knowledge

This directory is tuika's Open Knowledge Format (OKF) bundle: the durable
product, architecture, policy, and development-process memory used by
maintainers and coding agents.

Read this index first, then open only the concepts relevant to the task and
follow their links. Public documentation lives in [`README.md`](../README.md)
and [`docs/`](../docs/); it must not link back into this internal bundle.

Concepts are split by what they describe: `specs/` defines the product — what
tuika is and how it is built — while `processes/` defines how maintainers and
agents work on it.

## Product direction

- [Product goal](specs/goal.md) — what tuika is for, and the boundary it defends.
- [Architecture](specs/architecture.md) — the view/state/layout/host model and its seams.

## Capabilities

- [Markdown](specs/markdown.md) — CommonMark rendering, streaming, and the highlighter seam.
- [Images](specs/images.md) — terminal graphics protocols over reserved cells.
- [Keymap](specs/keymap.md) — declarative key-binding dispatch.
- [Styling](specs/styling.md) — themes as tokens, stylesheets as rules.
- [Out-of-band escapes](specs/out-of-band.md) — hyperlinks, clipboard, and native progress.

## Engineering processes

- [Testing](processes/testing.md) — hermetic rendering tests and the benchmark gates.
- [Shipping](processes/shipping.md) — requirements for landing a change.
- [Maintenance](processes/maintenance.md) — repository health and release readiness.
- [Release](processes/release.md) — publishing tuika and tuika-codeformatters.
- [Documentation](specs/documentation.md) — public/internal documentation contract.
