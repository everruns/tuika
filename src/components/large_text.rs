//! [`LargeText`] — a banner or title drawn several cells tall.
//!
//! On a terminal that implements Kitty's text sizing protocol (OSC 66) the text
//! is real type at two to seven times the cell height, so it stays selectable,
//! scrolls with the screen, and keeps the terminal's own font. Everywhere else
//! it falls back to [`AsciiFont`]'s block letters when the bundled font can
//! spell the text, and to bold text at normal size when it cannot. See
//! [`term::text_size`](crate::term::text_size) for the protocol.

use crate::geometry::{Rect, Size};
use crate::style::{Modifier, Style};
use crate::surface::Surface;
use crate::term::text_size::{MAX_SCALE, SizedRun, apply_sized_runs_in};
use crate::view::{RenderCtx, View};
use crate::width::str_cols;

use super::ascii_font::{self, AsciiFont};

/// A line of text drawn `scale` times larger where the terminal can.
///
/// It takes the largest scale up to [`scale`](Self::scale) that fits the space
/// it is given, so a narrow pane shrinks the title rather than cutting it off.
/// The sized path needs the host to declare OSC 66 support on the render context
/// ([`Runner`](crate::Runner) detects it; custom hosts call
/// [`RenderCtx::with_text_sizing`]).
///
/// | Builder | Default | Effect |
/// | --- | --- | --- |
/// | [`new(text)`](Self::new) | — | the single line to draw |
/// | [`scale(n)`](Self::scale) | `2` | the largest scale to use, `2..=7` |
/// | [`style(s)`](Self::style) | accent, bold | colors and attributes |
/// | [`ascii_fallback(b)`](Self::ascii_fallback) | `true` | use [`AsciiFont`] when sizing is unavailable |
///
/// ```no_run
/// use tuika::prelude::*;
/// let title = LargeText::new("Release notes").scale(3);
/// # let _ = title;
/// ```
pub struct LargeText {
    text: String,
    scale: u8,
    style: Option<Style>,
    ascii_fallback: bool,
}

/// How a [`LargeText`] will paint into a given space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plan {
    /// OSC 66 at this scale.
    Sized(u8),
    /// The bundled block-letter font.
    Ascii,
    /// Normal-size text.
    Plain,
}

impl LargeText {
    /// A large line of `text`. Line breaks and control characters are dropped:
    /// the view draws exactly one line.
    pub fn new(text: impl Into<String>) -> Self {
        let text: String = text.into();
        let text = text
            .chars()
            .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
            .filter(|c| !c.is_control())
            .collect();
        Self {
            text,
            scale: 2,
            style: None,
            ascii_fallback: true,
        }
    }

    /// The largest scale to draw at, clamped to `2..=7`.
    pub fn scale(mut self, scale: u8) -> Self {
        self.scale = scale.clamp(2, MAX_SCALE);
        self
    }

    /// Override the style (defaults to the theme accent, bold).
    pub fn style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }

    /// Whether to fall back to [`AsciiFont`] block letters when the terminal
    /// cannot size text. With `false`, or when the font cannot spell the text,
    /// the fallback is the text at normal size.
    pub fn ascii_fallback(mut self, enabled: bool) -> Self {
        self.ascii_fallback = enabled;
        self
    }

    fn plan(&self, space: Size, ctx: &RenderCtx) -> Plan {
        let cols = str_cols(&self.text);
        if cols == 0 {
            return Plan::Plain;
        }
        if ctx.text_sizing().is_supported() {
            let fits = (2..=self.scale).rev().find(|&s| {
                u32::from(cols) * u32::from(s) <= u32::from(space.width)
                    && u16::from(s) <= space.height
            });
            if let Some(scale) = fits {
                return Plan::Sized(scale);
            }
        }
        if self.ascii_fallback && ascii_font::covers(&self.text) {
            let font = AsciiFont::new(self.text.clone());
            let wanted = font.measure(Size::new(u16::MAX, u16::MAX), ctx);
            if wanted.width <= space.width && wanted.height <= space.height {
                return Plan::Ascii;
            }
        }
        Plan::Plain
    }

    fn resolved_style(&self, ctx: &RenderCtx) -> Style {
        self.style.unwrap_or_else(|| {
            Style::default()
                .fg(ctx.theme.accent)
                .add_modifier(Modifier::BOLD)
        })
    }
}

impl View for LargeText {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        let cols = str_cols(&self.text);
        let size = match self.plan(available, ctx) {
            Plan::Sized(scale) => Size::new(cols * u16::from(scale), u16::from(scale)),
            Plan::Ascii => AsciiFont::new(self.text.clone()).measure(available, ctx),
            Plan::Plain => Size::new(cols, u16::from(cols > 0)),
        };
        size.clamp_to(available)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.is_empty() {
            return;
        }
        let style = self.resolved_style(ctx);
        match self.plan(area.as_size(), ctx) {
            Plan::Ascii => {
                let mut font = AsciiFont::new(self.text.clone());
                if let Some(fg) = style.fg {
                    font = font.color(fg);
                }
                font.render(area, surface, ctx);
            }
            plan => {
                let end = surface.set_string(area.x, area.y, &self.text, style);
                // A block cut by the surface's clip (a scrolled viewport) would
                // be erased by the cells around it; leave it at normal size.
                if let Plan::Sized(scale) = plan
                    && surface.area().intersection(area) == area
                {
                    let run = SizedRun {
                        line: 0,
                        start_col: 0,
                        end_col: end.saturating_sub(area.x),
                        scale,
                    };
                    apply_sized_runs_in(surface.buffer_mut(), area, &[run]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::CellDiffOption;
    use crate::style::Theme;
    use crate::term::text_size::{TextSizing, visible_text};
    use crate::testing::{grid, render, render_with_context};

    fn sized(theme: &Theme) -> RenderCtx<'_> {
        RenderCtx::new(theme).with_text_sizing(TextSizing::Supported)
    }

    #[test]
    fn sized_text_takes_scale_rows_and_scaled_columns() {
        let theme = Theme::default();
        let ctx = sized(&theme);
        let view = LargeText::new("Hi").scale(3);
        assert_eq!(view.measure(Size::new(80, 24), &ctx), Size::new(6, 3));
        let buf = render_with_context(&view, 8, 3, &ctx);
        assert_eq!(visible_text(buf[(0, 0)].symbol()), "H");
        assert!(buf[(0, 0)].symbol().starts_with("\x1b]66;s=3:w=1;"));
        assert_eq!(visible_text(buf[(3, 0)].symbol()), "i");
        assert_eq!(buf[(1, 2)].diff_option, CellDiffOption::Skip);
        assert_eq!(buf[(0, 0)].fg, theme.accent);
    }

    #[test]
    fn a_narrow_space_lowers_the_scale() {
        let theme = Theme::default();
        let ctx = sized(&theme);
        let view = LargeText::new("Title").scale(3);
        // 15 columns at 3×, 10 at 2×: a 12-wide space takes 2×.
        assert_eq!(view.measure(Size::new(12, 24), &ctx), Size::new(10, 2));
        let buf = render_with_context(&view, 12, 2, &ctx);
        assert!(buf[(0, 0)].symbol().starts_with("\x1b]66;s=2:w=1;"));
    }

    #[test]
    fn without_sizing_it_falls_back_to_block_letters() {
        let theme = Theme::default();
        let view = LargeText::new("HI");
        let ctx = RenderCtx::new(&theme);
        assert_eq!(view.measure(Size::new(80, 24), &ctx), Size::new(11, 5));
        let buf = render(&view, 11, 5, &theme);
        assert_eq!(buf[(0, 0)].symbol(), "█");
        assert_eq!(buf[(0, 0)].fg, theme.accent);
    }

    #[test]
    fn text_the_font_cannot_spell_falls_back_to_bold_text() {
        let theme = Theme::default();
        let view = LargeText::new("Привіт");
        let buf = render(&view, 10, 1, &theme);
        assert_eq!(grid(&buf), "Привіт    ");
        assert!(buf[(0, 0)].modifier.contains(Modifier::BOLD));
        // And when the block letters do not fit, the same.
        let buf = render(&LargeText::new("HI"), 6, 5, &theme);
        assert_eq!(grid(&buf).lines().next(), Some("HI    "));
        // Opting out of the font.
        let buf = render(&LargeText::new("HI").ascii_fallback(false), 4, 1, &theme);
        assert_eq!(grid(&buf), "HI  ");
    }

    #[test]
    fn line_breaks_and_controls_are_dropped() {
        let theme = Theme::default();
        let ctx = sized(&theme);
        let view = LargeText::new("a\nb\x1b[31m");
        let buf = render_with_context(&view, 20, 2, &ctx);
        let escapes: usize = (0..20)
            .map(|x| buf[(x, 0)].symbol().matches('\x1b').count())
            .sum();
        // Two per sized glyph (open + ST), for a, b, [, 3, 1, m — the space
        // between a and b stays a plain cell.
        assert_eq!(escapes, 12);
    }

    #[test]
    fn a_clipped_block_stays_normal_size() {
        let theme = Theme::default();
        let ctx = sized(&theme);
        let view = LargeText::new("Hi");
        let mut buf = crate::buffer::Buffer::empty(Rect::new(0, 0, 4, 2));
        let mut surface = Surface::new(&mut buf, Rect::new(0, 0, 4, 1));
        view.render(Rect::new(0, 0, 4, 2), &mut surface, &ctx);
        assert_eq!(buf[(0, 0)].symbol(), "H");
    }

    #[test]
    fn degenerate_sizes_do_not_panic() {
        let theme = Theme::default();
        let ctx = sized(&theme);
        for (w, h) in [(0, 0), (1, 1), (0, 5), (5, 0), (3, 1), (2, 2)] {
            let _ = render_with_context(&LargeText::new("Hello world"), w, h, &ctx);
            let _ = render(&LargeText::new("Hello world"), w, h, &theme);
        }
    }
}
