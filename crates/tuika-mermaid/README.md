# tuika-mermaid

Terminal-native Mermaid diagrams inside
[`tuika`](https://crates.io/crates/tuika) Markdown. The crate adapts
[`mmdflux`](https://crates.io/crates/mmdflux) to tuika's generic
`MarkdownBlockRenderer` seam; diagrams are Unicode cells, not images, and
require no browser or JavaScript runtime.

```rust
use tuika::components::Markdown;
use tuika_mermaid::MermaidRenderer;

let mermaid = MermaidRenderer::new();
let document = Markdown::new(
    "```mermaid\nflowchart LR\n  Parse --> Layout --> Paint\n```",
)
.block_renderer(&mermaid);

# let _ = document;
```

`MermaidRenderer` handles `mermaid` fences. A diagram wider than the available
columns is re-laid out at tighter node separation until it fits, best-effort —
a graph with many parallel branches can be irreducibly wider than the terminal.
Unsupported syntax and fences over 64 KiB fall back to tuika's ordinary themed
code-block presentation, so malformed or oversized content remains visible.

<img src="examples/mermaid_markdown/mermaid.gif" width="880" alt="Mermaid diagram rendered as Unicode cells inside tuika Markdown">

Run the integration example:

```sh
cargo run -p tuika-mermaid --example mermaid_markdown
```
