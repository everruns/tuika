use tuika::Theme;
use tuika::components::Markdown;
use tuika::testing::{grid, render};
use tuika_mermaid::MermaidRenderer;

const DOCUMENT: &str = "\
# Native Mermaid

The fenced block below is laid out as Unicode cells:

```mermaid
flowchart LR
  Source[Markdown] --> Parse
  Parse --> Layout
  Layout --> Paint[Terminal cells]
```
";

fn main() {
    let theme = Theme::default();
    let mermaid = MermaidRenderer::new();
    let document = Markdown::new(DOCUMENT).block_renderer(&mermaid);
    let buffer = render(&document, 80, 12, &theme);
    let output = grid(&buffer)
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    println!("{}", output.trim_end());
}
