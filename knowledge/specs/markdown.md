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
- A fixed whitelist of *inline* HTML tags renders through the same style roles as
  the markdown constructs they mirror; everything else is dropped.

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

The boundary is also only a boundary once the line that carries it is
*terminated*. A stream arrives token by token, so the buffer routinely ends
mid-line — and a nested item's indent, or an indented block's, is a
whitespace-only line until its content lands. Committing there splits a list
between two independently parsed segments, and the cache makes that permanent:
the halves become unrelated top-level blocks for the rest of the transcript. The
invariant that catches this class is that a streamed render must equal the
one-shot render of the same source, delta size notwithstanding.

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

### Inline HTML is a whitelist, not a parser

pulldown-cmark hands raw HTML back verbatim, and dropping it is lossy twice
over: the markup goes *and* so does its meaning, so `a<br>b` silently joins two
lines. Rendering the presentational inline tags — emphasis, code, links, images,
breaks, sub/sup — recovers the intent for the HTML that actually appears in
transcripts and READMEs.

What keeps this from becoming an HTML renderer is that it is a **tag-name
whitelist over a string pulldown already isolated**. There is no DOM, no
attribute model beyond `href`/`src`/`alt`, and no new dependency. Every
recognized tag resolves a `StyleSheet` role that markdown already uses, so
`<b>` cannot look different from `**bold**` and a host restyling one restyles
both. Anything unrecognized keeps the old behavior — dropped, never echoed as
literal markup, which would let untrusted input paint arbitrary text.

Two invariants bound the failure modes. Each open tag records the stack depths
it pushed, so an unbalanced or crossed tag can only fail to unwind, never
corrupt the parser's style and link stacks; and nesting is capped. Scopes are
closed at the end of every block, which is both the sane reading of an unclosed
tag and what keeps the settled-prefix cache honest: a scope that outlived a
block boundary would style the tail only while the tail was still being
re-parsed, breaking the streamed-equals-one-shot invariant *in styles while the
text matched*.

`<sub>`/`<sup>` transliterate to Unicode all-or-nothing. Partial coverage
(`4ᵗh`) depends on which characters a word happens to use, which is a worse
result than leaving the text alone.

### Block HTML is a seam for the same reason highlighting is

The presentational inline tags need no parser. Block HTML does — a tree builder
that recovers implied end tags, inserts the `<tbody>` nobody wrote, and survives
malformed input is exactly the dependency [goal.md](./goal.md) keeps out. So
`HtmlBlockRenderer` takes the raw run, the available width, the theme, *and the
stylesheet*: an implementation resolves the same roles the surrounding markdown
does, or HTML in one document would look like it came from another. With no
renderer attached the block is dropped, which is what markdown did with all HTML
before the seam existed, so attaching one is purely additive.

`tuika-html` is that implementation, and it is also where the line inside this
capability shows: the same crate serves the fenced-`html` seam, and adds a
standalone `Html` view for fragments that are not inside markdown at all.

One consequence of pulldown-cmark's framing is worth stating, because it looks
like a bug: an HTML block ends at a blank line, so an element whose content is
separated by blank lines arrives as several independent blocks. That framing is
identical whether the source is streamed or rendered in one shot, which is
precisely what lets the settled-prefix cache hold an HTML block at all. Joining
adjacent blocks before rendering would be nicer to look at and would break that
— the halves can straddle a cache boundary — so the framing stands and the
renderer is told about it.

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

- No HTML *document* rendering: no DOM, no CSS, no block-level HTML layout.
  Block HTML (`<div>`, `<details>`, `<table>`) is dropped in core; a host that
  wants it supplies the parser behind a seam, the way `FencedBlockRenderer`
  already lets a ` ```html ` fence be rendered by a companion crate.
- No raw-HTML passthrough: unrecognized markup is never echoed as literal text.
- No markdown *authoring* or round-tripping — rendering only.
- No document-level layout beyond a linear block sequence (no columns, no
  floats).

## Public surface

- [`docs/markdown.md`](../../docs/markdown.md) — the guide. Markdown carries more
  user-facing surface than one gallery entry holds (streaming, table fitting,
  the highlighter seam, link policy, images), so it gets a page of its own and
  [`docs/components.md`](../../docs/components.md) keeps only the gallery entry
  and points here.
- [`docs/components.md`](../../docs/components.md)
- [`README.md`](../../README.md) § Markdown and syntax highlighting
