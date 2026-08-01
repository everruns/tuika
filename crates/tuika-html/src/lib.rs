//! Terminal-native HTML for [tuika](https://crates.io/crates/tuika).
//!
//! Two ways in, one engine behind both:
//!
//! - [`HtmlRenderer`] plugs into markdown. It implements tuika's
//!   [`HtmlBlockRenderer`] seam, so raw `<details>`, `<table>`, and `<div>`
//!   blocks inside a markdown document lay out instead of being dropped — and
//!   [`FencedBlockRenderer`], so a ` ```html ` fence renders rather than showing
//!   its source.
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
//!     .html_renderer(&renderer);
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
//! [`FencedBlockRenderer`]: tuika::components::FencedBlockRenderer
//! [`HtmlBlockRenderer`]: tuika::components::HtmlBlockRenderer
//! [`StyleSheet`]: tuika::style::StyleSheet

use ratatui_core::text::Line;
use tuika::Theme;
use tuika::components::{FencedBlockRenderer, HtmlBlockRenderer, RenderedBlock};
use tuika::style::StyleSheet;

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

/// Renders HTML for tuika's markdown seams.
///
/// One value serves both: attach it with
/// [`Markdown::html_renderer`](tuika::components::Markdown::html_renderer) for raw
/// block HTML, and with
/// [`Markdown::block_renderer`](tuika::components::Markdown::block_renderer) for
/// ` ```html ` fences.
///
/// ```
/// use tuika::prelude::*;
/// use tuika_html::HtmlRenderer;
///
/// let renderer = HtmlRenderer::new();
/// let doc = Markdown::new("<ul><li>one</li><li>two</li></ul>")
///     .html_renderer(&renderer)
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

impl HtmlBlockRenderer for HtmlRenderer {
    fn render(
        &self,
        source: &str,
        width: u16,
        theme: &Theme,
        sheet: &StyleSheet,
    ) -> Option<RenderedBlock> {
        let block = to_block_with_limits(source, width, theme, sheet, self.limits)?;
        // An empty result would silently swallow the block; let markdown's own
        // "no renderer" path own that case instead.
        (!block.lines.is_empty()).then_some(block)
    }
}

impl FencedBlockRenderer for HtmlRenderer {
    fn render(
        &self,
        language: &str,
        source: &str,
        width: u16,
        theme: &Theme,
    ) -> Option<Vec<Line<'static>>> {
        if !matches!(
            language.to_ascii_lowercase().as_str(),
            "html" | "htm" | "xhtml"
        ) {
            return None;
        }
        // A fence carries no stylesheet through the seam, so the theme's own
        // default mapping stands in — the same styles a host that never
        // customized its sheet would see.
        let sheet = StyleSheet::from_theme(theme);
        // A fence reaches the terminal as lines only — the fenced seam has
        // nowhere to carry link runs — so anchors inside one are styled but not
        // clickable. Block HTML, which does carry them, is the richer path.
        let block = to_block_with_limits(source, width, theme, &sheet, self.limits)?;
        (!block.lines.is_empty()).then_some(block.lines)
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
    to_block(html, width, theme, sheet).lines
}

/// Render an HTML fragment to lines **and** the hyperlink runs inside them.
///
/// The full result: [`to_lines`] is this with the links dropped. Feed the links
/// to [`apply_buffer_links`](tuika::term::hyperlink::apply_buffer_links) after
/// painting, and an `<a href>` becomes a real OSC 8 hyperlink.
///
/// ```
/// use tuika::prelude::*;
/// let theme = Theme::default();
/// let sheet = StyleSheet::from_theme(&theme);
/// let block = tuika_html::to_block(
///     r#"<p>see <a href="https://docs.rs/tuika">the docs</a></p>"#,
///     40,
///     &theme,
///     &sheet,
/// );
/// assert_eq!(block.links[0].url, "https://docs.rs/tuika");
/// ```
pub fn to_block(html: &str, width: u16, theme: &Theme, sheet: &StyleSheet) -> RenderedBlock {
    to_block_with_limits(html, width, theme, sheet, Limits::default()).unwrap_or_default()
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
    to_block_with_limits(html, width, theme, sheet, limits).map(|block| block.lines)
}

/// [`to_block`] with explicit [`Limits`]: `None` when the fragment is over
/// `max_input_bytes` or nested deeper than `max_depth`.
pub fn to_block_with_limits(
    html: &str,
    width: u16,
    theme: &Theme,
    sheet: &StyleSheet,
    limits: Limits,
) -> Option<RenderedBlock> {
    if html.len() > limits.max_input_bytes {
        return None;
    }
    // Measured on the source, before parsing: see `dom::max_depth`.
    if dom::max_depth(html) > limits.max_depth {
        return None;
    }
    let root = dom::parse(html);
    let (lines, links) = block::Layout::new(theme, sheet, limits).render(&root, width);
    Some(RenderedBlock::new(lines, links))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_core::style::Modifier;

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

    fn styled(html: &str, needle: &str) -> ratatui_core::style::Style {
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

    /// The hyperlink runs of a render, as `(line, start_col, end_col, url)`.
    fn links(html: &str, width: u16) -> Vec<(u16, u16, u16, String)> {
        let theme = Theme::default();
        to_block(html, width, &theme, &StyleSheet::from_theme(&theme))
            .links
            .into_iter()
            .map(|l| (l.line, l.start_col, l.end_col, l.url))
            .collect()
    }

    /// The columns a link covers must be the columns its label was painted at,
    /// whatever chrome the block pass put in front of it — otherwise OSC 8 lands
    /// on the wrong cells and the wrong text becomes clickable.
    #[test]
    fn anchors_report_the_columns_their_label_occupies() {
        let out = plain(r#"<p>see <a href="https://e.co">docs</a> now</p>"#, 40);
        assert_eq!(out, vec!["see docs now"]);
        assert_eq!(
            links(r#"<p>see <a href="https://e.co">docs</a> now</p>"#, 40),
            vec![(0, 4, 8, "https://e.co".to_string())]
        );
    }

    #[test]
    fn a_list_items_anchor_clears_the_marker_and_the_indent() {
        let html = r#"<ul><li>x</li><li><a href="https://e.co">go</a></li></ul>"#;
        let out = plain(html, 40);
        assert_eq!(out[1], "• go");
        // Past the `• ` hang, on the second item's row.
        assert_eq!(links(html, 40), vec![(1, 2, 4, "https://e.co".to_string())]);
    }

    #[test]
    fn a_table_cells_anchor_clears_the_box_chrome() {
        let html = r#"<table><tr><td>a</td><td><a href="https://e.co">go</a></td></tr></table>"#;
        let out = plain(html, 40);
        let (row, _, start, end, _) = {
            let (i, line) = out
                .iter()
                .enumerate()
                .find(|(_, l)| l.contains("go"))
                .expect("cell row");
            let start = line.chars().position(|c| c == 'g').unwrap() as u16;
            (i as u16, line, start, start + 2, ())
        };
        assert_eq!(
            links(html, 40),
            vec![(row, start, end, "https://e.co".into())]
        );
    }

    #[test]
    fn a_details_summary_can_be_clickable() {
        let html = r#"<details><summary><a href="https://e.co">More</a></summary>x</details>"#;
        let out = plain(html, 40);
        assert_eq!(out[0], "▸ More");
        // Past the disclosure marker.
        assert_eq!(links(html, 40), vec![(0, 2, 6, "https://e.co".to_string())]);
    }

    #[test]
    fn a_link_wrapped_across_rows_becomes_one_run_per_row() {
        let html = r#"<p><a href="https://e.co">alpha beta gamma</a></p>"#;
        let out = plain(html, 12);
        assert!(out.len() > 1, "{out:?}");
        let got = links(html, 12);
        assert_eq!(got.len(), out.len(), "one run per row: {got:?}");
        for ((row, start, end, url), line) in got.iter().zip(&out) {
            assert_eq!(url, "https://e.co");
            assert_eq!(*start, 0);
            assert_eq!(*end as usize, line.chars().count());
            assert!((*row as usize) < out.len());
        }
    }

    #[test]
    fn an_unresolved_image_links_to_its_own_source() {
        let html = "<p><img src='https://e.co/cat.png' alt='a cat'></p>";
        assert_eq!(
            links(html, 40)
                .iter()
                .map(|(_, _, _, u)| u.clone())
                .collect::<Vec<_>>(),
            vec!["https://e.co/cat.png".to_string()]
        );
        // Inside an anchor, the anchor wins — one destination, not two.
        let wrapped =
            "<p><a href='https://e.co/page'><img src='https://e.co/cat.png' alt='a cat'></a></p>";
        assert!(
            links(wrapped, 40)
                .iter()
                .all(|(_, _, _, u)| u == "https://e.co/page"),
            "{:?}",
            links(wrapped, 40)
        );
    }

    #[test]
    fn styling_follows_the_stylesheet() {
        use ratatui_core::style::Color;
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
    fn the_renderer_serves_both_markdown_seams() {
        let theme = Theme::default();
        let sheet = StyleSheet::from_theme(&theme);
        let renderer = HtmlRenderer::new();

        let block = HtmlBlockRenderer::render(&renderer, "<p>hi</p>", 20, &theme, &sheet);
        assert!(block.is_some());
        let fence = FencedBlockRenderer::render(&renderer, "html", "<p>hi</p>", 20, &theme);
        assert!(fence.is_some());
        // Other languages stay with markdown's code-block presentation.
        assert!(
            FencedBlockRenderer::render(&renderer, "rust", "fn main() {}", 20, &theme).is_none()
        );
        // Nothing to show is `None`, so markdown's own drop path handles it.
        assert!(
            HtmlBlockRenderer::render(&renderer, "<!-- note -->", 20, &theme, &sheet).is_none()
        );
    }
}
