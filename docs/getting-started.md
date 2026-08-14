---
title: Getting started
description: Create and run a small Rust terminal application with tuika, then learn where application state, views, and input handling belong.
sidebar:
  order: 1
---

# Getting started

This builds a complete terminal application with tuika: it enters the alternate
screen, renders a small view, handles input, and restores the terminal on exit.

## Create the project

```sh
cargo new hello-tuika
cd hello-tuika
cargo add tuika
```

Replace `src/main.rs` with:

```rust
use std::io;

use tuika::prelude::*;

fn main() -> io::Result<()> {
    let theme = Theme::default();
    let runner = Runner::new(RunnerConfig::default());

    runner.run(
        &theme,
        from_fn(
            &mut (),
            |_state, _frame| {
                view! {
                    col(padding = Padding::all(1)) {
                        fixed(3) {
                            boxed(title = " tuika ") {
                                text("Hello from the terminal")
                            }
                        }
                        grow(1) { spacer() }
                        fixed(1) { text("q or esc to quit") }
                    }
                }
            },
            |_state, signal| match signal {
                Signal::Event(Event::Key(key))
                    if key.plain() && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) =>
                {
                    UpdateResult::Exit
                }
                _ => UpdateResult::Clean,
            },
        ),
    )
}
```

Run it:

```sh
cargo run
```

You should see a bordered panel and a quit hint. Press `q` or `Esc` to return to
your shell.

## What the example does

- `Runner` owns the terminal session and event loop.
- The first closure builds a view from application state for each frame.
- `view!` describes layout; it expands to the ordinary tuika builders.
- The second closure handles input and says whether to redraw or exit.
- ratatui remains underneath, owning the cell buffer and its terminal diff.

For stateful input, replace `()` with an application struct and keep selection,
scroll, focus, or text-input state there. Views borrow that state for a frame;
tuika does not maintain a hidden component tree.

## When to use tuika

Use tuika when an application needs structure around ratatui: layout, focus,
keymaps, overlays, components, or terminal lifecycle. For a small screen made
from a few widgets, ratatui alone may be enough. Existing ratatui widgets remain
usable through `RatatuiView`.

## Next

- [Layout](layout.md) — compose responsive screens with Flex, Flow, and Grid.
- [Components](components.md) — find a view and see its recorded output.
- [Markdown](markdown.md) — render streaming CommonMark, code, and tables.
- [Keymap](keymap.md) — map keys and sequences to application commands.
- [Terminal features](features.md) — screen modes, images, mouse, and clipboard.
