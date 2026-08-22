//! The [`Html`] view: a fragment placed in a layout.

use tuika::components::text::line_width;
use tuika::geometry::Size;
use tuika::style::{StyleSheet, Theme};
use tuika::surface::Surface;
use tuika::ui::Rect;
use tuika::view::{RenderCtx, View};

use crate::{Limits, to_lines_with_limits};

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
/// `cargo run -p tuika-html --example html_view` is the scene above.
pub struct Html {
    source: String,
    limits: Limits,
}

impl Html {
    /// A view over `source`.
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            limits: Limits::default(),
        }
    }

    /// Bound this view's rendering; see [`Limits`].
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    fn lines(
        &self,
        width: u16,
        theme: &Theme,
        sheet: &StyleSheet,
    ) -> Vec<tuika::ui::Line<'static>> {
        to_lines_with_limits(&self.source, width, theme, sheet, self.limits).unwrap_or_default()
    }
}

impl View for Html {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        let lines = self.lines(available.width, ctx.theme, &ctx.sheet);
        let width = lines.iter().map(line_width).max().unwrap_or(0);
        Size::new(width.min(available.width), lines.len() as u16)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        for (row, line) in self
            .lines(area.width, ctx.theme, &ctx.sheet)
            .iter()
            .enumerate()
        {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuika::style::StyleBundle;
    use tuika::testing::{grid, render, render_with_sheet};
    use tuika::ui::Color;

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
    fn a_tiny_area_neither_panics_nor_overflows() {
        for (w, h) in [(0, 0), (1, 1), (3, 2), (80, 1)] {
            let view = Html::new("<table><tr><th>a</th><th>b</th></tr></table>");
            let _ = render(&view, w, h, &Theme::default());
        }
    }
}
