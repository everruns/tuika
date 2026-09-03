//! Interoperability with ratatui widgets.
//!
//! tuika owns its cell grid, so its [`Buffer`] and
//! ratatui's are different types. Everything in this module exists to bridge
//! that one boundary, so a host keeps every ratatui widget ever written:
//! [`RatatuiView`] participates in tuika layout while rendering through
//! [`Surface::render_ratatui`](crate::Surface::render_ratatui), which converts
//! a scratch buffer across and back.
//!
//! The conversion is a cell-by-cell copy of the *rendered area only* — never the
//! whole frame — so it costs one clone per cell a ratatui widget actually
//! touches, and nothing at all for a host that renders no ratatui widgets.
//!
//! The whole module requires the `ratatui` feature, which is what keeps
//! `ratatui-core` out of the default dependency graph.

use ratatui_core::buffer::Buffer as ForeignBuffer;
use ratatui_core::layout::Rect as ForeignRect;
use ratatui_core::style::{Color as ForeignColor, Modifier as ForeignModifier};

use crate::buffer::Buffer;
use crate::geometry::Rect;
use crate::style::{Color, Modifier};
use crate::{RenderCtx, Size, Surface, View};

/// Convert a tuika rect to ratatui's.
pub(crate) fn to_ratatui_rect(rect: Rect) -> ForeignRect {
    ForeignRect::new(rect.x, rect.y, rect.width, rect.height)
}

/// Convert a ratatui rect to tuika's.
pub(crate) fn from_ratatui_rect(rect: ForeignRect) -> Rect {
    Rect::new(rect.x, rect.y, rect.width, rect.height)
}

fn to_ratatui_color(color: Color) -> ForeignColor {
    match color {
        Color::Reset => ForeignColor::Reset,
        Color::Black => ForeignColor::Black,
        Color::Red => ForeignColor::Red,
        Color::Green => ForeignColor::Green,
        Color::Yellow => ForeignColor::Yellow,
        Color::Blue => ForeignColor::Blue,
        Color::Magenta => ForeignColor::Magenta,
        Color::Cyan => ForeignColor::Cyan,
        Color::Gray => ForeignColor::Gray,
        Color::DarkGray => ForeignColor::DarkGray,
        Color::LightRed => ForeignColor::LightRed,
        Color::LightGreen => ForeignColor::LightGreen,
        Color::LightYellow => ForeignColor::LightYellow,
        Color::LightBlue => ForeignColor::LightBlue,
        Color::LightMagenta => ForeignColor::LightMagenta,
        Color::LightCyan => ForeignColor::LightCyan,
        Color::White => ForeignColor::White,
        Color::Rgb(r, g, b) => ForeignColor::Rgb(r, g, b),
        Color::Indexed(i) => ForeignColor::Indexed(i),
    }
}

fn from_ratatui_color(color: ForeignColor) -> Color {
    match color {
        ForeignColor::Reset => Color::Reset,
        ForeignColor::Black => Color::Black,
        ForeignColor::Red => Color::Red,
        ForeignColor::Green => Color::Green,
        ForeignColor::Yellow => Color::Yellow,
        ForeignColor::Blue => Color::Blue,
        ForeignColor::Magenta => Color::Magenta,
        ForeignColor::Cyan => Color::Cyan,
        ForeignColor::Gray => Color::Gray,
        ForeignColor::DarkGray => Color::DarkGray,
        ForeignColor::LightRed => Color::LightRed,
        ForeignColor::LightGreen => Color::LightGreen,
        ForeignColor::LightYellow => Color::LightYellow,
        ForeignColor::LightBlue => Color::LightBlue,
        ForeignColor::LightMagenta => Color::LightMagenta,
        ForeignColor::LightCyan => Color::LightCyan,
        ForeignColor::White => Color::White,
        ForeignColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
        ForeignColor::Indexed(i) => Color::Indexed(i),
    }
}

/// Both sides use the same bit layout, so modifiers convert by value.
///
/// Asserted by `modifier_bits_line_up` below rather than assumed — a
/// reordering upstream would otherwise silently swap bold for dim.
fn to_ratatui_modifier(modifier: Modifier) -> ForeignModifier {
    ForeignModifier::from_bits_truncate(modifier.bits())
}

fn from_ratatui_modifier(modifier: ForeignModifier) -> Modifier {
    [
        (ForeignModifier::BOLD, Modifier::BOLD),
        (ForeignModifier::DIM, Modifier::DIM),
        (ForeignModifier::ITALIC, Modifier::ITALIC),
        (ForeignModifier::UNDERLINED, Modifier::UNDERLINED),
        (ForeignModifier::SLOW_BLINK, Modifier::SLOW_BLINK),
        (ForeignModifier::RAPID_BLINK, Modifier::RAPID_BLINK),
        (ForeignModifier::REVERSED, Modifier::REVERSED),
        (ForeignModifier::HIDDEN, Modifier::HIDDEN),
        (ForeignModifier::CROSSED_OUT, Modifier::CROSSED_OUT),
    ]
    .into_iter()
    .filter(|(foreign, _)| modifier.contains(*foreign))
    .fold(Modifier::empty(), |acc, (_, own)| acc.union(own))
}

/// Convert one cell to ratatui's.
///
/// Only the differential backend test needs a single cell; the render path
/// converts whole buffers.
#[cfg(test)]
pub(crate) fn to_ratatui_cell(cell: &crate::buffer::Cell) -> ratatui_core::buffer::Cell {
    // `Cell::new` upstream takes `&'static str`; a grapheme from our grid is
    // borrowed, so the symbol is set after construction.
    let mut out = ratatui_core::buffer::Cell::default();
    out.set_symbol(cell.symbol());
    out.set_fg(to_ratatui_color(cell.fg));
    out.set_bg(to_ratatui_color(cell.bg));
    out.underline_color = to_ratatui_color(cell.underline_color);
    out.modifier = to_ratatui_modifier(cell.modifier);
    out
}

/// Copy a tuika buffer into a fresh ratatui one covering the same area.
pub(crate) fn to_ratatui_buffer(source: &Buffer) -> ForeignBuffer {
    let mut out = ForeignBuffer::empty(to_ratatui_rect(source.area));
    for y in source.area.y..source.area.bottom() {
        for x in source.area.x..source.area.right() {
            let Some(cell) = source.cell((x, y)) else {
                continue;
            };
            let target = &mut out[(x, y)];
            target.set_symbol(cell.symbol());
            target.set_fg(to_ratatui_color(cell.fg));
            target.set_bg(to_ratatui_color(cell.bg));
            target.modifier = to_ratatui_modifier(cell.modifier);
        }
    }
    out
}

/// Copy a ratatui buffer back over a tuika one, cell for cell.
///
/// Only the overlap is copied: a widget that resized its buffer cannot enlarge
/// the destination, which is what keeps the clip guarantee intact.
pub(crate) fn from_ratatui_buffer(source: &ForeignBuffer, destination: &mut Buffer) {
    let overlap = from_ratatui_rect(source.area).intersection(destination.area);
    for y in overlap.y..overlap.bottom() {
        for x in overlap.x..overlap.right() {
            let Some(cell) = source.cell((x, y)) else {
                continue;
            };
            let Some(target) = destination.cell_mut((x, y)) else {
                continue;
            };
            target.set_symbol(cell.symbol());
            target.fg = from_ratatui_color(cell.fg);
            target.bg = from_ratatui_color(cell.bg);
            target.modifier = from_ratatui_modifier(cell.modifier);
        }
    }
}

/// A tuika view backed by a ratatui rendering closure.
///
/// The closure form is intentional: many ratatui widgets borrow their data.
/// Captured owned data can be borrowed while constructing the widget inside
/// the closure, without requiring a self-referential wrapper.
pub struct RatatuiView<F> {
    measure: Measure,
    render: F,
}

#[derive(Clone, Copy, Debug)]
enum Measure {
    Fill,
    Fixed(Size),
}

impl<F> RatatuiView<F>
where
    F: Fn(ForeignRect, &mut ForeignBuffer),
{
    /// Create a view that requests all space offered by its parent.
    ///
    /// ```
    /// use ratatui::widgets::{Block, Widget};
    /// use tuika::interop::RatatuiView;
    ///
    /// let view = RatatuiView::fill(|area, buffer| {
    ///     Block::bordered().title(" ratatui ").render(area, buffer);
    /// });
    /// ```
    pub fn fill(render: F) -> Self {
        Self {
            measure: Measure::Fill,
            render,
        }
    }

    /// Create a view with a fixed intrinsic size for `Dimension::Auto` layout.
    pub fn sized(size: Size, render: F) -> Self {
        Self {
            measure: Measure::Fixed(size),
            render,
        }
    }
}

impl<F> View for RatatuiView<F>
where
    F: Fn(ForeignRect, &mut ForeignBuffer),
{
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        match self.measure {
            Measure::Fill => available,
            Measure::Fixed(size) => Size::new(
                size.width.min(available.width),
                size.height.min(available.height),
            ),
        }
    }

    fn render(&self, area: Rect, surface: &mut Surface, _ctx: &RenderCtx) {
        surface.render_ratatui(area, &self.render);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Style;
    use crate::style::Theme;
    use crate::tests::support::row;

    #[test]
    fn ratatui_view_renders_borrowing_widget_data() {
        use ratatui::widgets::{Sparkline, Widget};

        let data = vec![1, 2, 4, 8];
        let view = RatatuiView::fill(move |area, buffer| {
            Sparkline::default().data(&data).render(area, buffer);
        });
        let theme = Theme::default();
        let rendered = crate::testing::render(&view, 4, 1, &theme);
        assert_ne!(row(&rendered, 0), "");
    }

    /// The conversion is only sound while the two bit layouts agree; this is the
    /// assertion that catches an upstream reordering instead of shipping bold
    /// text rendered as dim.
    #[test]
    fn modifier_bits_line_up() {
        for (own, foreign) in [
            (Modifier::BOLD, ForeignModifier::BOLD),
            (Modifier::DIM, ForeignModifier::DIM),
            (Modifier::ITALIC, ForeignModifier::ITALIC),
            (Modifier::UNDERLINED, ForeignModifier::UNDERLINED),
            (Modifier::SLOW_BLINK, ForeignModifier::SLOW_BLINK),
            (Modifier::RAPID_BLINK, ForeignModifier::RAPID_BLINK),
            (Modifier::REVERSED, ForeignModifier::REVERSED),
            (Modifier::HIDDEN, ForeignModifier::HIDDEN),
            (Modifier::CROSSED_OUT, ForeignModifier::CROSSED_OUT),
        ] {
            assert_eq!(own.bits(), foreign.bits(), "{own:?} vs {foreign:?}");
        }
    }

    #[test]
    fn every_color_survives_a_round_trip() {
        let colors = [
            Color::Reset,
            Color::Black,
            Color::Red,
            Color::Green,
            Color::Yellow,
            Color::Blue,
            Color::Magenta,
            Color::Cyan,
            Color::Gray,
            Color::DarkGray,
            Color::LightRed,
            Color::LightGreen,
            Color::LightYellow,
            Color::LightBlue,
            Color::LightMagenta,
            Color::LightCyan,
            Color::White,
            Color::Rgb(1, 2, 3),
            Color::Indexed(42),
        ];
        for color in colors {
            assert_eq!(from_ratatui_color(to_ratatui_color(color)), color);
        }
    }

    #[test]
    fn every_modifier_survives_a_round_trip() {
        let all = Modifier::all();
        assert_eq!(from_ratatui_modifier(to_ratatui_modifier(all)), all);
        assert_eq!(
            from_ratatui_modifier(to_ratatui_modifier(Modifier::empty())),
            Modifier::empty()
        );
    }

    /// Symbols, colors and attributes all have to survive the trip, or a widget
    /// would render with the right glyphs and the wrong paint.
    #[test]
    fn a_buffer_survives_a_round_trip() {
        let mut original = Buffer::empty(Rect::new(0, 0, 3, 1));
        original.set_string(
            0,
            0,
            "ab",
            Style::new()
                .fg(Color::Rgb(9, 9, 9))
                .bg(Color::Indexed(4))
                .bold(),
        );

        let foreign = to_ratatui_buffer(&original);
        let mut restored = Buffer::empty(Rect::new(0, 0, 3, 1));
        from_ratatui_buffer(&foreign, &mut restored);

        assert_eq!(restored[(0, 0)].symbol(), "a");
        assert_eq!(restored[(0, 0)].fg, Color::Rgb(9, 9, 9));
        assert_eq!(restored[(0, 0)].bg, Color::Indexed(4));
        assert!(restored[(0, 0)].modifier.contains(Modifier::BOLD));
    }

    /// A widget that grows its buffer must not grow the destination — that is
    /// the containment guarantee `render_scratch` exists to provide.
    #[test]
    fn a_larger_foreign_buffer_cannot_enlarge_the_destination() {
        let foreign = ForeignBuffer::filled(
            ForeignRect::new(0, 0, 8, 4),
            ratatui_core::buffer::Cell::new("x"),
        );
        let mut destination = Buffer::empty(Rect::new(0, 0, 2, 1));
        from_ratatui_buffer(&foreign, &mut destination);
        assert_eq!(destination.area, Rect::new(0, 0, 2, 1));
        assert_eq!(destination[(0, 0)].symbol(), "x");
    }
}
