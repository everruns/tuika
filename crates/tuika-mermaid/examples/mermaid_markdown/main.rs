use tuika::prelude::*;
use tuika_mermaid::MermaidRenderer;

#[path = "../support/mod.rs"]
mod support;

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

struct App {
    renderer: MermaidRenderer,
}

impl Application for App {
    fn update(&mut self, signal: Signal) -> UpdateResult {
        match signal {
            Signal::Event(Event::Key(key))
                if key.plain() && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) =>
            {
                UpdateResult::Exit
            }
            _ => UpdateResult::Clean,
        }
    }

    fn view(&self, _frame: u64) -> ScopedElement<'_> {
        element(Markdown::new(DOCUMENT).block_renderer(&self.renderer))
    }
}

fn main() -> std::io::Result<()> {
    let (theme, args) = support::theme_and_args()?;
    if !args.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "usage: cargo run -p tuika-mermaid --example mermaid_markdown [-- --theme NAME]",
        ));
    }
    let mut app = App {
        renderer: MermaidRenderer::new(),
    };
    Runner::new(RunnerConfig::default()).run(&theme, &mut app)
}
