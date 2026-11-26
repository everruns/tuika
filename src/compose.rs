//! Alpha compositing of overlay layers over the cells already painted.
//!
//! A terminal cell has no alpha channel, so translucency is resolved once, on
//! the cell grid, when a layer is composited: the result is an ordinary opaque
//! cell whose colors are the blend. Only [`Color::Rgb`] values carry arithmetic
//! tuika can blend; [`Color::Reset`] resolves to the theme slot it stands for
//! (when that slot is itself RGB). Anything else — an indexed or named ANSI
//! color, or a [`themes::TERMINAL`](crate::themes::TERMINAL)-style theme built
//! from them — has no known RGB without the terminal's palette, so those cells
//! take the non-truecolor fallback instead of a guessed blend: the dim
//! modifier for a backdrop, and the opaque overlay cell for a panel.

use crate::buffer::Cell;
use crate::buffer::{Buffer, CellDiffOption};
use crate::geometry::Rect;
use crate::style::{Color, Modifier, Style, Theme};

/// Blend `top` over `bottom` with `alpha` in `0..=255` (255 is `top`).
fn mix(bottom: (u8, u8, u8), top: (u8, u8, u8), alpha: u8) -> Color {
    let a = u16::from(alpha);
    // Integer rounding keeps the result exact at both ends of the range and
    // deterministic across platforms (golden snapshots compare cells).
    let channel = |b: u8, t: u8| ((u16::from(b) * (255 - a) + u16::from(t) * a + 127) / 255) as u8;
    Color::Rgb(
        channel(bottom.0, top.0),
        channel(bottom.1, top.1),
        channel(bottom.2, top.2),
    )
}

/// The RGB value of `color`, resolving [`Color::Reset`] to `reset`.
fn rgb(color: Color, reset: Color) -> Option<(u8, u8, u8)> {
    match color {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        Color::Reset => match reset {
            Color::Rgb(r, g, b) => Some((r, g, b)),
            _ => None,
        },
        _ => None,
    }
}

/// Wash `area` of `buffer` toward `tint` by `alpha`, the way a translucent
/// sheet laid over the layers behind it would look.
///
/// Background, foreground, and underline colors are all blended, so text
/// behind the sheet fades with it. A cell whose colors cannot be blended (see
/// the module docs) gets the dim modifier instead; when `tint` itself has no
/// RGB value the whole area is dimmed.
pub(crate) fn tint(buffer: &mut Buffer, area: Rect, tint: Color, alpha: u8, theme: &Theme) {
    let area = area.intersection(buffer.area);
    let Some(top) = rgb(tint, theme.background) else {
        dim(buffer, area);
        return;
    };
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let cell = &mut buffer[(x, y)];
            let bg = rgb(cell.bg, theme.background);
            let fg = rgb(cell.fg, theme.text);
            match (bg, fg) {
                (Some(bg), Some(fg)) => {
                    cell.bg = mix(bg, top, alpha);
                    cell.fg = mix(fg, top, alpha);
                    if let Some(underline) = rgb(cell.underline_color, theme.text) {
                        cell.underline_color = mix(underline, top, alpha);
                    }
                }
                _ => cell.modifier = cell.modifier.union(Modifier::DIM),
            }
        }
    }
}

/// Add the dim modifier to every cell of `area`.
pub(crate) fn dim(buffer: &mut Buffer, area: Rect) {
    let area = area.intersection(buffer.area);
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let cell = &mut buffer[(x, y)];
            cell.modifier = cell.modifier.union(Modifier::DIM);
        }
    }
}

/// Marks a cell the translucent layer has not painted. U+FFFF is a Unicode
/// noncharacter, so no view draws it on purpose, and [`over`] replaces every
/// occurrence before the frame leaves the compositor.
const UNPAINTED: &str = "\u{FFFF}";

/// Prepare `area` for a translucent layer: return what it covers (row-major,
/// for [`over`]) and mark every cell unpainted, filling it with `clear` when
/// the layer clears its rect.
///
/// The mark is what lets [`over`] tell a cell the layer left alone — where the
/// content beneath should show through — from one it drew, even a space in a
/// title or a padded row, which stays the layer's own.
pub(crate) fn begin(buffer: &mut Buffer, area: Rect, clear: Option<Style>) -> Vec<Cell> {
    let area = area.intersection(buffer.area);
    let mut under = Vec::with_capacity(area.area() as usize);
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let cell = &mut buffer[(x, y)];
            under.push(cell.clone());
            cell.set_symbol(UNPAINTED);
            if let Some(style) = clear {
                cell.set_style(style);
            }
        }
    }
    under
}

/// Composite the layer just painted into `area` over `under`, the cells
/// [`begin`] returned for the same area, at `alpha` opacity.
///
/// The layer's background blends with what was beneath it, and the glyphs it
/// drew stay at full strength so a panel's own text remains legible. Where it
/// drew nothing, the glyph beneath shows through, washed toward the layer's
/// background by the same amount. A cell that cannot blend (see the module
/// docs), or that holds an image placeholder, keeps the layer's opaque cell.
pub(crate) fn over(buffer: &mut Buffer, area: Rect, under: &[Cell], alpha: u8, theme: &Theme) {
    let area = area.intersection(buffer.area);
    debug_assert_eq!(under.len(), area.area() as usize);
    let width = area.width as usize;
    for row in 0..area.height {
        let y = area.y + row;
        let below_at = |col: u16| &under[row as usize * width + col as usize];
        let mut col = 0u16;
        while col < area.width {
            let x = area.x + col;
            let below = below_at(col);
            col += 1;
            let unpainted = buffer[(x, y)].raw_symbol() == UNPAINTED;
            if unpainted {
                buffer[(x, y)].set_symbol(" ");
            }
            let above = &buffer[(x, y)];
            if above.diff_option != CellDiffOption::None
                || below.diff_option != CellDiffOption::None
            {
                continue;
            }
            let (Some(top_bg), Some(bottom_bg)) = (
                rgb(above.bg, theme.background),
                rgb(below.bg, theme.background),
            ) else {
                continue;
            };
            let bg = mix(bottom_bg, top_bg, alpha);
            buffer[(x, y)].bg = bg;
            let below_fg = rgb(below.fg, theme.text);
            let shows = unpainted && below.raw_symbol().trim() != "" && below_fg.is_some();
            if !shows {
                continue;
            }
            let wide = below.cell_width() > 1;
            // A wide glyph needs its placeholder column inside the layer and
            // still unpainted there; otherwise it would be cut in half, so the
            // cell stays blank.
            if wide && (col >= area.width || buffer[(x + 1, y)].raw_symbol() != UNPAINTED) {
                continue;
            }
            let cell = &mut buffer[(x, y)];
            cell.set_symbol(below.raw_symbol());
            cell.fg = mix(below_fg.unwrap_or(top_bg), top_bg, alpha);
            cell.modifier = below.modifier;
            if wide {
                let next = &mut buffer[(x + 1, y)];
                let next_bg = rgb(below_at(col).bg, theme.background).unwrap_or(bottom_bg);
                let next_top = rgb(next.bg, theme.background).unwrap_or(top_bg);
                next.set_symbol("");
                next.bg = mix(next_bg, next_top, alpha);
                col += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> Theme {
        Theme {
            background: Color::Rgb(0, 0, 0),
            text: Color::Rgb(200, 200, 200),
            ..Theme::default()
        }
    }

    #[test]
    fn mix_is_exact_at_the_ends_and_rounds_between() {
        assert_eq!(mix((0, 0, 0), (255, 100, 10), 0), Color::Rgb(0, 0, 0));
        assert_eq!(
            mix((0, 0, 0), (255, 100, 10), 255),
            Color::Rgb(255, 100, 10)
        );
        assert_eq!(
            mix((0, 0, 0), (255, 255, 255), 128),
            Color::Rgb(128, 128, 128)
        );
    }

    #[test]
    fn tint_blends_rgb_cells_and_resolves_reset_through_the_theme() {
        let area = Rect::new(0, 0, 2, 1);
        let mut buffer = Buffer::empty(area);
        buffer[(0, 0)].bg = Color::Rgb(100, 100, 100);
        buffer[(0, 0)].fg = Color::Rgb(200, 0, 0);
        // (1, 0) stays Reset/Reset: it resolves to theme.background/text.
        tint(&mut buffer, area, Color::Rgb(0, 0, 0), 128, &theme());
        assert_eq!(buffer[(0, 0)].bg, Color::Rgb(50, 50, 50));
        assert_eq!(buffer[(0, 0)].fg, Color::Rgb(100, 0, 0));
        assert_eq!(buffer[(1, 0)].bg, Color::Rgb(0, 0, 0));
        assert_eq!(buffer[(1, 0)].fg, Color::Rgb(100, 100, 100));
        assert!(!buffer[(0, 0)].modifier.contains(Modifier::DIM));
    }

    #[test]
    fn tint_falls_back_to_dim_without_rgb() {
        let area = Rect::new(0, 0, 2, 1);
        let mut buffer = Buffer::empty(area);
        buffer[(0, 0)].bg = Color::Indexed(4);
        buffer[(1, 0)].bg = Color::Rgb(9, 9, 9);
        tint(&mut buffer, area, Color::Rgb(0, 0, 0), 128, &theme());
        assert!(buffer[(0, 0)].modifier.contains(Modifier::DIM));
        assert_eq!(buffer[(0, 0)].bg, Color::Indexed(4));
        assert!(!buffer[(1, 0)].modifier.contains(Modifier::DIM));

        let mut buffer = Buffer::empty(area);
        tint(&mut buffer, area, Color::Blue, 128, &theme());
        assert!(
            buffer
                .content
                .iter()
                .all(|c| c.modifier.contains(Modifier::DIM))
        );
        assert!(buffer.content.iter().all(|c| c.bg == Color::Reset));
    }

    fn white() -> Option<Style> {
        Some(Style::default().bg(Color::Rgb(255, 255, 255)))
    }

    #[test]
    fn over_blends_background_and_shows_through_where_the_layer_drew_nothing() {
        let area = Rect::new(0, 0, 4, 1);
        let mut buffer = Buffer::empty(area);
        buffer.set_string(0, 0, "abcd", Style::default().fg(Color::Rgb(200, 0, 0)));
        let under = begin(&mut buffer, area, white());
        // The layer draws one glyph and one deliberate space of its own.
        buffer[(1, 0)].set_char('X').fg = Color::Rgb(0, 0, 255);
        buffer[(2, 0)].set_char(' ');
        over(&mut buffer, area, &under, 128, &theme());

        assert_eq!(buffer[(0, 0)].symbol(), "a", "unpainted cell shows through");
        assert_eq!(buffer[(0, 0)].bg, Color::Rgb(128, 128, 128));
        assert_eq!(buffer[(0, 0)].fg, Color::Rgb(228, 128, 128));
        assert_eq!(buffer[(1, 0)].symbol(), "X", "the layer's own glyph wins");
        assert_eq!(buffer[(1, 0)].fg, Color::Rgb(0, 0, 255));
        assert_eq!(buffer[(1, 0)].bg, Color::Rgb(128, 128, 128));
        assert_eq!(buffer[(2, 0)].symbol(), " ", "a drawn space is the layer's");
        assert_eq!(buffer[(3, 0)].symbol(), "d");
    }

    #[test]
    fn over_keeps_wide_glyphs_whole_or_not_at_all() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 1));
        buffer.set_string(1, 0, "界", Style::default());
        // The layer covers columns 0..2: the glyph at 1 would need column 2.
        let layer = Rect::new(0, 0, 2, 1);
        let under = begin(&mut buffer, layer, white());
        over(&mut buffer, layer, &under, 128, &theme());
        assert_eq!(
            buffer[(1, 0)].symbol(),
            " ",
            "half a wide glyph is never drawn"
        );

        let area = Rect::new(0, 0, 3, 1);
        let mut buffer = Buffer::empty(area);
        buffer.set_string(0, 0, "界", Style::default());
        let under = begin(&mut buffer, area, white());
        over(&mut buffer, area, &under, 128, &theme());
        assert_eq!(buffer[(0, 0)].symbol(), "界");
        assert_eq!(buffer[(1, 0)].raw_symbol(), "");
    }

    #[test]
    fn over_leaves_unblendable_cells_opaque_and_never_leaks_the_mark() {
        let area = Rect::new(0, 0, 2, 1);
        let mut buffer = Buffer::empty(area);
        buffer.set_string(0, 0, "ab", Style::default().bg(Color::Indexed(3)));
        let under = begin(&mut buffer, area, white());
        over(&mut buffer, area, &under, 128, &theme());
        assert_eq!(buffer[(0, 0)].symbol(), " ");
        assert_eq!(buffer[(0, 0)].bg, Color::Rgb(255, 255, 255));
        assert!(buffer.content.iter().all(|c| c.raw_symbol() != UNPAINTED));
    }
}
