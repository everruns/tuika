use crate::*;

fn labeled_row(glyph: Element, label: &str, theme: &Theme) -> Element {
    let text = Text::new(vec![Line::from(Span::styled(
        label.to_string(),
        theme.text_style(),
    ))]);
    view! {
        row(gap = 1) {
            fixed(2) { node(glyph) }
            grow(1) { node(text) }
        }
    }
}

pub(crate) fn scene_spinner(frame: u64, theme: &Theme) -> Element {
    view! {
        col(gap = 1) {
            fixed(1) { node(labeled_row(element(Spinner::new(frame).style(SpinnerStyle::Braille)), "Braille — the smooth default", theme)) }
            fixed(1) { node(labeled_row(element(Spinner::new(frame).style(SpinnerStyle::Line)), "Line — ASCII fallback", theme)) }
            fixed(1) { node(labeled_row(element(Spinner::new(frame).style(SpinnerStyle::Dots)), "Dots — bouncing", theme)) }
        }
    }
}

pub(crate) fn scene_progress(frame: u64, theme: &Theme) -> Element {
    let animated = tuika::anim::ping_pong(frame, 140);
    let _ = theme;
    view! {
        col(gap = 1) {
            fixed(1) { node(ProgressBar::determinate(0.25).percent(true)) }
            fixed(1) { node(ProgressBar::determinate(0.60).label("0:42 / 3:07").percent(true)) }
            fixed(1) { node(ProgressBar::determinate(animated).percent(true)) }
            fixed(1) { node(ProgressBar::indeterminate(frame)) }
        }
    }
}

pub(crate) fn scene_activity_list(frame: u64, theme: &Theme) -> Element {
    let _ = theme;
    let progress = tuika::anim::ping_pong(frame, 180);
    element(
        ActivityList::new(vec![
            ActivityItem::new("Resolve dependencies", ActivityStatus::Succeeded)
                .detail("42 crates"),
            ActivityItem::new("Compile workspace", ActivityStatus::Running)
                .detail("tuika")
                .progress(progress),
            ActivityItem::new("Run tests", ActivityStatus::Queued),
            ActivityItem::new("Publish artifacts", ActivityStatus::Skipped),
        ])
        .frame(frame)
        .gap(1),
    )
}

pub(crate) fn scene_loader(frame: u64, theme: &Theme) -> Element {
    let _ = theme;
    view! {
        col(gap = 1) {
            fixed(1) { node(Loader::new(frame, "compiling crate…").hint("esc to cancel")) }
            fixed(1) { node(Loader::new(frame, "fetching dependencies…").spinner_style(SpinnerStyle::Line)) }
        }
    }
}

pub(crate) fn scene_text(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let styled = Text::new(vec![
        Line::from(vec![
            Span::styled(
                "error",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(": unresolved import ", theme.text_style()),
            Span::styled("`tuika::Widget`", theme.accent_style()),
        ]),
        Line::from(Span::styled(
            "  perhaps you meant `tuika::View`?",
            theme.muted_style(),
        )),
    ]);
    let prose = tuika::components::Paragraph::new(
        "Paragraph word-wraps plain text to the render width in a single style, \
         re-flowing every frame so a resize just re-wraps.",
        theme.text_style(),
    );
    view! {
        col(gap = 1) {
            fixed(2) { node(styled) }
            grow(1) { node(prose) }
        }
    }
}

/// The document the `markdown` scene streams in, one glyph at a time.
const MARKDOWN_DOC: &str = "\
# Streaming Markdown

Renders **CommonMark** as it *streams* — only the in-flight tail
re-parses, so settled blocks and `code` never re-tokenize.

- headings, **bold**, *italic*, `inline code`
- nested lists and tables

```rust
fn greet(name: &str) {
    println!(\"hello, {name}!\");
}
```
";

/// Animated: reveal `MARKDOWN_DOC` progressively through a `MarkdownState`, the
/// same way a host feeds an assistant message as it streams. Holds on the full
/// document, then restarts.
pub(crate) fn scene_markdown(frame: u64, theme: &Theme) -> Element {
    let _ = theme;
    let total = MARKDOWN_DOC.chars().count();
    // Reveal briskly so the whole document — including the highlighted code
    // block — lands within a short recording, then hold before the cycle repeats.
    let pos = (frame as usize * 6) % (total + 120);
    let revealed: String = MARKDOWN_DOC.chars().take(pos.min(total)).collect();

    let mut state = MarkdownState::new();
    state.set(revealed);
    // Rendered at a fixed width for the demo frame; a real host passes the live
    // viewport width so prose re-wraps on resize.
    let sheet = tuika::StyleSheet::from_theme(theme);
    let lines = state
        .lines(
            64,
            theme,
            &sheet,
            tuika::highlight::CodeHighlighter::With(&HL),
        )
        .to_vec();
    element(Text::new(lines))
}

/// The GFM table the `markdown_table` scene renders.
///
/// Deliberately exercises everything a table cell can carry: per-column
/// alignment from the `:---:` markers, inline code and bold, wide emoji (which
/// must be measured grapheme-aware or the borders drift), and links whose label
/// is what gets painted while the destination rides along as OSC 8.
const MARKDOWN_TABLE_DOC: &str = "\
| Component   |  Status   |                          Docs |
| :---------- | :-------: | ----------------------------: |
| `Markdown`  | ✅ stable | [docs.rs](https://docs.rs/tuika) |
| `CodeBlock` | ✅ stable | \
[gallery](https://github.com/everruns/tuika/blob/main/docs/components.md) |
| **Image**   |  🚧 beta  | \
[features](https://github.com/everruns/tuika/blob/main/docs/features.md) |
";

/// A GFM table rendered by the one-shot `Markdown` view.
///
/// Column widths come from the content and are fitted to the area, so the same
/// source reflows on resize rather than being pre-formatted by the host.
pub(crate) fn scene_markdown_table(frame: u64, theme: &Theme) -> Element {
    let _ = (frame, theme);
    element(Markdown::new(MARKDOWN_TABLE_DOC))
}

const MARKDOWN_HTML_DOC: &str = "\
### Inline HTML

Markdown in the wild carries HTML, so the presentational inline
tags render instead of being dropped — each resolving the same
`StyleSheet` role as the markdown it mirrors.

- <b>strong</b> and <em>emphasis</em>, <s>struck</s>, <u>underlined</u>
- <mark>highlighted</mark>, <kbd>Ctrl</kbd>+<kbd>C</kbd>, <a href=\"https://docs.rs/tuika\">a link</a>
- H<sub>2</sub>O and 3&times;10<sup>8</sup> m/s

A line<br>broken by a tag.
";

/// Inline HTML inside markdown: every tag resolves a `StyleSheet` role, so
/// `<b>` cannot look different from `**bold**`.
pub(crate) fn scene_markdown_html(frame: u64, theme: &Theme) -> Element {
    let _ = (frame, theme);
    element(Markdown::new(MARKDOWN_HTML_DOC))
}

/// A single themed, syntax-highlighted fenced block via `CodeBlock`.
pub(crate) fn scene_code_block(frame: u64, theme: &Theme) -> Element {
    let _ = (frame, theme);
    let source = "pub fn fib(n: u64) -> u64 {\n    match n {\n        0 | 1 => n,\n        _ => fib(n - 1) + fib(n - 2),\n    }\n}";
    element(
        CodeBlock::new("rust", source)
            .highlighter(&HL)
            .line_numbers(true),
    )
}

pub(crate) fn scene_rule(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    view! {
        col(gap = 1) {
            fixed(1) { node(Rule::new().style(theme.muted_style())) }
            fixed(1) { node(Rule::new().title(Line::from(Span::styled(" Section ", theme.accent_style()))).style(theme.muted_style())) }
            fixed(1) { node(Rule::new().glyph('┈').style(theme.muted_style())) }
            fixed(1) { node(Rule::new().title(Line::from(Span::styled(" dotted ", theme.accent_style()))).glyph('·').style(theme.muted_style())) }
        }
    }
}

pub(crate) fn scene_boxed(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let inner = Text::new(vec![Line::from(Span::styled(
        "border + padding + title",
        theme.text_style(),
    ))]);
    let plain = Text::new(vec![Line::from(Span::styled(
        "rounded border",
        theme.text_style(),
    ))]);
    view! {
        col(gap = 1) {
            fixed(3) {
                boxed(title = Line::from(Span::styled(" thick ", theme.accent_style())), border = BorderStyle::Thick, padding = Padding::symmetric(1, 0)) {
                    node(inner)
                }
            }
            fixed(3) {
                boxed(title = Line::from(Span::styled(" rounded ", theme.accent_style())), border = BorderStyle::Rounded) {
                    node(plain)
                }
            }
        }
    }
}
