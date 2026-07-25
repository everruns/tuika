# Markdown in tuika

tuika renders CommonMark (plus GFM tables and strikethrough) straight to styled
terminal lines — no HTML step, no intermediate document model to hold. It is the
component an agent or chat host leans on hardest, so it has more moving parts
than the rest of the [gallery](components.md): a streaming form, width-driven
table layout, a pluggable syntax highlighter, clickable links, and inline images.
This page covers all of it in one place.

- [The two entry points](#the-two-entry-points)
- [What renders](#what-renders)
- [Streaming](#streaming)
- [GFM tables](#gfm-tables)
- [Fenced code](#fenced-code)
- [Links](#links)
- [Images](#images)
- [Styling](#styling)

## The two entry points

`Markdown` is a `View`: hand it a string, place it in a layout, done. It is the
right answer for static markdown — a help panel, a release note, a rendered
`README`.

```rust
use tuika::prelude::*;
view! {
    col(padding = Padding::all(1)) {
        node(Markdown::new("# Title\n\nSome **bold** prose."))
    }
}
```

`MarkdownState` is the streaming form, and the one a transcript wants: it holds
the source, is fed deltas as a message arrives, and hands back styled lines on
demand. `to_lines` is the same renderer as a bare function, for a host that has
neither a layout slot nor a stream.

```rust
use tuika::prelude::*;
let mut md = MarkdownState::new();
md.push_str(delta);                                  // forward each stream delta
let lines = md.lines(width, &theme, &sheet, CodeHighlighter::Plain);
view! { node(tuika::components::Text::new(lines)) }
```

Lines come out already wrapped to the width you passed. Draw them **without**
further wrapping — tuika's `Text`, or ratatui's `Paragraph` with no `.wrap` —
or code indentation and table borders will be re-flowed into nonsense.

## What renders

| Construct | Notes |
| :-------- | :---- |
| Headings | `#`–`######`; bold + themed, italic from `###` down |
| Emphasis | `**bold**`, `*italic*`, `~~strikethrough~~`, `` `inline code` `` |
| Lists | Bullet and ordered, nested (2 columns per level), markers themed |
| Task lists | `- [ ]` / `- [x]`, checkbox painted as a themed marker |
| Block quotes | Indented per level of nesting |
| Thematic breaks | `---` as a themed rule |
| Fenced code | Verbatim, with a language label and an optional highlighter |
| Tables | GFM pipe tables, boxed and fitted to the width |
| Links | Label painted, destination emitted as an OSC 8 hyperlink |
| Images | `![alt](url)` — real pixels via a host resolver, alt text otherwise |

Every one of these is measured in *cells*, not chars, so wide CJK glyphs and
multi-scalar emoji keep the layout honest.

## Streaming

<img src="demos/markdown.gif" width="880" alt="Markdown streaming demo: a document arriving one glyph at a time, with headings, bold and italic prose, a bullet list, and a syntax-highlighted Rust code block.">

A transcript re-renders on every delta, which is exactly the workload a naive
renderer is worst at. `MarkdownState` splits the source at the last **stable
block boundary** — a blank line outside an open code fence — and re-parses only
the in-flight tail. Everything before it is parsed and highlighted once and
cached, so a long conversation does not re-tokenize, and a settled code block is
not handed back to the highlighter, on each frame.

The cache holds *width-independent* parsed blocks. Layout — wrapping, table
column fitting, code framing — is recomputed each frame from the width you pass,
so the same state tracks the viewport as the terminal resizes; there is nothing
to invalidate by hand.

## GFM tables

<img src="demos/markdown_table.png" width="880" alt="A rendered GFM table with box-drawing borders: a bold header row; a left-aligned Component column of inline-code names; a centered Status column with ✅ and 🚧 emoji; and a right-aligned Docs column of underlined links.">

Pipe tables render with box-drawing borders, a bold header, and per-column
alignment taken from the `:---:` markers. Cells keep their inline styles — bold,
inline code, links, emoji — and are measured grapheme-aware, so a wide emoji
advances two columns and the borders stay square.

```rust
use tuika::prelude::*;
let doc = Markdown::new("\
| Component   |  Status   |                          Docs |
| :---------- | :-------: | ----------------------------: |
| `Markdown`  | ✅ stable | [docs.rs](https://docs.rs/tuika) |
| **Image**   |  🚧 beta  | [features](https://github.com/everruns/tuika) |
");
# let _ = doc;
```

Column widths come from the content, then the whole table is fitted to the
available width: the widest column is shrunk first, wrapping its cells, and the
rest keep their natural size. Below `4 * cols + 1` columns even that cannot fit,
so the box is dropped for ` | `-joined rows that word-wrap:

```text
Wide area — a fitted grid.          Very narrow — boxless fallback.
╭───────────┬────────┬──────────╮   Component | Status | Docs
│ Component │ Status │     Docs │   Markdown | stable | docs.rs
├───────────┼────────┼──────────┤   Image | beta | features
│ Markdown  │ stable │  docs.rs │
│ Image     │  beta  │ features │
╰───────────┴────────┴──────────╯
```

Because the fit is width-driven and re-run per frame, one source covers every
terminal size — the host never pre-formats a table for the pane it lands in.

## Fenced code

<img src="demos/code_block.png" width="880" alt="CodeBlock demo: a themed Rust snippet with a language label, a left rail, a line-number gutter, and syntax coloring.">

A fenced block is emitted **verbatim** — indentation is meaningful, so it is
never word-wrapped — and framed by the same renderer as the standalone
[`CodeBlock`](components.md#codeblock) component: language label, left rail, code
background.

Syntax coloring comes from a `Highlighter` you supply, because grammars are far
too heavy to live in tuika:

```rust
use tuika::prelude::*;
view! { node(Markdown::new(source).highlighter(&highlighter)) }
```

Without one, code is themed but uncolored. The `tuika-codeformatters` crate ships
a tree-sitter implementation covering the common languages; a host with its own
lexer implements the two-method trait instead.

## Links

A `[label](url)` paints the label in the theme's link style and emits the
destination as an [OSC 8 hyperlink](features.md#hyperlinks-osc-8) — clickable in
terminals that support it, plain styled text everywhere else. Bare URLs in prose
are linked in place.

`link_policy` decides which schemes are emitted:

```rust
use tuika::prelude::*;
use tuika::term::hyperlink::LinkPolicy;
// Default: http(s) only. `NONE` styles labels but emits no OSC 8 —
// for a host that handles clicks itself, or wants links inert.
view! { node(Markdown::new(source).link_policy(LinkPolicy::NONE)) }
```

Since markdown is usually model- or user-authored, the policy is a real security
boundary, not a preference: it is what stops a `file://` or `javascript:` URL in
untrusted text from becoming a clickable target. `LinkPolicy::WEB.with_mailto()`
opts `mailto:` back in where a host wants it.

## Images

`![alt](url)` renders as a real image where the terminal has a graphics protocol.
Markdown carries only the URL, so — exactly like the highlighter seam — the host
supplies the decode through an `ImageResolver`; a resolved image reserves a block
in the layout, an unresolved one stays an inline, link-styled placeholder rather
than dropping the URL.

```rust
use tuika::prelude::*;
view! { node(Markdown::new(source).images(&resolver, support, &layer)) }
```

A host driving `MarkdownState::lines` itself reads `MarkdownState::images()` and
paints each `MarkdownImage` at its `rect(area)`. See
[Images](features.md#images-kitty-iterm2--sixel-graphics-protocols) for the
protocols and the alt-text fallback.

## Styling

Every markdown element resolves its look through the active `StyleSheet` —
`heading`, `strong`, `emphasis`, `strikethrough`, `list_marker`, `link`, and the
`CodeTheme` slots behind fenced code — so markdown inherits the app's
[theme](themes.md) and [stylesheet](styling.md) instead of carrying colors of its
own. Restyling the app restyles the transcript.

## See also

- [Component gallery](components.md) — every component, with a demo each.
- [Terminal features](features.md) — hyperlinks, images, and the rest of the
  out-of-band terminal surface.
- [API documentation](https://docs.rs/tuika/latest/tuika/components/markdown/index.html)
  — `Markdown`, `MarkdownState`, `to_lines`, `ImageResolver`.
- [Runnable example](../examples/markdown.rs) — `cargo run --example markdown`
  streams a document in a real terminal.
