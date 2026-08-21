//! Terminal-native HTML for [tuika](https://crates.io/crates/tuika).
//!
//! Two ways in, one engine behind both:
//!
//! - [`HtmlRenderer`] plugs into markdown through tuika's
//!   [`MarkdownBlockRenderer`] boundary. One implementation handles raw `<details>`,
//!   `<table>`, and `<div>` blocks as well as ` ```html ` fences.
//! - [`Html`] is a `View`, the standalone viewer: hand it a fragment, place it
//!   in a layout, and it paints like [`Markdown`](tuika::components::Markdown)
//!   does.
//!
//! ```
//! use tuika::prelude::*;
//! use tuika_html::{Html, HtmlRenderer};
//!
//! // In markdown:
//! let renderer = HtmlRenderer::new();
//! let doc = Markdown::new("<details><summary>Notes</summary>Body</details>")
//!     .block_renderer(&renderer);
//!
//! // Standalone:
//! let view = Html::new("<h1>Title</h1><p>Prose with <b>bold</b>.</p>");
//! # let _ = (doc, view);
//! ```
//!
//! # What renders
//!
//! Headings, paragraphs, lists (ordered, unordered, nested), definition lists,
//! block quotes, `<pre>`, `<hr>`, `<table>`, `<details>`/`<summary>`, and the
//! presentational inline elements — `<b>`, `<em>`, `<code>`, `<kbd>`, `<mark>`,
//! `<a>`, `<img>` (as alt text), `<br>`, `<sub>`, `<sup>`. Unknown elements stay
//! transparent, so their text still shows. `<script>`, `<style>`, and embedded
//! objects are dropped with their content.
//!
//! Styling resolves through the active [`StyleSheet`] roles rather than colors
//! of its own, so HTML inherits the host's theme along with everything else on
//! the screen.
//!
//! # What does not
//!
//! There is no CSS, no `style` attribute, no floats or positioning, and no
//! layout that depends on the document outside the fragment. This renders
//! *content*, not pages — the goal is that HTML in a transcript reads as well as
//! the markdown around it, not that a terminal becomes a browser.
//!
//! # Untrusted input
//!
//! HTML in a transcript is untrusted. Control bytes are stripped before any text
//! becomes a cell, so markup can never emit terminal commands; input, output,
//! and nesting are bounded by [`Limits`], and exceeding either the input-size or
//! the nesting bound returns `None` (markdown then drops the block) rather than
//! doing unbounded work. Nesting is measured on the source *before* parsing,
//! because html5ever builds — and drops — its tree recursively, and deep enough
//! markup overflows the stack inside the size bound. No network is touched:
//! `<img>` becomes its alt text, never a fetch.
//!
//! [`MarkdownBlockRenderer`]: tuika::components::MarkdownBlockRenderer
//! [`StyleSheet`]: tuika::style::StyleSheet

use tuika::Theme;
use tuika::components::{MarkdownBlock, MarkdownBlockContext, MarkdownBlockRenderer};
use tuika::style::StyleSheet;
use tuika::ui::Line;

mod block;
mod dom;
mod inline;
mod table;
mod view;

pub use view::Html;

/// Bounds on one render.
///
/// Untrusted markup must degrade — dropped, or truncated — rather than consume
/// the frame, so every render is capped in three directions at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Fragments larger than this are refused outright (`render` returns
    /// `None`). Default: 256 KiB.
    pub max_input_bytes: usize,
    /// Output lines kept; the rest are dropped. Default: 4096.
    pub max_lines: usize,
    /// Element nesting accepted. A fragment nested deeper than this is refused
    /// outright (`render` returns `None`), because the *parser* recurses: a tree
    /// built from arbitrarily deep markup overflows the stack before any of this
    /// crate's code runs. Deeper subtrees inside an accepted fragment are also
    /// dropped rather than walked. Default: 64.
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 256 * 1024,
            max_lines: 4096,
            max_depth: 64,
        }
    }
}

/// Renders HTML for tuika's structured markdown block boundary.
///
/// One value serves both raw block HTML and ` ```html ` fences. Attach it with
/// [`Markdown::block_renderer`](tuika::components::Markdown::block_renderer).
///
/// ```
/// use tuika::prelude::*;
/// use tuika_html::HtmlRenderer;
///
/// let renderer = HtmlRenderer::new();
/// let doc = Markdown::new("<ul><li>one</li><li>two</li></ul>")
///     .block_renderer(&renderer);
/// # let _ = doc;
/// ```
///
/// ![HTML blocks in markdown](https://raw.githubusercontent.com/everruns/tuika/main/crates/tuika-html/examples/html_markdown/html.png)
///
/// `cargo run -p tuika-html --example html_markdown` is the scene above.
#[derive(Clone, Copy, Debug, Default)]
pub struct HtmlRenderer {
    limits: Limits,
}

impl HtmlRenderer {
    /// A renderer with the default [`Limits`].
    pub fn new() -> Self {
        Self::default()
    }

    /// A renderer with custom [`Limits`].
    pub fn with_limits(limits: Limits) -> Self {
        Self { limits }
    }

    /// The bounds this renderer applies.
    pub fn limits(&self) -> Limits {
        self.limits
    }
}

impl MarkdownBlockRenderer for HtmlRenderer {
    fn render(
        &self,
        block: MarkdownBlock<'_>,
        context: MarkdownBlockContext<'_>,
    ) -> Option<Vec<Line<'static>>> {
        let source = match block {
            MarkdownBlock::Html { source } => source,
            MarkdownBlock::Fenced { language, source }
                if matches!(
                    language.to_ascii_lowercase().as_str(),
                    "html" | "htm" | "xhtml"
                ) =>
            {
                source
            }
            _ => return None,
        };
        let lines = to_lines_with_limits(
            source,
            context.width,
            context.theme,
            context.sheet,
            self.limits,
        )?;
        // An empty result would silently swallow a fence's visible fallback.
        (!lines.is_empty()).then_some(lines)
    }
}

/// Render an HTML fragment to width-fitted styled lines.
///
/// Draw the result **without** further wrapping — it is already fitted to
/// `width`, and `<pre>` content must not be re-flowed.
///
/// ```
/// use tuika::prelude::*;
/// let theme = Theme::default();
/// let sheet = StyleSheet::from_theme(&theme);
/// let lines = tuika_html::to_lines("<p>Hello <b>world</b></p>", 40, &theme, &sheet);
/// assert_eq!(lines.len(), 1);
///
/// // Block elements are separated, and each is fitted to the width.
/// let page = tuika_html::to_lines("<h1>Title</h1><p>Prose.</p>", 40, &theme, &sheet);
/// assert_eq!(page.len(), 3); // heading, blank, prose
/// ```
pub fn to_lines(html: &str, width: u16, theme: &Theme, sheet: &StyleSheet) -> Vec<Line<'static>> {
    to_lines_with_limits(html, width, theme, sheet, Limits::default()).unwrap_or_default()
}

/// [`to_lines`] with explicit [`Limits`]: `None` when the input is over
/// `max_input_bytes`.
pub fn to_lines_with_limits(
    html: &str,
    width: u16,
    theme: &Theme,
    sheet: &StyleSheet,
    limits: Limits,
) -> Option<Vec<Line<'static>>> {
    if html.len() > limits.max_input_bytes {
        return None;
    }
    // Measured on the source, before parsing: see `dom::max_depth`.
    if dom::max_depth(html) > limits.max_depth {
        return None;
    }
    let root = dom::parse(html);
    Some(block::Layout::new(theme, sheet, limits).render(&root, width))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuika::ui::Modifier;

    /// The plain text of a render, one string per line.
    fn plain(html: &str, width: u16) -> Vec<String> {
        let theme = Theme::default();
        to_lines(html, width, &theme, &StyleSheet::from_theme(&theme))
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    fn styled(html: &str, needle: &str) -> tuika::ui::Style {
        let theme = Theme::default();
        to_lines(html, 60, &theme, &StyleSheet::from_theme(&theme))
            .iter()
            .flat_map(|l| l.spans.clone())
            .find(|s| s.content.contains(needle))
            .unwrap_or_else(|| panic!("no span containing {needle:?}"))
            .style
    }

    #[test]
    fn headings_and_paragraphs_are_separated() {
        let out = plain("<h1>Title</h1><p>Some prose.</p>", 40);
        assert_eq!(out, vec!["Title", "", "Some prose."]);
        assert!(
            styled("<h1>Title</h1>", "Title")
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn prose_wraps_to_the_width() {
        let out = plain("<p>one two three four five six seven</p>", 12);
        assert!(out.len() > 1, "{out:?}");
        assert!(out.iter().all(|l| l.chars().count() <= 12), "{out:?}");
    }

    #[test]
    fn lists_number_nest_and_hang() {
        let out = plain(
            "<ol start=3><li>first</li><li>second<ul><li>inner</li></ul></li></ol>",
            40,
        );
        assert!(out.iter().any(|l| l == "3. first"), "{out:?}");
        assert!(out.iter().any(|l| l == "4. second"), "{out:?}");
        assert!(out.iter().any(|l| l.trim() == "• inner"), "{out:?}");
        let inner = out.iter().find(|l| l.contains("inner")).unwrap();
        assert!(inner.starts_with("   "), "nested under its item: {inner:?}");
    }

    #[test]
    fn a_long_list_item_hangs_under_its_marker() {
        let out = plain("<ul><li>one two three four five six</li></ul>", 14);
        assert!(out[0].starts_with("• "), "{out:?}");
        for line in &out[1..] {
            assert!(line.starts_with("  "), "continuation hangs: {out:?}");
        }
    }

    #[test]
    fn block_quotes_indent_their_content() {
        let out = plain("<blockquote><p>quoted</p></blockquote>", 40);
        assert!(out.iter().any(|l| l == "  quoted"), "{out:?}");
    }

    #[test]
    fn pre_is_verbatim_and_never_wrapped() {
        let out = plain("<pre>  keep   spacing\nand lines</pre>", 40);
        assert!(out.iter().any(|l| l == "  keep   spacing"), "{out:?}");
        assert!(out.iter().any(|l| l == "and lines"), "{out:?}");

        let long = format!("<pre>{}</pre>", "x".repeat(60));
        let wide = plain(&long, 20);
        assert_eq!(wide.len(), 1, "code must not wrap: {wide:?}");
    }

    #[test]
    fn details_shows_its_summary_and_indents_the_body() {
        let out = plain("<details><summary>More</summary><p>Body</p></details>", 40);
        assert!(out.iter().any(|l| l == "▸ More"), "{out:?}");
        assert!(out.iter().any(|l| l == "  Body"), "{out:?}");
        // An open one says so.
        let open = plain("<details open><summary>More</summary>x</details>", 40);
        assert!(open.iter().any(|l| l == "▾ More"), "{open:?}");
    }

    #[test]
    fn tables_are_boxed_and_fitted() {
        let out = plain(
            "<table><tr><th>Name</th><th>Role</th></tr>\
             <tr><td>Ada</td><td>Author</td></tr></table>",
            40,
        );
        assert!(out[0].starts_with('╭'), "{out:?}");
        assert!(out.iter().any(|l| l.contains("Name") && l.contains("Role")));
        assert!(
            out.iter()
                .any(|l| l.contains("Ada") && l.contains("Author"))
        );
        assert!(out.last().unwrap().starts_with('╰'), "{out:?}");
        for line in &out {
            assert!(line.chars().count() <= 40, "over width: {line:?}");
        }
    }

    #[test]
    fn a_table_too_narrow_for_a_grid_keeps_its_content() {
        let out = plain(
            "<table><tr><th>Name</th><th>Role</th></tr>\
             <tr><td>Ada</td><td>Author</td></tr></table>",
            12,
        );
        let joined = out.join(" ");
        assert!(joined.contains("Ada"), "{out:?}");
        assert!(joined.contains("Author"), "{out:?}");
        for line in &out {
            assert!(line.chars().count() <= 12, "over width: {line:?}");
        }
    }

    #[test]
    fn a_headerless_table_still_renders() {
        let out = plain("<table><tr><td>a</td><td>b</td></tr></table>", 40);
        assert!(
            out.iter().any(|l| l.contains('a') && l.contains('b')),
            "{out:?}"
        );
    }

    #[test]
    fn horizontal_rules_are_themed() {
        let out = plain("<p>a</p><hr><p>b</p>", 40);
        assert!(out.iter().any(|l| l.starts_with("───")), "{out:?}");
    }

    #[test]
    fn definition_lists_render_terms_and_definitions() {
        let out = plain("<dl><dt>Term</dt><dd>Meaning</dd></dl>", 40);
        assert!(out.iter().any(|l| l == "Term"), "{out:?}");
        assert!(out.iter().any(|l| l == "  Meaning"), "{out:?}");
    }

    #[test]
    fn malformed_markup_degrades_instead_of_failing() {
        for html in [
            "<p>unclosed",
            "</p>stray close",
            "<ul><li>a<li>b",
            "<table><td>lonely cell",
            "<b><i>crossed</b></i>",
            "<div".repeat(50).as_str(),
            "&notanentity; &amp; &#x41;",
        ] {
            let out = plain(html, 30);
            for line in &out {
                assert!(line.chars().count() <= 30, "{html:?} -> {line:?}");
            }
        }
        assert!(plain("<p>unclosed", 30).iter().any(|l| l == "unclosed"));
        assert!(plain("&amp; &#x41;", 30).iter().any(|l| l == "& A"));
    }

    #[test]
    fn deep_nesting_is_bounded() {
        let deep = format!("{}deep{}", "<div>".repeat(500), "</div>".repeat(500));
        // Past `max_depth` the subtree is dropped rather than recursed into —
        // what matters is that it returns at all.
        let out = plain(&deep, 30);
        assert!(out.len() <= 2, "{out:?}");
    }

    #[test]
    fn oversized_input_is_refused_rather_than_rendered() {
        let theme = Theme::default();
        let sheet = StyleSheet::from_theme(&theme);
        let limits = Limits {
            max_input_bytes: 16,
            ..Limits::default()
        };
        assert!(
            to_lines_with_limits("<p>much too long for this</p>", 40, &theme, &sheet, limits)
                .is_none()
        );
        assert!(to_lines_with_limits("<p>ok</p>", 40, &theme, &sheet, limits).is_some());
    }

    #[test]
    fn output_is_capped() {
        let theme = Theme::default();
        let sheet = StyleSheet::from_theme(&theme);
        let limits = Limits {
            max_lines: 5,
            ..Limits::default()
        };
        let many = "<p>x</p>".repeat(100);
        let lines = to_lines_with_limits(&many, 40, &theme, &sheet, limits).expect("rendered");
        assert!(lines.len() <= 5, "{}", lines.len());
    }

    #[test]
    fn styling_follows_the_stylesheet() {
        use tuika::ui::Color;
        let theme = Theme::default();
        let sheet = StyleSheet {
            strong: tuika::style::StyleBundle::new().fg(Color::Green),
            ..StyleSheet::from_theme(&theme)
        };
        let lines = to_lines("<p><b>bold</b></p>", 40, &theme, &sheet);
        let span = lines[0]
            .spans
            .iter()
            .find(|s| s.content.contains("bold"))
            .expect("bold span");
        assert_eq!(span.style.fg, Some(Color::Green));
    }

    #[test]
    fn one_renderer_serves_both_markdown_block_kinds() {
        let theme = Theme::default();
        let sheet = StyleSheet::from_theme(&theme);
        let renderer = HtmlRenderer::new();
        let context = MarkdownBlockContext::new(20, &theme, &sheet);

        let block = renderer.render(
            MarkdownBlock::Html {
                source: "<p>hi</p>",
            },
            context,
        );
        assert!(block.is_some());
        let fence = renderer.render(
            MarkdownBlock::Fenced {
                language: "html",
                source: "<p>hi</p>",
            },
            context,
        );
        assert!(fence.is_some());
        // Other languages stay with markdown's code-block presentation.
        assert!(
            renderer
                .render(
                    MarkdownBlock::Fenced {
                        language: "rust",
                        source: "fn main() {}",
                    },
                    context,
                )
                .is_none()
        );
        // Nothing to show is `None`, so markdown's own drop path handles it.
        assert!(
            renderer
                .render(
                    MarkdownBlock::Html {
                        source: "<!-- note -->",
                    },
                    context,
                )
                .is_none()
        );
    }
}
