use tuika::Theme;
use tuika::components::Markdown;
use tuika::testing::{grid, render};
use tuika_mermaid::MermaidRenderer;

const DOCUMENT: &str = "\
# Native Mermaid

The fenced blocks below are laid out as Unicode cells:

## Flowchart

```mermaid
flowchart LR
  Source[Markdown] --> Parse
  Parse --> Layout
  Layout --> Paint[Terminal cells]
```

## Sequence diagram

```mermaid
sequenceDiagram
  participant Host
  participant Markdown
  participant Renderer
  Host->>Markdown: Render source
  Markdown->>Renderer: Mermaid fence
  Renderer-->>Markdown: Unicode cells
  Markdown-->>Host: Composed frame
```
";

fn main() {
    let theme = Theme::default();
    let mermaid = MermaidRenderer::new();
    let document = Markdown::new(DOCUMENT).block_renderer(&mermaid);
    let buffer = render(&document, 80, 30, &theme);
    let output = grid(&buffer)
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    println!("{}", output.trim_end());
}
