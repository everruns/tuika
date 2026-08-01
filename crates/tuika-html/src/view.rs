//! The [`Html`] view: a fragment placed in a layout.

use ratatui_core::layout::Rect;
use tuika::components::RenderedBlock;
use tuika::components::text::line_width;
use tuika::geometry::Size;
use tuika::style::{StyleSheet, Theme};
use tuika::surface::Surface;
use tuika::term::hyperlink::{LinkPolicy, apply_buffer_links};
use tuika::view::{RenderCtx, View};

use crate::{Limits, to_block_with_limits};

/// A view that renders an HTML fragment to its area.
///
/// ![Html view demo](https://raw.githubusercontent.com/everruns/tuika/main/crates/tuika-html/examples/html_view/html_view.png)
///
/// The standalone counterpart to attaching an
/// [`HtmlRenderer`](crate::HtmlRenderer) to markdown: same engine, but the whole
/// area is HTML. Prose wraps to the width, `<pre>` stays verbatim, and tables
/// are fitted — recomputed each frame, so one fragment covers every terminal
/// size.
///
/// ```no_run
/// use tuika::prelude::*;
/// use tuika_html::Html;
/// let page = Html::new("<h1>Release notes</h1><ul><li>Faster</li></ul>");
/// // `page` is a `View`: render it via `tuika::paint` or embed it in a `Flex`.
/// # let _ = page;
/// ```
///
/// An `<a href>` is styled *and* clickable: the view emits OSC 8 for the
/// hyperlink runs the engine reports, under a [`LinkPolicy`] the host can
/// narrow — see [`link_policy`](Self::link_policy).
///
/// `cargo run -p tuika-html --example html_view` is the scene above.
pub struct Html {
    source: String,
    limits: Limits,
    link_policy: LinkPolicy,
}

impl Html {
    /// A view over `source`.
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            limits: Limits::default(),
            link_policy: LinkPolicy::default(),
        }
    }

    /// Bound this view's rendering; see [`Limits`].
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Configure which URL schemes an `<a href>` is emitted under as an OSC 8
    /// hyperlink.
    ///
    /// The default ([`LinkPolicy::WEB`]) links `http(s)` only;
    /// [`LinkPolicy::NONE`] paints styled labels but emits no OSC 8. The same
    /// knob [`Markdown::link_policy`](tuika::components::Markdown::link_policy)
    /// offers, so a host can hold both to one policy.
    ///
    /// ```
    /// use tuika::term::hyperlink::LinkPolicy;
    /// use tuika_html::Html;
    /// let inert = Html::new(r#"<a href="https://example.com">docs</a>"#)
    ///     .link_policy(LinkPolicy::NONE);
    /// # let _ = inert;
    /// ```
    pub fn link_policy(mut self, policy: LinkPolicy) -> Self {
        self.link_policy = policy;
        self
    }

    fn block(&self, width: u16, theme: &Theme, sheet: &StyleSheet) -> RenderedBlock {
        to_block_with_limits(&self.source, width, theme, sheet, self.limits).unwrap_or_default()
    }
}

impl View for Html {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        let lines = self.block(available.width, ctx.theme, &ctx.sheet).lines;
        let width = lines.iter().map(line_width).max().unwrap_or(0);
        Size::new(width.min(available.width), lines.len() as u16)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        let block = self.block(area.width, ctx.theme, &ctx.sheet);
        for (row, line) in block.lines.iter().enumerate() {
            let y = area.y.saturating_add(row as u16);
            if y >= area.bottom() {
                break;
            }
            let mut x = area.x;
            for span in &line.spans {
                if x >= area.right() {
                    break;
                }
                x = surface.set_string(x, y, span.content.as_ref(), span.style);
            }
        }
        // Anchor labels are painted as ordinary text, so the destination only
        // reaches the terminal as OSC 8 — the same pass `Markdown` runs.
        apply_buffer_links(
            surface.buffer_mut(),
            ratatui_core::layout::Position {
                x: area.x,
                y: area.y,
            },
            &block.links,
            self.link_policy,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_core::style::Color;
    use tuika::style::StyleBundle;
    use tuika::testing::{grid, render, render_with_sheet};

    #[test]
    fn the_view_paints_its_fragment() {
        let view = Html::new("<h1>Hi</h1><p>there</p>");
        let painted = grid(&render(&view, 10, 3, &Theme::default()));
        assert_eq!(painted, "Hi        \n          \nthere     ");
    }

    #[test]
    fn the_view_measures_its_rendered_size() {
        let view = Html::new("<h1>Title</h1><p>body</p>");
        let theme = Theme {
            accent: Color::Cyan,
            ..Theme::default()
        };
        let sheet = StyleSheet {
            heading: StyleBundle::new().fg(Color::Magenta),
            ..StyleSheet::from_theme(&theme)
        };
        let ctx = RenderCtx::new(&theme).with_sheet(sheet);
        let size = view.measure(Size::new(12, 10), &ctx);

        assert_eq!(size, Size::new(5, 3));
        let painted = render_with_sheet(&view, 12, size.height, &theme, sheet);
        assert_eq!(grid(&painted), "Title       \n            \nbody        ");
        assert_eq!(painted[(0, 0)].fg, Color::Magenta);
    }

    #[test]
    fn an_anchor_becomes_a_real_hyperlink() {
        let view = Html::new(r#"<p>see <a href="https://docs.rs/tuika">docs</a></p>"#);
        let painted = render(&view, 20, 1, &Theme::default());
        let row: String = (0..20).map(|x| painted[(x, 0)].symbol()).collect();
        assert!(row.contains("\x1b]8;;https://docs.rs/tuika"), "{row:?}");
        // The visible label is unchanged: the opener and closer ride along with
        // the run's boundary cells and cost no columns.
        assert!(row.contains("\\docs\x1b]8;;"), "{row:?}");

        // A host can keep them inert.
        let inert = Html::new(r#"<p>see <a href="https://docs.rs/tuika">docs</a></p>"#)
            .link_policy(LinkPolicy::NONE);
        let painted = render(&inert, 20, 1, &Theme::default());
        let row: String = (0..20).map(|x| painted[(x, 0)].symbol()).collect();
        assert!(!row.contains("\x1b]8;;"), "{row:?}");
    }

    #[test]
    fn a_tiny_area_neither_panics_nor_overflows() {
        for (w, h) in [(0, 0), (1, 1), (3, 2), (80, 1)] {
            let view = Html::new("<table><tr><th>a</th><th>b</th></tr></table>");
            let _ = render(&view, w, h, &Theme::default());
        }
    }
}
