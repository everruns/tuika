//! tuika's ratatui [`Backend`], written directly against crossterm.
//!
//! This is the only piece tuika needs from `ratatui-crossterm`, and taking it
//! in-house drops that crate and the eight-crate `instability`/`darling`
//! proc-macro tree behind it from every consumer's graph. The trade is small
//! because tuika already owned the other half of this boundary:
//! [`HyperlinkBackend`](super::hyperlink::HyperlinkBackend) already implements
//! `Backend` end to end and already maps ratatui colors and styles onto
//! crossterm commands for `write_line`. What was left was the cell-drawing loop
//! and a handful of one-line command wrappers.
//!
//! The emitted byte stream is deliberately identical to `ratatui-crossterm`'s:
//! same cursor-move elision for a contiguous run, same attribute-diff order,
//! same trailing reset. `tests/pty_smoke.rs` asserts the result through a
//! reference terminal parser, which is what makes owning this safe rather than
//! brave.

use std::io::{self, Write};

use crate::buffer::Cell;
use crate::geometry::{Position, Size};
use crate::style::{Color, Modifier};
use crate::term::traits::{Backend, ClearType, WindowSize};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::queue;
use crossterm::style::{
    Attribute as CtAttribute, Color as CtColor, Colors as CtColors, Print, SetAttribute,
    SetBackgroundColor, SetColors, SetForegroundColor, SetUnderlineColor,
};
use crossterm::terminal::{self, Clear};

use super::hyperlink::to_ct_color;

/// A [`Backend`] that draws through crossterm.
///
/// Most hosts never name this: [`Runner`](crate::Runner) and
/// [`AsyncRunner`](crate::AsyncRunner) build one, and a host wanting OSC 8
/// hyperlinks wraps its writer in
/// [`HyperlinkBackend`](super::hyperlink::HyperlinkBackend) instead. It is
/// public for the host that drives
/// [`Terminal`](crate::term::terminal::Terminal) itself.
pub struct CrosstermBackend<W: Write> {
    writer: W,
}

impl<W: Write> CrosstermBackend<W> {
    /// Draw into `writer` — in production the process's stdout, in tests a
    /// `Vec<u8>` the assertions read back.
    pub const fn new(writer: W) -> Self {
        Self { writer }
    }
}

impl<W: Write> Write for CrosstermBackend<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writer.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

impl<W: Write> Backend for CrosstermBackend<W> {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        // ratatui hands over only the cells that changed, so the stream is
        // sparse. Everything here exists to keep it small: the cursor moves only
        // when the next cell is not adjacent to the last, and colors and
        // attributes are re-emitted only on a change.
        let mut fg = Color::Reset;
        let mut bg = Color::Reset;
        let mut underline_color = Color::Reset;
        let mut modifier = Modifier::empty();
        let mut last_pos: Option<Position> = None;
        for (x, y, cell) in content {
            if !matches!(last_pos, Some(p) if x == p.x + 1 && y == p.y) {
                queue!(self.writer, MoveTo(x, y))?;
            }
            last_pos = Some(Position { x, y });
            if cell.modifier != modifier {
                queue_modifier_diff(&mut self.writer, modifier, cell.modifier)?;
                modifier = cell.modifier;
            }
            if cell.fg != fg || cell.bg != bg {
                queue!(
                    self.writer,
                    SetColors(CtColors::new(to_ct_color(cell.fg), to_ct_color(cell.bg)))
                )?;
                fg = cell.fg;
                bg = cell.bg;
            }
            if cell.underline_color != underline_color {
                queue!(
                    self.writer,
                    SetUnderlineColor(to_ct_color(cell.underline_color))
                )?;
                underline_color = cell.underline_color;
            }
            queue!(self.writer, Print(cell.symbol()))?;
        }
        // Leave the terminal in a neutral state: whatever the host prints next
        // — a published scrollback block, its own shell prompt after exit — must
        // not inherit the last cell's color or attributes.
        queue!(
            self.writer,
            SetForegroundColor(CtColor::Reset),
            SetBackgroundColor(CtColor::Reset),
            SetUnderlineColor(CtColor::Reset),
            SetAttribute(CtAttribute::Reset),
        )
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        queue!(self.writer, Hide)?;
        self.writer.flush()
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        queue!(self.writer, Show)?;
        self.writer.flush()
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        crossterm::cursor::position()
            .map(|(x, y)| Position { x, y })
            .map_err(io::Error::other)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        let Position { x, y } = position.into();
        queue!(self.writer, MoveTo(x, y))?;
        self.writer.flush()
    }

    fn clear(&mut self) -> io::Result<()> {
        self.clear_region(ClearType::All)
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        queue!(
            self.writer,
            Clear(match clear_type {
                ClearType::All => terminal::ClearType::All,
                ClearType::AfterCursor => terminal::ClearType::FromCursorDown,
                ClearType::BeforeCursor => terminal::ClearType::FromCursorUp,
                ClearType::CurrentLine => terminal::ClearType::CurrentLine,
                ClearType::UntilNewLine => terminal::ClearType::UntilNewLine,
            })
        )?;
        self.writer.flush()
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        for _ in 0..n {
            queue!(self.writer, Print("\n"))?;
        }
        self.writer.flush()
    }

    fn size(&self) -> io::Result<Size> {
        let (width, height) = terminal::size()?;
        Ok(Size { width, height })
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        let terminal::WindowSize {
            columns,
            rows,
            width,
            height,
        } = terminal::window_size()?;
        Ok(WindowSize {
            columns_rows: Size {
                width: columns,
                height: rows,
            },
            pixels: Size { width, height },
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }

    #[cfg(feature = "scrolling-regions")]
    fn scroll_region_up(&mut self, region: std::ops::Range<u16>, amount: u16) -> io::Result<()> {
        queue!(self.writer, ScrollInRegion::up(region, amount))?;
        self.writer.flush()
    }

    #[cfg(feature = "scrolling-regions")]
    fn scroll_region_down(&mut self, region: std::ops::Range<u16>, amount: u16) -> io::Result<()> {
        queue!(self.writer, ScrollInRegion::down(region, amount))?;
        self.writer.flush()
    }
}

/// Emit the attribute changes that turn `from` into `to`.
///
/// Order matters and is not arbitrary. Bold and dim share one SGR intensity
/// slot, so dropping either has to reset the slot and then re-apply whichever
/// of the two survives — which is why the intensity reset is decided first and
/// the `added` pass skips bold/dim when it already ran. Slow and rapid blink
/// share a slot the same way.
fn queue_modifier_diff<W: Write>(w: &mut W, from: Modifier, to: Modifier) -> io::Result<()> {
    let removed = from - to;
    if removed.contains(Modifier::REVERSED) {
        queue!(w, SetAttribute(CtAttribute::NoReverse))?;
    }
    let reset_intensity = removed.contains(Modifier::BOLD) || removed.contains(Modifier::DIM);
    if reset_intensity {
        queue!(w, SetAttribute(CtAttribute::NormalIntensity))?;
        if to.contains(Modifier::DIM) {
            queue!(w, SetAttribute(CtAttribute::Dim))?;
        }
        if to.contains(Modifier::BOLD) {
            queue!(w, SetAttribute(CtAttribute::Bold))?;
        }
    }
    if removed.contains(Modifier::ITALIC) {
        queue!(w, SetAttribute(CtAttribute::NoItalic))?;
    }
    if removed.contains(Modifier::UNDERLINED) {
        queue!(w, SetAttribute(CtAttribute::NoUnderline))?;
    }
    if removed.contains(Modifier::CROSSED_OUT) {
        queue!(w, SetAttribute(CtAttribute::NotCrossedOut))?;
    }
    if removed.contains(Modifier::HIDDEN) {
        queue!(w, SetAttribute(CtAttribute::NoHidden))?;
    }
    if removed.contains(Modifier::SLOW_BLINK) || removed.contains(Modifier::RAPID_BLINK) {
        queue!(w, SetAttribute(CtAttribute::NoBlink))?;
    }

    let added = to - from;
    if added.contains(Modifier::REVERSED) {
        queue!(w, SetAttribute(CtAttribute::Reverse))?;
    }
    if added.contains(Modifier::BOLD) && !reset_intensity {
        queue!(w, SetAttribute(CtAttribute::Bold))?;
    }
    if added.contains(Modifier::ITALIC) {
        queue!(w, SetAttribute(CtAttribute::Italic))?;
    }
    if added.contains(Modifier::UNDERLINED) {
        queue!(w, SetAttribute(CtAttribute::Underlined))?;
    }
    if added.contains(Modifier::DIM) && !reset_intensity {
        queue!(w, SetAttribute(CtAttribute::Dim))?;
    }
    if added.contains(Modifier::CROSSED_OUT) {
        queue!(w, SetAttribute(CtAttribute::CrossedOut))?;
    }
    if added.contains(Modifier::HIDDEN) {
        queue!(w, SetAttribute(CtAttribute::Hidden))?;
    }
    if added.contains(Modifier::SLOW_BLINK) {
        queue!(w, SetAttribute(CtAttribute::SlowBlink))?;
    }
    if added.contains(Modifier::RAPID_BLINK) {
        queue!(w, SetAttribute(CtAttribute::RapidBlink))?;
    }
    Ok(())
}

/// Scroll a DECSTBM region, the command crossterm itself does not expose.
///
/// Reached only under the `scrolling-regions` feature. The region is set,
/// scrolled, and reset in one command so no other writer can observe a
/// half-configured terminal — a leaked scrolling region would confine every
/// later write to those rows.
#[cfg(feature = "scrolling-regions")]
struct ScrollInRegion {
    first_row: u16,
    last_row: u16,
    lines: u16,
    /// `S` scrolls the region's content up, `T` down.
    verb: char,
}

#[cfg(feature = "scrolling-regions")]
impl ScrollInRegion {
    fn up(region: std::ops::Range<u16>, lines: u16) -> Self {
        Self::new(region, lines, 'S')
    }

    fn down(region: std::ops::Range<u16>, lines: u16) -> Self {
        Self::new(region, lines, 'T')
    }

    fn new(region: std::ops::Range<u16>, lines: u16, verb: char) -> Self {
        Self {
            first_row: region.start,
            last_row: region.end.saturating_sub(1),
            lines,
            verb,
        }
    }
}

#[cfg(feature = "scrolling-regions")]
impl crossterm::Command for ScrollInRegion {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        if self.lines == 0 {
            return Ok(());
        }
        // DECSTBM rows are 1-based.
        write!(
            f,
            "\x1b[{};{}r\x1b[{}{}\x1b[r",
            self.first_row.saturating_add(1),
            self.last_row.saturating_add(1),
            self.lines,
            self.verb,
        )
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "scrolling regions are not supported through the Windows console API",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Style;

    /// Draw `cells` at consecutive columns on row 0 and return the byte stream.
    fn drawn(cells: &[(&'static str, Style)]) -> String {
        let owned: Vec<Cell> = cells
            .iter()
            .map(|&(symbol, style)| {
                let mut cell = Cell::new(symbol);
                cell.set_style(style);
                cell
            })
            .collect();
        let mut backend = CrosstermBackend::new(Vec::<u8>::new());
        backend
            .draw(
                owned
                    .iter()
                    .enumerate()
                    .map(|(i, cell)| (i as u16, 0u16, cell)),
            )
            .expect("draw");
        String::from_utf8(backend.writer).expect("utf8")
    }

    #[test]
    fn a_contiguous_run_moves_the_cursor_once() {
        let out = drawn(&[
            ("a", Style::default()),
            ("b", Style::default()),
            ("c", Style::default()),
        ]);
        assert_eq!(out.matches("\x1b[1;1H").count(), 1);
        assert_eq!(
            out.matches('H').count(),
            1,
            "no second cursor move: {out:?}"
        );
        assert!(out.contains('a') && out.contains('b') && out.contains('c'));
    }

    #[test]
    fn a_gap_in_the_run_re_emits_a_cursor_move() {
        let a = Cell::new("a");
        let b = Cell::new("b");
        let mut backend = CrosstermBackend::new(Vec::<u8>::new());
        backend
            .draw([(0u16, 0u16, &a), (5u16, 0u16, &b)].into_iter())
            .expect("draw");
        let out = String::from_utf8(backend.writer).expect("utf8");
        assert!(out.contains("\x1b[1;1H"), "{out:?}");
        assert!(out.contains("\x1b[1;6H"), "{out:?}");
    }

    #[test]
    fn colors_are_emitted_once_per_change_and_reset_at_the_end() {
        let red = Style::default().fg(Color::Rgb(255, 0, 0));
        let out = drawn(&[("a", red), ("b", red), ("c", Style::default())]);
        assert_eq!(
            out,
            // Move once, set the pair once for "ab", set it once more for "c",
            // then reset fg/bg/underline/attributes. Not one color run per cell.
            "\x1b[1;1H\x1b[38;2;255;0;0;49mab\x1b[39;49mc\x1b[39m\x1b[49m\x1b[59m\x1b[0m"
        );
    }

    #[test]
    fn dropping_bold_while_keeping_dim_reapplies_dim() {
        // Bold and dim share the SGR intensity slot, so clearing bold clears dim
        // too — the diff has to put it back or the run silently brightens.
        let mut out = Vec::new();
        queue_modifier_diff(&mut out, Modifier::BOLD | Modifier::DIM, Modifier::DIM).expect("diff");
        let out = String::from_utf8(out).expect("utf8");
        assert_eq!(out, "\x1b[22m\x1b[2m", "normal intensity, then dim again");
    }

    #[test]
    fn adding_an_attribute_does_not_reset_the_others() {
        let mut out = Vec::new();
        queue_modifier_diff(&mut out, Modifier::BOLD, Modifier::BOLD | Modifier::ITALIC)
            .expect("diff");
        let out = String::from_utf8(out).expect("utf8");
        assert_eq!(out, "\x1b[3m", "italic only: {out:?}");
    }

    /// Byte-for-byte parity with the crate this backend replaced.
    ///
    /// `ratatui-crossterm` is still reachable from tests through the `ratatui`
    /// dev-dependency, so the removal can be *proved* rather than asserted:
    /// every terminal tuika supports already agrees on what that byte stream
    /// means. The corpus below is chosen to hit the paths where a hand-written
    /// backend would plausibly drift — cursor elision, the shared
    /// intensity/blink SGR slots, indexed and truecolor pairs, underline color,
    /// and wide/zero-width symbols.
    ///
    /// Gated on the `ratatui` feature because the comparison needs the cell
    /// conversion in [`crate::interop`], which is what that feature turns on.
    /// CI runs `--all-features`, so the proof still runs on every change.
    #[cfg(feature = "ratatui")]
    #[test]
    fn the_byte_stream_matches_ratatui_crossterm_exactly() {
        use ratatui::backend::CrosstermBackend as Reference;

        let styles = [
            Style::default(),
            Style::default().fg(Color::Rgb(1, 2, 3)),
            Style::default().fg(Color::Indexed(200)).bg(Color::Reset),
            Style::default().add_modifier(Modifier::BOLD | Modifier::DIM),
            Style::default().add_modifier(Modifier::DIM),
            Style::default().add_modifier(Modifier::ITALIC | Modifier::UNDERLINED),
            Style::default()
                .add_modifier(Modifier::SLOW_BLINK)
                .underline_color(Color::LightRed),
            Style::default().add_modifier(Modifier::RAPID_BLINK),
            Style::default().add_modifier(Modifier::REVERSED | Modifier::CROSSED_OUT),
            Style::default().add_modifier(Modifier::HIDDEN),
            Style::default().bg(Color::Rgb(9, 9, 9)).fg(Color::White),
        ];
        let symbols = ["a", "世", "\u{301}", " ", "▁"];

        let cells: Vec<(u16, u16, Cell)> = styles
            .iter()
            .enumerate()
            .map(|(i, style)| {
                let mut cell = Cell::new(symbols[i % symbols.len()]);
                cell.set_style(*style);
                // Every third cell jumps a column and a row, so both the
                // contiguous-run path and the cursor-move path are exercised.
                let x = if i % 3 == 0 { i as u16 + 1 } else { i as u16 };
                (x, (i / 4) as u16, cell)
            })
            .collect();

        let mut ours = CrosstermBackend::new(Vec::<u8>::new());
        ours.draw(cells.iter().map(|(x, y, cell)| (*x, *y, cell)))
            .expect("draw");
        let foreign: Vec<(u16, u16, ratatui_core::buffer::Cell)> = cells
            .iter()
            .map(|(x, y, cell)| (*x, *y, crate::interop::to_ratatui_cell(cell)))
            .collect();
        let mut expected = Vec::<u8>::new();
        let mut reference = Reference::new(&mut expected);
        ratatui::backend::Backend::draw(
            &mut reference,
            foreign.iter().map(|(x, y, cell)| (*x, *y, cell)),
        )
        .expect("draw");

        assert_eq!(
            String::from_utf8_lossy(&ours.writer),
            String::from_utf8_lossy(&expected),
        );
    }

    #[cfg(feature = "scrolling-regions")]
    #[test]
    fn scrolling_a_region_sets_scrolls_and_resets_it() {
        use crossterm::Command;

        let mut out = String::new();
        ScrollInRegion::up(2..6, 3)
            .write_ansi(&mut out)
            .expect("up");
        assert_eq!(out, "\x1b[3;6r\x1b[3S\x1b[r");

        let mut out = String::new();
        ScrollInRegion::down(2..6, 1)
            .write_ansi(&mut out)
            .expect("down");
        assert_eq!(out, "\x1b[3;6r\x1b[1T\x1b[r");

        // A zero-line scroll must not leave a region set behind.
        let mut out = String::new();
        ScrollInRegion::up(2..6, 0)
            .write_ansi(&mut out)
            .expect("up");
        assert_eq!(out, "");
    }
}
