//! Streaming Markdown rendering.
//!
//! [`MarkdownState`] renders CommonMark (via `pulldown-cmark`) to styled
//! [`Line`]s, incrementally: as a message streams in, only the **in-flight tail**
//! is re-parsed each frame. Everything before the last *stable block boundary*
//! (a blank line outside an open code fence) is parsed and highlighted once and
//! cached — so a long transcript does not re-tokenize, and tree-sitter does not
//! re-highlight settled code blocks, on every delta. This mirrors the split
//! Hermes' TUI uses for its streaming markdown.
//!
//! Output is width-aware and correct for prose, code, and tables: prose is
//! word-wrapped (via [`components::wrap_lines`](crate::components::text::wrap_lines));
//! code is emitted verbatim, because its
//! indentation is meaningful; and GFM tables are re-laid-out to the width each
//! frame, with per-column fitting and styled cells (bold headers, links, inline
//! code, emoji). Callers draw the returned lines **without** further wrapping
//! (e.g. ratatui's `Paragraph` with no `.wrap`, or tuika's
//! [`Text`](crate::components::Text)).
//!
//! For one-shot (non-streaming) text, [`to_lines`] renders a whole
//! string in one call. The [`Markdown`] view wraps either for direct placement
//! in a layout.
//!
//! # Layout
//!
//! Rendering is a two-pass pipeline, and the files follow it: source is parsed
//! into a width-*independent* intermediate form, then flattened against a
//! concrete width. That split is what makes streaming affordable — a settled
//! block is parsed once and re-flattened only when the width changes.
//!
//! | File | Owns |
//! | --- | --- |
//! | `item` | the intermediate form both passes speak: `MdItem`, `RichSpan`, `TableData` |
//! | `parse` | pass one — pulldown-cmark events to `MdItem`s, including inline styling and link detection |
//! | `flatten` | pass two — `MdItem`s to width-fitted `Line`s: wrapping, indentation, hyperlink runs |
//! | `table` | table layout, called from `flatten`; big enough to drown the rest of pass two |
//! | `stream` | [`MarkdownState`] and the settled-prefix cache that makes streaming O(delta) |
//! | `image` | the [`ImageResolver`] seam and cell sizing for block images |
//! | `view` | the [`Markdown`] view over either entry point |
//!
//! The submodules are private: `components::markdown` is the one path in, and
//! `to_lines`, `MarkdownState`, and `Markdown` are what it exposes.

use ratatui_core::text::Line;

use crate::highlight::CodeHighlighter;
use crate::style::{StyleSheet, Theme};
use crate::term::hyperlink::BufferLink;

mod flatten;
mod image;
mod item;
mod parse;
mod stream;
mod table;
#[cfg(test)]
mod testutil;
mod view;

pub use image::{ImageResolver, MarkdownImage};
pub use stream::MarkdownState;
pub use view::Markdown;

use flatten::flatten_linked;
use parse::parse;

/// Replaces a fenced code block with width-aware rendered lines.
///
/// This is the extension seam for languages whose presentation is more than
/// syntax coloring: diagrams, mathematical notation, query plans, and similar
/// block content. Implementations inspect `language` and return `None` when they
/// do not handle it (or when rendering fails); markdown then falls back to the
/// ordinary themed [`CodeBlock`](crate::components::CodeBlock).
///
/// Renderers should be deterministic for `(language, source, width, theme)`.
/// [`MarkdownState`] caches settled blocks and calls the renderer again only
/// when a block is still streaming, the width changes, or the theme changes.
pub trait FencedBlockRenderer {
    /// Render a fenced block, or return `None` to use the normal code-block
    /// presentation.
    fn render(
        &self,
        language: &str,
        source: &str,
        width: u16,
        theme: &Theme,
    ) -> Option<Vec<Line<'static>>>;
}

/// Render a whole markdown string to width-fitted styled lines in one call.
///
/// For streaming input, prefer [`MarkdownState`], which caches the settled
/// prefix instead of re-parsing the whole buffer each frame.
pub fn to_lines(
    source: &str,
    width: u16,
    theme: &Theme,
    sheet: &StyleSheet,
    highlighter: CodeHighlighter,
) -> Vec<Line<'static>> {
    to_linked_lines(source, width, theme, sheet, highlighter).0
}

/// Render markdown with a host-supplied fenced-block renderer.
///
/// The renderer can replace any language fence with width-aware lines; returning
/// `None` preserves the ordinary [`CodeBlock`](crate::components::CodeBlock).
pub fn to_lines_with_renderer(
    source: &str,
    width: u16,
    theme: &Theme,
    sheet: &StyleSheet,
    highlighter: CodeHighlighter,
    block_renderer: &dyn FencedBlockRenderer,
) -> Vec<Line<'static>> {
    to_linked_lines_with_renderer(source, width, theme, sheet, highlighter, block_renderer).0
}

/// Like [`to_lines`], but also returns [`BufferLink`]s for every
/// hyperlink run (labeled `[text](url)` and bare URLs) after wrapping.
///
/// Apply them with [`apply_buffer_links`](crate::term::hyperlink::apply_buffer_links) after painting the lines so OSC 8 /
/// Ctrl+click can open the destination even when the visible label is not the
/// URL. Pass [`LinkPolicy::NONE`](crate::term::hyperlink::LinkPolicy::NONE) to [`apply_buffer_links`](crate::term::hyperlink::apply_buffer_links) to skip emission.
pub fn to_linked_lines(
    source: &str,
    width: u16,
    theme: &Theme,
    sheet: &StyleSheet,
    highlighter: CodeHighlighter,
) -> (Vec<Line<'static>>, Vec<BufferLink>) {
    let items = parse(source, theme, sheet, highlighter);
    flatten_linked(&items, width, theme, None)
}

/// Like [`to_linked_lines`], with a host-supplied [`FencedBlockRenderer`].
pub fn to_linked_lines_with_renderer(
    source: &str,
    width: u16,
    theme: &Theme,
    sheet: &StyleSheet,
    highlighter: CodeHighlighter,
    block_renderer: &dyn FencedBlockRenderer,
) -> (Vec<Line<'static>>, Vec<BufferLink>) {
    let items = parse(source, theme, sheet, highlighter);
    flatten_linked(&items, width, theme, Some(block_renderer))
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;
    use crate::highlight::CodeHighlighter;
    use crate::style::StyleBundle;
    use crate::style::{StyleSheet, Theme};
    use crate::term::hyperlink::LinkPolicy;
    use ratatui_core::text::Span;

    use ratatui_core::layout::Rect;
    use ratatui_core::style::Modifier;

    #[test]
    fn heading_is_bold_and_themed() {
        let theme = Theme::default();
        let lines = to_lines(
            "# Title",
            40,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        );
        let span = &lines[0].spans[0];
        assert_eq!(span.content.as_ref(), "Title");
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(span.style.fg, Some(theme.code.heading));
    }

    #[test]
    fn emphasis_and_strong_carry_modifiers() {
        let theme = Theme::default();
        let lines = to_lines(
            "plain *em* and **bold**",
            60,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        );
        let em = lines[0]
            .spans
            .iter()
            .find(|s| s.content.contains("em"))
            .expect("emphasis span");
        assert!(em.style.add_modifier.contains(Modifier::ITALIC));
        let bold = lines[0]
            .spans
            .iter()
            .find(|s| s.content.contains("bold"))
            .expect("strong span");
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn inline_code_gets_code_background() {
        let theme = Theme::default();
        let lines = to_lines(
            "use `cargo test` now",
            60,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        );
        let code = lines[0]
            .spans
            .iter()
            .find(|s| s.content.contains("cargo test"))
            .expect("inline code span");
        assert_eq!(code.style.bg, Some(theme.code.background));
    }

    #[test]
    fn bullet_list_renders_markers() {
        let out = plain("- one\n- two", 40);
        assert!(out.iter().any(|l| l.contains("• one")), "{out:?}");
        assert!(out.iter().any(|l| l.contains("• two")), "{out:?}");
    }

    #[test]
    fn ordered_list_numbers_increment() {
        let out = plain("1. first\n2. second", 40);
        assert!(out.iter().any(|l| l.contains("1. first")), "{out:?}");
        assert!(out.iter().any(|l| l.contains("2. second")), "{out:?}");
    }

    #[test]
    fn nested_list_is_indented() {
        let out = plain("- outer\n  - inner", 40);
        let inner = out.iter().find(|l| l.contains("inner")).unwrap();
        assert!(
            inner.starts_with("  "),
            "nested item should be indented: {inner:?}"
        );
    }

    #[test]
    fn task_list_checkboxes_are_themed_markers() {
        // The checkbox is a marker, not prose: it takes the `task_marker` slot,
        // which stayed unreachable while the parser option was off.
        let theme = Theme::default();
        let sheet = StyleSheet::from_theme(&theme);
        let lines = to_lines(
            "- [ ] todo\n- [x] done",
            40,
            &theme,
            &sheet,
            CodeHighlighter::Plain,
        );
        let marker = lines
            .iter()
            .flat_map(|l| &l.spans)
            .find(|s| s.content.contains("[x]"))
            .expect("checked marker span");
        assert_eq!(marker.style.fg, sheet.task_marker.to_style().fg);
        let out: Vec<String> = lines.iter().map(text).collect();
        assert!(
            out.iter().any(|l| l.contains("[ ] todo")),
            "unchecked item: {out:?}"
        );
    }

    #[test]
    fn nested_list_starts_its_own_line() {
        // A tight item carries no `Paragraph`, so the nested list used to open
        // while the parent's text was still buffered — gluing the two together
        // as "• outerinner".
        let out = plain("- outer\n  - inner", 40);
        assert!(
            out.iter().any(|l| l.trim() == "• outer"),
            "parent item keeps its own line: {out:?}"
        );
        assert!(
            out.iter().any(|l| l.trim() == "• inner"),
            "nested item keeps its own line: {out:?}"
        );
    }

    #[test]
    fn block_inside_tight_item_follows_the_item_text() {
        // Same flush, for the other block kinds a tight item can open: the quote
        // must not join the item's line, and the fence must land after — not
        // ahead of — the item it belongs to.
        let out = plain("- item\n  > quoted\n- next\n  ```\n  code\n  ```", 40);
        let at = |needle: &str| {
            out.iter()
                .position(|l| l.contains(needle))
                .unwrap_or_else(|| panic!("missing {needle:?}: {out:?}"))
        };
        assert!(
            out.iter().any(|l| l.trim() == "• item"),
            "item text stays on its own line: {out:?}"
        );
        assert!(
            at("quoted") > at("• item"),
            "quote follows its item: {out:?}"
        );
        assert!(at("code") > at("• next"), "fence follows its item: {out:?}");
    }

    #[test]
    fn loose_item_keeps_its_marker_on_the_first_paragraph() {
        // The flush above must not fire on a bare pending marker, or the bullet
        // would be emitted alone and the paragraph would lose it.
        let out = plain("- one\n\n  more\n\n- two", 40);
        assert!(
            out.iter().any(|l| l.trim() == "• one"),
            "marker rides the first paragraph: {out:?}"
        );
        assert!(
            !out.iter().any(|l| l.trim() == "•"),
            "no marker-only line: {out:?}"
        );
    }

    #[test]
    fn fenced_code_preserves_indentation_verbatim() {
        // A line-oriented wrapper would eat the leading spaces; code must not.
        let src = "```\n    indented\n```";
        let out = plain(src, 40);
        assert!(
            out.iter().any(|l| l.contains("    indented")),
            "code indentation must survive: {out:?}"
        );
    }

    #[test]
    fn fenced_code_shows_language_label() {
        let out = plain("```rust\nfn main() {}\n```", 40);
        assert!(out.iter().any(|l| l.contains("rust")), "{out:?}");
    }

    #[test]
    fn code_fence_is_not_word_wrapped() {
        // A long code line exceeds width but is emitted as a single (clipped)
        // row, never reflowed into multiple lines.
        let long = "x".repeat(60);
        let src = format!("```\n{long}\n```");
        let lines = to_lines(
            &src,
            20,
            &Theme::default(),
            &StyleSheet::default(),
            CodeHighlighter::Plain,
        );
        let code_rows = lines.iter().filter(|l| text(l).contains("xxxx")).count();
        assert_eq!(code_rows, 1, "code line must not wrap");
    }

    struct DiagramRenderer;

    impl FencedBlockRenderer for DiagramRenderer {
        fn render(
            &self,
            language: &str,
            source: &str,
            width: u16,
            theme: &Theme,
        ) -> Option<Vec<Line<'static>>> {
            (language == "diagram").then(|| {
                vec![Line::from(Span::styled(
                    format!("rendered at {width}: {source}"),
                    ratatui_core::style::Style::default().fg(theme.accent),
                ))]
            })
        }
    }

    #[test]
    fn fenced_block_renderer_replaces_recognized_language() {
        let theme = Theme::default();
        let lines = to_lines_with_renderer(
            "```diagram\nA --> B\n```",
            37,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
            &DiagramRenderer,
        );

        assert_eq!(lines.len(), 1);
        assert_eq!(text(&lines[0]), "rendered at 37: A --> B");
        assert_eq!(lines[0].spans[0].style.fg, Some(theme.accent));
    }

    #[test]
    fn fenced_block_renderer_none_preserves_code_block() {
        let theme = Theme::default();
        let lines = to_lines_with_renderer(
            "```rust\nfn main() {}\n```",
            40,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
            &DiagramRenderer,
        );
        let output: Vec<String> = lines.iter().map(text).collect();

        assert!(
            output.iter().any(|line| line.contains("rust")),
            "{output:?}"
        );
        assert!(
            output.iter().any(|line| line.contains("fn main()")),
            "{output:?}"
        );
    }

    #[test]
    fn prose_word_wraps_to_width() {
        let out = plain("one two three four five six seven eight", 12);
        assert!(out.len() > 1, "long prose should wrap: {out:?}");
        for line in &out {
            assert!(line.chars().count() <= 12, "line over width: {line:?}");
        }
    }

    #[test]
    fn bare_url_is_linkified() {
        let theme = Theme::default();
        let lines = to_lines(
            "see https://example.com now",
            60,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        );
        let url = lines[0]
            .spans
            .iter()
            .find(|s| s.content.as_ref().contains("example.com"))
            .expect("url span");
        assert_eq!(url.style.fg, Some(theme.code.link));
        assert!(url.style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn labeled_markdown_link_preserves_destination_for_ctrl_click() {
        // Reproduction of the Ghostty Ctrl+click failure: `[label](url)` was
        // only styled — the destination was dropped — so OSC 8 / ctrl_click had
        // nothing to open under the pointer.
        use crate::term::hyperlink::{apply_buffer_links, ctrl_click_url};
        use crate::{Mouse, MouseButton, MouseKind};
        use ratatui_core::buffer::Buffer;
        use ratatui_core::layout::Position;

        let theme = Theme::default();
        let (lines, links) = to_linked_lines(
            "See the [docs](https://example.com/api) please.",
            60,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        );
        assert!(
            links.iter().any(|l| l.url == "https://example.com/api"),
            "labeled link must yield a BufferLink: {links:?}"
        );
        let link = links
            .iter()
            .find(|l| l.url == "https://example.com/api")
            .unwrap();
        // Visible text is the label, not the URL.
        let plain: String = lines[link.line as usize]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(plain.contains("docs"), "label visible: {plain:?}");
        assert!(
            !plain.contains("example.com"),
            "URL must not replace the label: {plain:?}"
        );

        let area = Rect::new(0, 0, 60, 3);
        let mut buffer = Buffer::empty(area);
        for (row, line) in lines.iter().enumerate() {
            let mut x = 0u16;
            for span in &line.spans {
                x = buffer.set_span(x, row as u16, span, area.width).0;
            }
        }
        apply_buffer_links(
            &mut buffer,
            Position { x: 0, y: 0 },
            &links,
            LinkPolicy::WEB,
        );
        let click_col = link.start_col + 1; // inside "docs"
        let mut event = Mouse::at(MouseKind::Up(MouseButton::Left), click_col, link.line);
        event.ctrl = true;
        assert_eq!(
            ctrl_click_url(&event, &buffer, area).as_deref(),
            Some("https://example.com/api"),
            "Ctrl+click on the label must resolve the markdown destination"
        );
    }

    #[test]
    fn custom_sheet_restyles_both_links_and_bare_urls() {
        use ratatui_core::style::Color;
        let theme = Theme::default();
        // One central rule remaps the link role: green + bold, no underline.
        let sheet = StyleSheet {
            link: StyleBundle::new().fg(Color::Green).bold(),
            ..StyleSheet::from_theme(&theme)
        };
        // A markdown link and a bare URL — both resolve the same `link` role.
        let lines = to_lines(
            "[docs](https://ex.com) and https://bare.example.com here",
            80,
            &theme,
            &sheet,
            CodeHighlighter::Plain,
        );
        let spans: Vec<&Span> = lines.iter().flat_map(|l| &l.spans).collect();
        for needle in ["docs", "bare.example.com"] {
            let span = spans
                .iter()
                .find(|s| s.content.contains(needle))
                .unwrap_or_else(|| panic!("missing {needle:?} span"));
            assert_eq!(span.style.fg, Some(Color::Green), "{needle}: recolored");
            assert!(
                span.style.add_modifier.contains(Modifier::BOLD),
                "{needle}: bold"
            );
            assert!(
                !span.style.add_modifier.contains(Modifier::UNDERLINED),
                "{needle}: underline dropped by the custom rule"
            );
        }
    }

    #[test]
    fn custom_sheet_restyles_headings() {
        use ratatui_core::style::Color;
        let theme = Theme::default();
        let sheet = StyleSheet {
            heading: StyleBundle::new().fg(Color::Magenta).italic(),
            ..StyleSheet::from_theme(&theme)
        };
        let lines = to_lines("# Title", 40, &theme, &sheet, CodeHighlighter::Plain);
        let span = &lines[0].spans[0];
        assert_eq!(span.content.as_ref(), "Title");
        assert_eq!(span.style.fg, Some(Color::Magenta));
        assert!(span.style.add_modifier.contains(Modifier::ITALIC));
        // The default heading was bold; this rule doesn't set bold, so it's gone.
        assert!(!span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn partial_emphasis_degrades_gracefully() {
        // An unterminated `**` should not panic and should still render text.
        let out = plain("this is **unfinished", 40);
        assert!(out.iter().any(|l| l.contains("unfinished")), "{out:?}");
    }
}
