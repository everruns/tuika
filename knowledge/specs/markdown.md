---
type: Product Specification
title: Markdown rendering
description: Defines tuika's CommonMark rendering, its streaming form, and the highlighter boundary that keeps grammars out of the crate.
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
  block boundary. Settled blocks keep their already-computed lines, highlighting,
  image placements, and hyperlink runs. `links()` exposes row-aligned targets
  for hosts that scroll or window those lines before applying OSC 8.
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
layout, the streaming cache, the image boundary, and the view.

### The settled-prefix cache

The cache boundary is the last *stable block boundary*, not the last newline: a
block is settled only once the parser can no longer reinterpret it, so an
unterminated code fence or a list that may still gain items stays in the
re-parsed tail. Everything before the boundary is retained as computed lines and
is never re-tokenized or re-highlighted.

Two constructs decide whether a blank line is a boundary at all, and both are
easy to get subtly wrong because the mistake only shows under *some* chunk
boundaries:

- **An open code fence.** CommonMark closes a fence only on a bare run of at
  least as many of the same delimiter, so a fence line carrying an info string
  inside an already-open block is code content, not a closer. Treating it as one
  settles a boundary the one-shot parse puts *inside* the block, and the rest of
  the document then renders as markdown in the stream and as code on a re-render.
- **An open list.** A list continues across a blank line, so a blank between
  items is a boundary only once a following *top-level* block proves the list
  ended — and never on the strength of an in-flight partial line, since `1` is
  not yet the marker `1.` the next delta makes it. Settling early parses the
  rest in isolation, which restarts the numbering.

The boundary is also only a boundary once the line that carries it is
*terminated*. A stream arrives token by token, so the buffer routinely ends
mid-line — and a nested item's indent, or an indented block's, is a
whitespace-only line until its content lands. Committing there splits a list
between two independently parsed segments, and the cache makes that permanent:
the halves become unrelated top-level blocks for the rest of the transcript. The
invariant that catches this whole class is that a streamed render must equal the
one-shot render of the same source, delta size notwithstanding — asserted as a
fuzz differential over generated markdown, chunk sizes, and widths (see
[Testing](../processes/testing.md)), which is how both rules above were found.

Anything positional that the host reads back per frame — block-image placements
in particular — must therefore be threaded *through* the cache: fixed rows for
settled blocks, re-derived each frame for the in-flight tail. A frame-local
computation that only looks at the tail would silently lose the settled part.

### Highlighting is a boundary, not a dependency

tuika owns the presentation of code — frame, a background that fills the
available width, language label, optional line-number gutter, wrapping — and
takes token colors from any `Highlighter` the host supplies. Nested blocks keep
their outer indentation unpainted. The trait is deliberately narrow: given a
language name and source, return styled spans.

This is the [goal.md](./goal.md) boundary applied to syntax: grammar crates are
large, numerous, and versioned independently of any UI concern. Keeping them out
means a host that renders no code pays nothing, and a host that renders code can
choose its own highlighter. `tuika-codeformatters` is the batteries-included
answer, published separately for exactly that reason.

### Structured blocks share one parsing boundary

Syntax highlighting must reconstruct the original source line-for-line, so it
cannot express a fence whose presentation has a different shape than its source.
`MarkdownBlockRenderer` handles that case: it receives a structured
`MarkdownBlock` descriptor plus one `MarkdownBlockContext` carrying width,
theme, and the active stylesheet. Today the descriptor distinguishes fenced
blocks from raw block HTML; it is non-exhaustive so another parser-backed block
form does not require another parallel trait. Returning `None` tries the next
renderer in registration order. If none handle a fence, the ordinary code-block
fallback remains, so unsupported or malformed content never disappears.

Parsed fenced blocks retain both their source and their already-highlighted
fallback. Rendering happens during width-dependent flattening: settled blocks
are rendered once per width, while an in-flight fence may be attempted on each
streaming frame. Implementations therefore stay deterministic and avoid I/O.
`tuika-mermaid` is a companion implementation, keeping mmdflux and its
diagram grammars outside tuika core. The adapter bounds source and output size,
disables ANSI output, and strips control bytes before creating cells; a diagram
fence is untrusted markdown, not permission to emit terminal commands.

A block renderer sits inside `View::render`, so it must not be able to fail the
frame: an unrenderable fence is a `None` and the code-block fallback, never an
unwind. That is a constraint on the *adapter*, not on the engine behind it —
`tuika-mermaid` contains mmdflux's panics rather than trusting a third-party
layout engine to be total — the containment is unconditional and stays whether
or not a reachable panic is currently known. Working *around* a specific
upstream defect is the opposite: it is carried only while the defect is
unfixed, and is removed on the release that fixes it rather than left as
permanent scar tissue. Containment does not extend to the panic hook: a
renderer runs every frame, and swapping a global hook that often would swallow
unrelated panics for as long as the window is open.

The context carries width because a renderer is expected to *use* it. A diagram
engine sized for vector output spreads a graph far past a terminal pane, and
`Markdown` clips rather than scrolls, so the adapter re-lays out an overflowing
diagram at tighter separations until it fits. Fitting is best-effort: a graph
can be irreducibly wider than the pane, and the narrowest layout is still the
right answer there. Diagrams that already fit keep the engine's own spacing, so
tightening never changes what already renders well.

The chain is deliberately open rather than one field per syntax. A host can
register Mermaid, HTML, and its own query-plan renderer in one ordered list;
each implementation sees every structured block and declines the kinds it does
not own. `Markdown` and `MarkdownState` both append renderers when their
`block_renderer` builders are called repeatedly.

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

### Block HTML reuses the structured-block boundary

The presentational inline tags need no parser. Block HTML does — a tree builder
that recovers implied end tags, inserts the `<tbody>` nobody wrote, and survives
malformed input is exactly the dependency [goal.md](./goal.md) keeps out. Raw
HTML is therefore the second `MarkdownBlock` variant, not a second trait. Its
renderer receives the raw run through the same context as a fence, including
the active stylesheet: headings and links resolve the same roles as the
surrounding markdown. With no renderer attached the block is dropped, which is
what markdown did with all HTML before the boundary existed, so attaching one is
purely additive.

`tuika-html` is that implementation, and it is also where the line inside this
capability shows: the same renderer serves raw HTML and fenced `html` blocks
without synthesizing a default stylesheet, and the crate adds a standalone
`Html` view for fragments that are not inside markdown at all.

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
  wants it supplies the parser behind `MarkdownBlockRenderer`, the same boundary a
  ` ```html ` fence uses.
- No raw-HTML passthrough: unrecognized markup is never echoed as literal text.
- No markdown *authoring* or round-tripping — rendering only.
- No document-level layout beyond a linear block sequence (no columns, no
  floats).

## Public surface

- [`docs/markdown.md`](../../docs/markdown.md) — the guide. Markdown carries more
  user-facing surface than one gallery entry holds (streaming, table fitting,
  the highlighter boundary, link policy, images), so it gets a page of its own and
  [`docs/components/markdown-code.md`](../../docs/components/markdown-code.md)
  keeps only the gallery entry and points here.
- [`docs/components/markdown-code.md`](../../docs/components/markdown-code.md)
- [`README.md`](../../README.md) § Markdown and syntax highlighting
