---
type: Product Specification
title: Markdown rendering
description: Defines tuika's CommonMark rendering, its streaming form, and the highlighter seam that keeps grammars out of the crate.
---

# Markdown rendering

## Why

Agent transcripts, help panes, and release notes are markdown. Rendering them in
a terminal is not just a parse: prose must wrap to the viewport while code and
tables must not, styling must come from the host's theme rather than hard-coded
colors, and — the hard part — text arrives *incrementally*, token by token, and
is re-rendered on every frame while it streams.

A naive implementation re-parses and re-highlights the whole document each
frame. For a long transcript with settled code blocks that is quadratic work for
a document whose prefix cannot change.

## What

- `Markdown` renders a complete CommonMark document to styled lines. Prose is
  word-wrapped to the available width; code blocks and tables are laid out
  verbatim so their alignment survives.
- `MarkdownState` is the streaming form. Fed deltas as a message arrives, it
  re-parses only the in-flight tail and caches everything before the last stable
  block boundary. Settled blocks keep their already-computed lines — and their
  already-computed highlighting.
- Inline images (`![alt](url)`) are promoted to block images when the host
  supplies a resolver; see [images.md](./images.md).

## Design

### Two passes, split at the width

Rendering is parse-then-flatten, and the two passes are separated by what they
know: parsing resolves block structure, inline styling, and links at *no
particular width*; flattening turns that intermediate form into width-fitted
lines. Nothing width-dependent may leak into the first pass.

That boundary is not tidiness — it is what makes both caching strategies
possible. The settled-prefix cache below can keep parsed blocks across frames
only because a parsed block is width-independent, and a resize can re-flatten
the prefix without re-tokenizing it. A parser that wrapped as it went would make
every width change a full re-parse.

The module's files follow the passes rather than the vocabulary, so a change has
one obvious home: the intermediate form, the parse pass, the flatten pass, table
layout, the streaming cache, the image seam, and the view.

### The settled-prefix cache

The cache boundary is the last *stable block boundary*, not the last newline: a
block is settled only once the parser can no longer reinterpret it, so an
unterminated code fence or a list that may still gain items stays in the
re-parsed tail. Everything before the boundary is retained as computed lines and
is never re-tokenized or re-highlighted.

Anything positional that the host reads back per frame — block-image placements
in particular — must therefore be threaded *through* the cache: fixed rows for
settled blocks, re-derived each frame for the in-flight tail. A frame-local
computation that only looks at the tail would silently lose the settled part.

### Highlighting is a seam, not a dependency

tuika owns the presentation of code — frame, background, language label,
optional line-number gutter, wrapping — and takes token colors from any
`Highlighter` the host supplies. The trait is deliberately narrow: given a
language name and source, return styled spans.

This is the [goal.md](./goal.md) boundary applied to syntax: grammar crates are
large, numerous, and versioned independently of any UI concern. Keeping them out
means a host that renders no code pays nothing, and a host that renders code can
choose its own highlighter. `tuika-codeformatters` is the batteries-included
answer, published separately for exactly that reason.

### Rich fenced blocks are a second seam

Syntax highlighting must reconstruct the original source line-for-line, so it
cannot express a fence whose presentation has a different shape than its source.
`FencedBlockRenderer` handles that case: given the language, source, current
width, and theme, it may replace the fence with styled lines. Returning `None`
keeps the ordinary code-block fallback, so unsupported or malformed content
never disappears.

Parsed fenced blocks retain both their source and their already-highlighted
fallback. Rendering happens during width-dependent flattening: settled blocks
are rendered once per width, while an in-flight fence may be attempted on each
streaming frame. Implementations therefore stay deterministic and avoid I/O.
`tuika-mermaid` is the first companion implementation, keeping mmdflux and its
diagram grammars outside tuika core. The adapter bounds source and output size,
disables ANSI output, and strips control bytes before creating cells; a diagram
fence is untrusted markdown, not permission to emit terminal commands.

### Styling is role-driven

Markdown parts (headings, links, inline code, block quotes, rules) resolve their
style through the `StyleSheet` roles rather than reading colors directly, so a
host that restyles headings restyles them in markdown too. See
[styling.md](./styling.md).

## Constraints

- Parsing uses `pulldown-cmark` with default features off: the bundled HTML
  renderer and SIMD scanner are unused weight, and dropping them keeps the
  dependency pure-Rust.
- Rendering untrusted markdown must degrade — slow, truncated, or unstyled — but
  never panic or allocate unboundedly. Parser-driven panics are a security
  concern, not just a bug (see [`SECURITY.md`](../../SECURITY.md)).
- The streaming path is benchmarked (`benches/markdown.rs`,
  `benches/markdown_iai.rs`); the instruction-count baseline is a CI gate, so a
  change that re-tokenizes settled blocks fails the build rather than quietly
  regressing. See [testing.md](../processes/testing.md).

## Non-goals

- No HTML rendering or raw-HTML passthrough.
- No markdown *authoring* or round-tripping — rendering only.
- No document-level layout beyond a linear block sequence (no columns, no
  floats).

## Public surface

- [`docs/components.md`](../../docs/components.md)
- [`README.md`](../../README.md) § Markdown and syntax highlighting
