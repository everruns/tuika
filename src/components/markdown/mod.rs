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
//! Streaming callers apply [`MarkdownState::links`] after painting so link
//! targets stay aligned through incremental diffs and viewporting.
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
//! | `html` | the inline-HTML whitelist pass one consults for `<b>`, `<br>`, `<a>`, … |
//! | `flatten` | pass two — `MdItem`s to width-fitted `Line`s: wrapping, indentation, hyperlink runs |
//! | `table` | table layout, called from `flatten`; big enough to drown the rest of pass two |
//! | `stream` | [`MarkdownState`] and the settled-prefix cache that makes streaming O(delta) |
//! | `image` | the [`ImageResolver`] boundary and cell sizing for block images |
//! | `view` | the [`Markdown`] view over either entry point |
//!
//! The submodules are private: `components::markdown` is the one path in, and
//! `to_lines`, `MarkdownState`, and `Markdown` are what it exposes.

use crate::text::Line;

use crate::highlight::CodeHighlighter;
use crate::style::{StyleSheet, Theme};
use crate::term::hyperlink::BufferLink;

mod flatten;
mod html;
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

/// A parsed block whose presentation may be supplied outside tuika.
///
/// The descriptor is width-independent: parsing identifies the block and keeps
/// its source; layout happens later through [`MarkdownBlockRenderer`]. This is
/// what lets [`MarkdownState`] cache settled parsing across resizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MarkdownBlock<'a> {
    /// A fenced code block, including its first info-string word as `language`.
    Fenced {
        /// The first word of the fence info string, or empty.
        language: &'a str,
        /// The verbatim fence body.
        source: &'a str,
    },
    /// One raw block-level HTML run as framed by pulldown-cmark.
    Html {
        /// The raw HTML run.
        source: &'a str,
    },
}

impl<'a> MarkdownBlock<'a> {
    /// The verbatim source carried by this block.
    pub fn source(self) -> &'a str {
        match self {
            Self::Fenced { source, .. } | Self::Html { source } => source,
        }
    }
}

/// Render-time facts shared by every markdown block renderer.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct MarkdownBlockContext<'a> {
    /// Columns available after markdown nesting indentation.
    pub width: u16,
    /// The active color theme.
    pub theme: &'a Theme,
    /// The active semantic stylesheet.
    pub sheet: &'a StyleSheet,
}

impl<'a> MarkdownBlockContext<'a> {
    /// A block context using the host's complete active styling policy.
    pub const fn new(width: u16, theme: &'a Theme, sheet: &'a StyleSheet) -> Self {
        Self {
            width,
            theme,
            sheet,
        }
    }
}

/// Parses and lays out host-defined presentations for structured markdown blocks.
///
/// One contract covers every block that needs a dependency kept outside core:
/// fenced diagrams, mathematical notation, raw block HTML, query plans, and
/// future parsed block forms. Implementations inspect [`MarkdownBlock`] and
/// return `None` when they do not handle it or parsing fails. Renderer chains
/// try each implementation in order.
///
/// An unhandled fence keeps the ordinary themed
/// [`CodeBlock`](crate::components::CodeBlock) fallback. Unhandled block HTML is
/// dropped, matching markdown's behavior without an HTML parser.
/// Implementations should be deterministic, perform no I/O, bound work and
/// output for untrusted source, and never emit control bytes into cells.
pub trait MarkdownBlockRenderer {
    /// Render `block` at the current width and styling context.
    fn render(
        &self,
        block: MarkdownBlock<'_>,
        context: MarkdownBlockContext<'_>,
    ) -> Option<Vec<Line<'static>>>;
}

/// An ordered chain of host-supplied markdown block renderers.
///
/// The first renderer returning `Some` owns the block. The chain stores borrowed
/// renderer references and lets unrelated companion crates compose:
///
/// ```
/// # use tuika::components::{MarkdownBlockRenderer, Renderers};
/// # fn f(mermaid: &dyn MarkdownBlockRenderer, html: &dyn MarkdownBlockRenderer) {
/// let renderers = Renderers::new().renderer(mermaid).renderer(html);
/// # let _ = renderers;
/// # }
/// ```
#[derive(Default, Clone)]
pub struct Renderers<'a> {
    blocks: Vec<&'a dyn MarkdownBlockRenderer>,
}

impl<'a> Renderers<'a> {
    /// No structured block renderers.
    pub const fn new() -> Self {
        Self { blocks: Vec::new() }
    }

    /// Append `renderer` to the ordered chain.
    pub fn renderer(mut self, renderer: &'a dyn MarkdownBlockRenderer) -> Self {
        self.blocks.push(renderer);
        self
    }

    fn render_block(
        &self,
        block: MarkdownBlock<'_>,
        context: MarkdownBlockContext<'_>,
    ) -> Option<Vec<Line<'static>>> {
        self.blocks
            .iter()
            .find_map(|renderer| renderer.render(block, context))
    }
}

impl MarkdownBlockRenderer for Renderers<'_> {
    fn render(
        &self,
        block: MarkdownBlock<'_>,
        context: MarkdownBlockContext<'_>,
    ) -> Option<Vec<Line<'static>>> {
        self.render_block(block, context)
    }
}

impl MarkdownBlockRenderer for [&dyn MarkdownBlockRenderer] {
    fn render(
        &self,
        block: MarkdownBlock<'_>,
        context: MarkdownBlockContext<'_>,
    ) -> Option<Vec<Line<'static>>> {
        self.iter()
            .find_map(|renderer| renderer.render(block, context))
    }
}

impl MarkdownBlockRenderer for [Box<dyn MarkdownBlockRenderer>] {
    fn render(
        &self,
        block: MarkdownBlock<'_>,
        context: MarkdownBlockContext<'_>,
    ) -> Option<Vec<Line<'static>>> {
        self.iter()
            .find_map(|renderer| renderer.render(block, context))
    }
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

/// Render markdown with one host-supplied structured-block renderer.
///
/// The renderer may handle fenced or raw HTML blocks. Returning `None` preserves
/// the ordinary [`CodeBlock`](crate::components::CodeBlock) fallback for a fence
/// and drops raw block HTML.
pub fn to_lines_with_renderer(
    source: &str,
    width: u16,
    theme: &Theme,
    sheet: &StyleSheet,
    highlighter: CodeHighlighter,
    block_renderer: &dyn MarkdownBlockRenderer,
) -> Vec<Line<'static>> {
    to_lines_with(
        source,
        width,
        theme,
        sheet,
        highlighter,
        Renderers::new().renderer(block_renderer),
    )
}

/// Render markdown with an ordered chain of host-supplied [`Renderers`].
pub fn to_lines_with(
    source: &str,
    width: u16,
    theme: &Theme,
    sheet: &StyleSheet,
    highlighter: CodeHighlighter,
    renderers: Renderers<'_>,
) -> Vec<Line<'static>> {
    to_linked_lines_with(source, width, theme, sheet, highlighter, renderers).0
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
    to_linked_lines_with(
        source,
        width,
        theme,
        sheet,
        highlighter,
        Renderers::default(),
    )
}

/// Like [`to_linked_lines`], with a host-supplied [`MarkdownBlockRenderer`].
pub fn to_linked_lines_with_renderer(
    source: &str,
    width: u16,
    theme: &Theme,
    sheet: &StyleSheet,
    highlighter: CodeHighlighter,
    block_renderer: &dyn MarkdownBlockRenderer,
) -> (Vec<Line<'static>>, Vec<BufferLink>) {
    to_linked_lines_with(
        source,
        width,
        theme,
        sheet,
        highlighter,
        Renderers::new().renderer(block_renderer),
    )
}

/// Like [`to_linked_lines`], with any combination of host-supplied
/// [`Renderers`].
pub fn to_linked_lines_with(
    source: &str,
    width: u16,
    theme: &Theme,
    sheet: &StyleSheet,
    highlighter: CodeHighlighter,
    renderers: Renderers<'_>,
) -> (Vec<Line<'static>>, Vec<BufferLink>) {
    let items = parse(source, theme, sheet, highlighter);
    flatten_linked(&items, width, theme, sheet, &renderers)
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;
    use crate::highlight::CodeHighlighter;
    use crate::style::StyleBundle;
    use crate::style::{StyleSheet, Theme};
    use crate::term::hyperlink::LinkPolicy;
    use crate::text::Span;

    use crate::geometry::Rect;
    use crate::style::Modifier;

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
    fn fenced_code_background_fills_the_available_width() {
        let theme = Theme::default();
        // Wider than the shared padding chunk, so the multi-span path is also
        // covered without changing the visible contract.
        let width = 300;
        let lines = to_lines(
            "```text\nx\n```",
            width,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        );

        assert_eq!(lines.len(), 2);
        for line in &lines {
            assert_eq!(crate::components::text::line_width(line), width);
            assert!(
                line.spans
                    .iter()
                    .all(|span| span.style.bg == Some(theme.code.background)),
                "all code cells should carry the code background: {line:?}"
            );
        }
    }

    #[test]
    fn nested_fenced_code_fills_only_after_its_indent() {
        let theme = Theme::default();
        let lines = to_lines(
            "> ```text\n> x\n> ```",
            12,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        );
        let label = lines
            .iter()
            .find(|line| text(line).contains("text"))
            .expect("language label");

        assert_eq!(crate::components::text::line_width(label), 12);
        assert_eq!(label.spans[0].content.as_ref(), "  ");
        assert_eq!(label.spans[0].style.bg, None);
        assert!(
            label.spans[1..]
                .iter()
                .all(|span| span.style.bg == Some(theme.code.background))
        );
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

    impl MarkdownBlockRenderer for DiagramRenderer {
        fn render(
            &self,
            block: MarkdownBlock<'_>,
            context: MarkdownBlockContext<'_>,
        ) -> Option<Vec<Line<'static>>> {
            let MarkdownBlock::Fenced { language, source } = block else {
                return None;
            };
            (language == "diagram").then(|| {
                vec![Line::from(Span::styled(
                    format!("rendered at {}: {source}", context.width),
                    crate::style::Style::default().fg(context.theme.accent),
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
    fn fenced_block_renderer_receives_the_active_stylesheet() {
        use crate::style::Color;

        struct StyledFence;
        impl MarkdownBlockRenderer for StyledFence {
            fn render(
                &self,
                block: MarkdownBlock<'_>,
                context: MarkdownBlockContext<'_>,
            ) -> Option<Vec<Line<'static>>> {
                matches!(
                    block,
                    MarkdownBlock::Fenced {
                        language: "styled",
                        ..
                    }
                )
                .then(|| {
                    vec![Line::from(Span::styled(
                        "styled",
                        context.sheet.heading.to_style(),
                    ))]
                })
            }
        }

        let theme = Theme::default();
        let sheet = StyleSheet {
            heading: StyleBundle::new().fg(Color::Green),
            ..StyleSheet::from_theme(&theme)
        };
        let lines = to_lines_with_renderer(
            "```styled\nsource\n```",
            20,
            &theme,
            &sheet,
            CodeHighlighter::Plain,
            &StyledFence,
        );

        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Green));
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
    fn bare_url_does_not_underline_the_following_space() {
        let theme = Theme::default();
        let source = "Fetched **https://everruns.com/** and saved a durable summary to:";
        let (lines, links) = to_linked_lines(
            source,
            80,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        );
        let url = "https://everruns.com/";
        let url_span = lines[0]
            .spans
            .iter()
            .find(|span| span.content.starts_with(url))
            .expect("url span");

        assert_eq!(url_span.content.as_ref(), url);
        let link = links.iter().find(|link| link.url == url).expect("url link");
        assert_eq!(link.end_col - link.start_col, url.len() as u16);

        let buffer = crate::testing::render(&Markdown::new(source), 80, 1, &theme);
        let following_space = link.end_col;
        assert!(
            !buffer[(following_space, 0)]
                .modifier
                .contains(Modifier::UNDERLINED)
        );
    }

    #[test]
    fn labeled_markdown_link_preserves_destination_for_ctrl_click() {
        // Reproduction of the Ghostty Ctrl+click failure: `[label](url)` was
        // only styled — the destination was dropped — so OSC 8 / ctrl_click had
        // nothing to open under the pointer.
        use crate::buffer::Buffer;
        use crate::geometry::Position;
        use crate::term::hyperlink::{apply_buffer_links, ctrl_click_url};
        use crate::{Mouse, MouseButton, MouseKind};

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
        use crate::style::Color;
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
        use crate::style::Color;
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

    /// The span carrying `needle`, for style assertions.
    fn span_with<'a>(lines: &'a [Line<'static>], needle: &str) -> &'a Span<'static> {
        lines
            .iter()
            .flat_map(|l| &l.spans)
            .find(|s| s.content.contains(needle))
            .unwrap_or_else(|| panic!("missing {needle:?} span"))
    }

    fn render(source: &str, width: u16) -> Vec<Line<'static>> {
        let theme = Theme::default();
        to_lines(
            source,
            width,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        )
    }

    #[test]
    fn inline_html_resolves_the_same_roles_as_markdown_markup() {
        let theme = Theme::default();
        let sheet = StyleSheet::from_theme(&theme);
        let lines = render(
            "<b>bee</b> <i>eye</i> <code>see</code> <u>you</u> <mark>em</mark>",
            60,
        );
        assert!(
            span_with(&lines, "bee")
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            span_with(&lines, "eye")
                .style
                .add_modifier
                .contains(Modifier::ITALIC)
        );
        assert_eq!(
            span_with(&lines, "see").style.bg,
            sheet.inline_code.to_style().bg
        );
        assert!(
            span_with(&lines, "you")
                .style
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
        assert!(
            span_with(&lines, "em")
                .style
                .add_modifier
                .contains(Modifier::REVERSED)
        );
    }

    #[test]
    fn inline_html_follows_a_restyled_role() {
        // The point of routing tags through the stylesheet: a host that restyles
        // `strong` restyles `<b>` with it, without knowing HTML exists.
        use crate::style::Color;
        let theme = Theme::default();
        let sheet = StyleSheet {
            strong: StyleBundle::new().fg(Color::Green),
            ..StyleSheet::from_theme(&theme)
        };
        let lines = to_lines("<b>tagged</b>", 40, &theme, &sheet, CodeHighlighter::Plain);
        assert_eq!(span_with(&lines, "tagged").style.fg, Some(Color::Green));
    }

    #[test]
    fn br_breaks_the_line_and_collapses_inside_a_cell() {
        let out: Vec<String> = render("one<br>two", 40).iter().map(text).collect();
        assert!(out.iter().any(|l| l.trim() == "one"), "{out:?}");
        assert!(out.iter().any(|l| l.trim() == "two"), "{out:?}");

        // A cell cannot become two blocks, so the break collapses to a space —
        // the same treatment a markdown hard break gets there.
        let cell: Vec<String> = render("| h |\n| --- |\n| a<br>b |", 40)
            .iter()
            .map(text)
            .collect();
        assert!(cell.iter().any(|l| l.contains("a b")), "{cell:?}");
    }

    #[test]
    fn anchor_keeps_its_label_and_destination() {
        let theme = Theme::default();
        let (lines, links) = to_linked_lines(
            r#"see <a href="https://example.com/x">the docs</a> now"#,
            60,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
        );
        let plain: String = lines.iter().map(text).collect();
        assert!(plain.contains("the docs"), "{plain:?}");
        assert!(!plain.contains("example.com"), "label, not URL: {plain:?}");
        assert!(
            links.iter().any(|l| l.url == "https://example.com/x"),
            "anchor must yield a BufferLink: {links:?}"
        );
    }

    #[test]
    fn img_tag_takes_the_markdown_image_path() {
        let out: Vec<String> = render(r#"before <img src="p.png" alt="a cat"> after"#, 60)
            .iter()
            .map(text)
            .collect();
        assert!(
            out.iter().any(|l| l.contains("🖼 a cat")),
            "unresolved `<img>` shows the alt placeholder: {out:?}"
        );
    }

    #[test]
    fn sub_and_sup_become_unicode_when_every_character_maps() {
        let out: Vec<String> = render("H<sub>2</sub>O and x<sup>-9</sup>", 40)
            .iter()
            .map(text)
            .collect();
        assert!(out.iter().any(|l| l.contains("H₂O")), "{out:?}");
        assert!(out.iter().any(|l| l.contains("x⁻⁹")), "{out:?}");

        // No form for letters, so the text renders unchanged rather than partly
        // transliterated.
        let mixed: Vec<String> = render("4<sup>th</sup>", 40).iter().map(text).collect();
        assert!(mixed.iter().any(|l| l.contains("4th")), "{mixed:?}");
    }

    /// Renders any HTML block as one line quoting its source and the width.
    struct EchoHtml;

    impl MarkdownBlockRenderer for EchoHtml {
        fn render(
            &self,
            block: MarkdownBlock<'_>,
            context: MarkdownBlockContext<'_>,
        ) -> Option<Vec<Line<'static>>> {
            let MarkdownBlock::Html { source } = block else {
                return None;
            };
            let text = source.split_whitespace().collect::<Vec<_>>().join(" ");
            (!text.is_empty()).then(|| {
                vec![Line::from(Span::styled(
                    format!("[{}] {text}", context.width),
                    crate::style::Style::default().fg(context.theme.accent),
                ))]
            })
        }
    }

    fn with_html(source: &str, width: u16) -> Vec<String> {
        let theme = Theme::default();
        to_lines_with(
            source,
            width,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
            Renderers::new().renderer(&EchoHtml),
        )
        .iter()
        .map(text)
        .collect()
    }

    #[test]
    fn structured_block_renderer_lays_out_raw_html() {
        let out = with_html(
            "before\n\n<details><summary>S</summary>body</details>\n\nafter",
            40,
        );
        assert!(
            out.iter()
                .any(|l| l == "[40] <details><summary>S</summary>body</details>"),
            "{out:?}"
        );
        // The surrounding markdown is untouched, blank spacers included.
        assert!(out.iter().any(|l| l == "before"), "{out:?}");
        assert!(out.iter().any(|l| l == "after"), "{out:?}");
    }

    #[test]
    fn html_block_is_indented_by_its_container() {
        // A block inside a quote gets the quote's indentation, like a fence
        // does, and the renderer is asked for the *remaining* width.
        let out = with_html("> quoted\n>\n> <div>x</div>", 40);
        let block = out
            .iter()
            .find(|l| l.contains("<div>"))
            .unwrap_or_else(|| panic!("{out:?}"));
        assert!(block.starts_with("  ["), "indented: {block:?}");
        assert!(block.contains("[38]"), "width less the indent: {block:?}");
    }

    #[test]
    fn html_block_without_a_renderer_is_dropped() {
        let out: Vec<String> = render("a\n\n<div>x</div>\n\nb", 40)
            .iter()
            .map(text)
            .collect();
        assert!(!out.join("\n").contains('x'), "{out:?}");
    }

    #[test]
    fn block_renderer_returning_none_drops_html() {
        struct Decline;
        impl MarkdownBlockRenderer for Decline {
            fn render(
                &self,
                _: MarkdownBlock<'_>,
                _: MarkdownBlockContext<'_>,
            ) -> Option<Vec<Line<'static>>> {
                None
            }
        }
        let theme = Theme::default();
        let out: Vec<String> = to_lines_with(
            "a\n\n<div>x</div>\n\nb",
            40,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
            Renderers::new().renderer(&Decline),
        )
        .iter()
        .map(text)
        .collect();
        assert!(!out.join("\n").contains('x'), "{out:?}");
        assert!(out.iter().any(|l| l == "a"), "{out:?}");
    }

    #[test]
    fn independent_block_renderers_compose() {
        let theme = Theme::default();
        let out: Vec<String> = to_lines_with(
            "```diagram\nA --> B\n```\n\n<div>html</div>",
            37,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
            Renderers::new()
                .renderer(&DiagramRenderer)
                .renderer(&EchoHtml),
        )
        .iter()
        .map(text)
        .collect();
        assert!(out.iter().any(|l| l.contains("rendered at 37")), "{out:?}");
        assert!(out.iter().any(|l| l.contains("[37] <div>html")), "{out:?}");
    }

    #[test]
    fn renderer_chain_uses_the_first_handler() {
        struct Named(&'static str);
        impl MarkdownBlockRenderer for Named {
            fn render(
                &self,
                block: MarkdownBlock<'_>,
                _: MarkdownBlockContext<'_>,
            ) -> Option<Vec<Line<'static>>> {
                matches!(block, MarkdownBlock::Html { .. }).then(|| vec![Line::raw(self.0)])
            }
        }

        let theme = Theme::default();
        let first = Named("first");
        let second = Named("second");
        let out = to_lines_with(
            "<div>html</div>",
            20,
            &theme,
            &StyleSheet::from_theme(&theme),
            CodeHighlighter::Plain,
            Renderers::new().renderer(&first).renderer(&second),
        );

        assert_eq!(text(&out[0]), "first");
    }

    #[test]
    fn unrecognized_html_is_dropped_as_before() {
        // Block HTML and non-whitelisted tags keep the old behavior: the markup
        // never reaches the screen as literal text.
        let out: Vec<String> = render("<div>\n<p>block</p>\n</div>\n\nafter <span>x</span>", 40)
            .iter()
            .map(text)
            .collect();
        let joined = out.join("\n");
        assert!(!joined.contains('<'), "no raw markup rendered: {out:?}");
        assert!(joined.contains("after x"), "{out:?}");
    }

    #[test]
    fn unbalanced_html_cannot_leak_past_its_block() {
        // An open tag with no close, a close with no open, and crossed nesting:
        // none may panic, and none may style the blocks that follow.
        let lines = render("<b>open\n\n</i>stray\n\n<b><i>crossed</b>tail", 40);
        for needle in ["stray", "tail"] {
            assert!(
                !span_with(&lines, needle)
                    .style
                    .add_modifier
                    .contains(Modifier::BOLD),
                "{needle} must not inherit an unclosed scope"
            );
        }
    }

    #[test]
    fn deeply_nested_html_is_bounded_and_renders_its_text() {
        let source = format!("{}deep{}", "<b>".repeat(500), "</b>".repeat(500));
        let out: Vec<String> = render(&source, 40).iter().map(text).collect();
        assert!(out.iter().any(|l| l.contains("deep")), "{out:?}");
    }

    #[test]
    fn html_inside_a_code_fence_stays_literal() {
        let out: Vec<String> = render("```html\n<b>x</b>\n```", 40)
            .iter()
            .map(text)
            .collect();
        assert!(
            out.iter().any(|l| l.contains("<b>x</b>")),
            "code is verbatim: {out:?}"
        );
    }

    #[test]
    fn partial_emphasis_degrades_gracefully() {
        // An unterminated `**` should not panic and should still render text.
        let out = plain("this is **unfinished", 40);
        assert!(out.iter().any(|l| l.contains("unfinished")), "{out:?}");
    }
}
