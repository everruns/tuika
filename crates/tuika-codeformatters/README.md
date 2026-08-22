# tuika-codeformatters

[![crates.io](https://img.shields.io/crates/v/tuika-codeformatters.svg)](https://crates.io/crates/tuika-codeformatters)
[![docs.rs](https://img.shields.io/docsrs/tuika-codeformatters)](https://docs.rs/tuika-codeformatters)

Tree-sitter syntax highlighting for [`tuika`](https://crates.io/crates/tuika)'s
`CodeBlock` and `Markdown` components.

`tuika` owns the *presentation* of code — framing, background, language label,
wrapping — but deliberately depends on no grammar, so its dependency set stays
small. This companion crate fills the gap: a ready-made `tuika::Highlighter`
backed by tree-sitter grammars, mapping token classes onto the host `Theme`'s
`code` palette so highlighted code follows the theme.

```rust
use tuika::{CodeBlock, Theme};
use tuika_codeformatters::TreeSitterHighlighter;

let theme = Theme::default();
let hl = TreeSitterHighlighter::new();
let block = CodeBlock::new("rust", "fn main() {}").highlighter(&hl);
// `block` is a `View`; render it with `tuika::paint` or embed it in a `Flex`.
# let _ = (theme, block);
```

## Examples

A runnable, interactive gallery of highlighted snippets across languages
(←/→ or Tab to switch, `q` to quit):

```bash
cargo run -p tuika-codeformatters --example languages
cargo run -p tuika-codeformatters --example languages -- --theme gruvbox-dark
```

<img src="https://raw.githubusercontent.com/everruns/tuika/main/crates/tuika-codeformatters/docs/languages.gif" width="880" alt="Syntax highlighting across languages">

To open a local source file in a scrollable highlighted viewer:

```bash
cargo run -p tuika-codeformatters --example highlight_file
cargo run -p tuika-codeformatters --example highlight_file -- path/to/file.rs
```

The viewer detects the language from the file extension and falls back to plain
code for unknown extensions. Use ↑/↓, j/k, Page Up/Page Down, Home/End, or
the mouse wheel to scroll. Drag selects and copies rendered code; `q` or Esc
quits.

## Supported languages

Rust, Python, TypeScript/JavaScript, TSX/JSX, Go, Java, Ruby, CSS, HTML, C#,
PHP, Zig, Scala, and SQL (with common aliases: `rs`, `py`, `ts`, `js`, `rb`,
`c#`, …). Unknown languages — or source that fails to parse — return `None`, and
the caller renders the block as plain, theme-colored code.

### Choosing grammars

Every grammar is on by default, and each has a cargo feature. A grammar is a
multi-megabyte parse table, so a binary that highlights everything carries
~21 MiB of them — C# alone is ~5 MiB. A host that knows its languages keeps only
those:

```toml
[dependencies]
tuika-codeformatters = { version = "0.4", default-features = false, features = ["rust", "python"] }
```

Feature names match the language keys above, lowercased and without punctuation:
`rust`, `python`, `typescript` (covers TSX/JSX too), `go`, `java`, `ruby`,
`css`, `html`, `csharp`, `php`, `zig`, `scala`, `sql`.

A language whose feature is off behaves exactly like one this crate never
supported: it returns `None` and the caller renders plain code.

## Compatibility

`ratatui` and `tuika` are part of this crate's public interface, so pin the same
minor versions in your own crate and Cargo will deduplicate them.

## License

MIT
