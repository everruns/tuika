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

## Maintaining this bundle

The bundle is written by the work it describes, not swept up afterwards. When a
change alters durable behavior, intent, architecture, policy, constraints,
terminology, or maintainer process, update the affected concepts **in the same
change**. A concept that no longer matches the code is a defect, not debt — and
the reason for a decision is cheapest to write down while it is still at hand.

- Adding, removing, renaming, or reclassifying a concept updates this index.
  `scripts/validate_okf.py` fails on a concept the index does not list, so a
  moved file cannot quietly become unreachable.
- Significant changes get an entry in [the log](log.md); routine wording,
  formatting, and link fixes do not.
- Concepts capture **why** and **what**. Transient plans, task status, test
  output, and exhaustive source-level detail belong outside the bundle.
- Not every change needs one. When none applies, say so and why rather than
  leaving the question unanswered — [Shipping](processes/shipping.md) makes that
  an explicit outcome.

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
