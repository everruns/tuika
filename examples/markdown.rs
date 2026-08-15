//! Streaming Markdown renderer. Run with `cargo run --example markdown`
//! (space pause/resume · r restart · PgUp/PgDn/Home/End scroll ·
//! q or esc quit).
//!
//! Demonstrates [`MarkdownState`]: a rich CommonMark document is fed in a few
//! glyphs per frame — exactly how a host feeds an assistant message as it
//! streams — while only the in-flight tail re-parses each frame. Fenced code is
//! syntax-highlighted through a tiny self-contained [`Highlighter`](tuika::highlight::Highlighter)
//! (see `DemoHighlighter` below); for production highlighting across many
//! languages, use the `tuika-codeformatters` crate. The document's URL is a
//! native OSC 8 link; pass `--mouse` to trade native link activation for
//! application mouse-wheel events.
//!
//! # Following a stream
//!
//! A streamed document outgrows the viewport, so this also shows the pattern
//! every streaming host needs: the view **follows the newest content, until the
//! reader scrolls away from it**. Scrolling back must not be yanked to the
//! bottom by the next delta, and returning to the bottom must resume following.
//!
//! That is two calls, not a mechanism:
//!
//! - [`ScrollState::clamp`] once per frame reconciles the offset with the
//!   current content and viewport heights, pinning it to the bottom *while the
//!   state is stuck there*. Appending content therefore scrolls it into view on
//!   its own.
//! - [`ScrollState::handle`] releases that stick when the reader scrolls up, and
//!   re-arms it when they reach the bottom again.
//!
//! [`ScrollState::is_stuck_to_bottom`] reads the flag back, which is how the
//! status bar below can say whether it is live or held.

use std::io;
use std::time::Duration;

use crossterm::event::{self};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::{Terminal, TerminalOptions, Viewport};

use tuika::prelude::*;
use tuika::term::hyperlink::{HyperlinkBackend, LinkPolicy, apply_buffer_links};

/// A tiny self-contained Rust highlighter — enough to satisfy the
/// [`Highlighter`] seam without a grammar dependency. Real hosts use the
/// `tuika-codeformatters` crate (tree-sitter, many languages).
struct DemoHighlighter;

impl Highlighter for DemoHighlighter {
    fn highlight(
        &self,
        lang: &str,
        lines: &[&str],
        theme: &Theme,
    ) -> Option<Vec<Vec<Span<'static>>>> {
        if lang != "rust" {
            return None;
        }
        Some(
            lines
                .iter()
                .map(|l| highlight_rust(l, &theme.code))
                .collect(),
        )
    }
}

/// Split one line of Rust into styled spans, reconstructing it exactly.
fn highlight_rust(line: &str, code: &CodeTheme) -> Vec<Span<'static>> {
    const KW: &[&str] = &[
        "fn", "let", "mut", "pub", "use", "struct", "enum", "impl", "match", "return", "for", "in",
        "if", "else", "while", "loop", "const", "as", "mod", "trait", "self", "true", "false",
    ];
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            let rest: String = chars[i..].iter().collect();
            out.push(Span::styled(
                rest,
                Style::default()
                    .fg(code.comment)
                    .add_modifier(Modifier::ITALIC),
            ));
            break;
        }
        if c == '"' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i = (i + 1).min(chars.len());
            let s: String = chars[start..i].iter().collect();
            out.push(Span::styled(s, Style::default().fg(code.string)));
            continue;
        }
        if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let style = if KW.contains(&word.as_str()) {
                Style::default()
                    .fg(code.keyword)
                    .add_modifier(Modifier::BOLD)
            } else if word.chars().all(|d| d.is_ascii_digit()) {
                Style::default().fg(code.constant)
            } else {
                Style::default().fg(code.text)
            };
            out.push(Span::styled(word, style));
            continue;
        }
        let start = i;
        i += 1;
        while i < chars.len() {
            let d = chars[i];
            if d.is_alphanumeric()
                || d == '_'
                || d == '"'
                || (d == '/' && chars.get(i + 1) == Some(&'/'))
            {
                break;
            }
            i += 1;
        }
        let s: String = chars[start..i].iter().collect();
        let style = if s.trim().is_empty() {
            Style::default().fg(code.text)
        } else {
            Style::default().fg(code.punctuation)
        };
        out.push(Span::styled(s, style));
    }
    out
}

const SOURCE: &str = "\
# Release checklist

Ship **only** when CI is green. The renderer streams *this very document*
in as it is typed — see <https://docs.rs/tuika>.

## Steps

1. `cargo test --all-features`
   - and `cargo clippy -- -D warnings`
2. Rebase onto `origin/main`
3. Squash & merge

- [x] changelog updated
- [ ] demos re-recorded

| Stage  | Owner | Gate        |
| ------ | ----- | ----------- |
| build  | ci    | clippy      |
| verify | you   | tests green |

```rust
fn main() {
    let ready = check_ci() && tests_pass();
    assert!(ready, \"do not ship red CI\");
}
```

> Never merge red CI.
";

fn main() -> io::Result<()> {
    let highlighter = DemoHighlighter;
    let doc: Vec<char> = SOURCE.chars().collect();

    let mut state = MarkdownState::new();
    let mut scroll = ScrollState::new();
    let mut cursor = 0usize;
    let mut paused = false;

    let capture_mouse = std::env::args().any(|argument| argument == "--mouse");
    let mode = if capture_mouse {
        ScreenMode::Alternate.with_mouse_capture()
    } else {
        ScreenMode::Alternate
    };
    let _session = TerminalSession::enter_with(mode)?;
    let mut terminal = Terminal::with_options(
        HyperlinkBackend::new(io::stdout(), true),
        TerminalOptions {
            viewport: Viewport::Fullscreen,
        },
    )?;
    let theme = Theme::default();
    let sheet = tuika::StyleSheet::from_theme(&theme);

    loop {
        // Advance the stream: a few glyphs per frame until the document is fully
        // in, then hold. `push_str` is the delta a host would forward verbatim.
        if !paused && cursor < doc.len() {
            // Fast enough that the document outgrows a normal viewport within a
            // few seconds, which is when the follow behavior becomes visible.
            let end = (cursor + 6).min(doc.len());
            let chunk: String = doc[cursor..end].iter().collect();
            state.push_str(&chunk);
            cursor = end;
        }

        // Lay the document out *before* drawing: the scroll state has to
        // reconcile against the content and viewport heights, and neither is
        // known until the lines exist.
        let size = terminal.size()?;
        let width = size.width.saturating_sub(4);
        let lines = state
            .lines(width, &theme, &sheet, CodeHighlighter::With(&highlighter))
            .to_vec();
        let content_h = lines.len();
        // The column's own chrome: one row of padding top and bottom, the gap,
        // and the status bar.
        let viewport_h = usize::from(size.height.saturating_sub(4));

        // The whole follow-while-streaming behavior. `clamp` pins the offset to
        // the newest content while the state is stuck to the bottom, and leaves
        // a reader who scrolled back exactly where they were.
        scroll.clamp(content_h, viewport_h);
        let visible_links = state
            .links()
            .iter()
            .filter_map(|link| {
                let line = usize::from(link.line);
                (line >= scroll.offset() && line < scroll.offset().saturating_add(viewport_h)).then(
                    || {
                        let mut visible = link.clone();
                        visible.line = u16::try_from(line - scroll.offset()).unwrap_or(u16::MAX);
                        visible
                    },
                )
            })
            .collect::<Vec<_>>();

        terminal.draw(|f| {
            let area = f.area();
            let status = if cursor < doc.len() {
                format!("streaming… {}/{} glyphs", cursor, doc.len())
            } else {
                "done".to_string()
            };
            // Reading the stick back is what lets a host tell the reader whether
            // they are live or holding a position.
            let follow = if scroll.is_stuck_to_bottom() {
                Span::styled(" following ", theme.selection_style())
            } else {
                Span::styled(" scrolled back — End to follow ", theme.warning_style())
            };
            let bar = StatusBar::new()
                .left(vec![
                    Span::styled(format!(" {status} "), theme.selection_style()),
                    follow,
                ])
                .right(vec![Span::styled(
                    "space pause · r restart · pgup/pgdn scroll · q quit ",
                    theme.muted_style(),
                )])
                .background(Style::default().bg(theme.surface));
            let root = view! {
                col(padding = Padding::all(1), gap = 1) {
                    grow(1) { node(Scroll::new(lines, &scroll)) }
                    fixed(1) { node(bar) }
                }
            };
            paint(f.buffer_mut(), area, &theme, root.as_ref(), &[]);
            apply_buffer_links(
                f.buffer_mut(),
                ratatui::layout::Position {
                    x: area.x.saturating_add(1),
                    y: area.y.saturating_add(1),
                },
                &visible_links,
                LinkPolicy::WEB,
            );
        })?;

        if !event::poll(Duration::from_millis(70))? {
            continue;
        }
        let Some(event) = translate_event(event::read()?) else {
            continue;
        };
        if let Event::Key(k) = &event
            && k.plain()
        {
            match k.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char(' ') => paused = !paused,
                KeyCode::Char('r') => {
                    state = MarkdownState::new();
                    scroll = ScrollState::new();
                    cursor = 0;
                    paused = false;
                }
                _ => {}
            }
        }
        // PgUp/PgDn/Home/End and the wheel; scrolling up releases the stick,
        // reaching the bottom re-arms it.
        let _ = scroll.handle(&event, content_h, viewport_h);
    }

    let _ = terminal.clear();
    drop(terminal);
    Ok(())
}
