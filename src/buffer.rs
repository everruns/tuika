//! The cell grid: [`Cell`] and [`Buffer`].
//!
//! A [`Buffer`] is a rectangle of [`Cell`]s — one grapheme cluster plus its
//! attributes each — and is the only thing a [`View`](crate::View) ever writes
//! to, always through a clipped [`Surface`](crate::Surface).
//!
//! Rendering is immediate-mode: the whole frame is rebuilt every time. What
//! makes that affordable is [`Buffer::diff`], which compares the new frame
//! against the last one and yields only the cells that changed, so an unchanged
//! frame writes nothing to the terminal.
//!
//! # Wide graphemes
//!
//! A cell holds a *grapheme cluster*, not a `char`, and that cluster may be two
//! columns wide (CJK, most emoji). The convention — the same one every terminal
//! uses — is that the wide cluster lives in its leading cell and the cell to its
//! right is left empty as a placeholder. [`Buffer::set_string`] maintains that
//! invariant, and [`diff`](Buffer::diff) knows to step over the placeholder
//! rather than emitting a stray write into it.

use std::num::NonZeroU16;

use unicode_segmentation::UnicodeSegmentation;

use crate::geometry::{Position, Rect};
use crate::style::{Color, Modifier, Style};
use crate::width::grapheme_cols;

/// How [`Buffer::diff`] should treat one cell.
///
/// The default compares against the previous frame and writes on a difference.
/// The other variants exist because some things a terminal displays are not
/// under the cell grid's control — an image painted by a graphics protocol, or
/// an OSC 8 hyperlink whose escape bytes live in a cell's symbol but occupy no
/// columns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CellDiffOption {
    /// Compare against the previous frame; write when it differs.
    #[default]
    None,
    /// Never write this cell. Used where something outside the cell model owns
    /// the pixels, so a redraw would erase it.
    Skip,
    /// Always write this cell, even when unchanged. Used where another renderer
    /// may have painted over the same area, so the previous frame is not
    /// evidence of what is on screen.
    AlwaysUpdate,
    /// Treat the cell as exactly this many columns wide, whatever its symbol
    /// measures. This is what lets a symbol carry escape bytes — an OSC 8 link
    /// wrapper — without those bytes being counted as columns.
    ForcedWidth(NonZeroU16),
}

/// Bytes a [`Symbol`] stores without allocating.
///
/// Sized so the enum lands in the same 24 bytes a `String` would take, and so
/// the clusters that actually occur — a `char`, a combining sequence, a
/// flag or skin-tone emoji — all fit. Longer ones still work; they just spill
/// to the heap.
const SYMBOL_INLINE_CAP: usize = 22;

/// A cell's grapheme cluster, stored inline when it fits.
///
/// A plain `String` would put every one of a screen's cells on the heap — and
/// the grid is reallocated on every resize and cloned per scratch buffer, so
/// that showed up as a ~70% instruction-count regression on the render
/// benchmarks. Clusters are almost always a handful of bytes, so they live in
/// the cell itself.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Symbol {
    /// `len` valid UTF-8 bytes at the front of `bytes`.
    Inline {
        len: u8,
        bytes: [u8; SYMBOL_INLINE_CAP],
    },
    /// A cluster too long to inline. Rare enough not to matter.
    Heap(Box<str>),
}

impl Symbol {
    const EMPTY: Symbol = Symbol::Inline {
        len: 0,
        bytes: [0; SYMBOL_INLINE_CAP],
    };

    fn new(text: &str) -> Self {
        let bytes = text.as_bytes();
        if bytes.len() <= SYMBOL_INLINE_CAP {
            let mut buf = [0u8; SYMBOL_INLINE_CAP];
            buf[..bytes.len()].copy_from_slice(bytes);
            Symbol::Inline {
                len: bytes.len() as u8,
                bytes: buf,
            }
        } else {
            Symbol::Heap(text.into())
        }
    }

    fn as_str(&self) -> &str {
        match self {
            // Safe: only ever written from a `&str`, so the prefix is valid
            // UTF-8 by construction.
            Symbol::Inline { len, bytes } => {
                let bytes = &bytes[..*len as usize];
                debug_assert!(
                    std::str::from_utf8(bytes).is_ok(),
                    "inline symbol bytes must stay valid UTF-8"
                );
                // SAFETY: `Symbol` is private, and the only way to populate
                // `Inline` is `Symbol::new`, which copies a whole `&str`'s bytes
                // and records that exact length — so this slice is always a
                // complete, valid UTF-8 string. Nothing else writes `bytes`, and
                // `len` is never changed independently of it.
                //
                // Re-validating here instead costs about 10% of the render
                // benchmarks' instructions, because this runs for every cell on
                // every diff. The `debug_assert!` turns any future break of the
                // invariant into a test failure rather than undefined behavior.
                unsafe { std::str::from_utf8_unchecked(bytes) }
            }
            Symbol::Heap(text) => text,
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Symbol::Inline { len, .. } => *len == 0,
            Symbol::Heap(text) => text.is_empty(),
        }
    }
}

impl Default for Symbol {
    fn default() -> Self {
        Symbol::EMPTY
    }
}

/// One cell of the grid: a grapheme cluster and its attributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    /// The grapheme cluster to draw. Empty means this is the placeholder to the
    /// right of a wide cluster.
    symbol: Symbol,
    /// Foreground color.
    pub fg: Color,
    /// Background color.
    pub bg: Color,
    /// Underline color, honored only by terminals implementing the extended
    /// underline SGR.
    pub underline_color: Color,
    /// Attributes.
    pub modifier: Modifier,
    /// How the diff should treat this cell.
    pub diff_option: CellDiffOption,
}

impl Cell {
    /// A cell showing `symbol`, unstyled.
    ///
    /// Not `const`: a cell can hold any grapheme
    /// cluster, and a cluster is not known at compile time. Use
    /// [`Cell::default`] for the blank cell, which is the only one a `const`
    /// would have bought.
    pub fn new(symbol: &str) -> Self {
        let mut cell = Self::default();
        cell.set_symbol(symbol);
        cell
    }

    /// The grapheme cluster, or `" "` when this cell is blank.
    pub fn symbol(&self) -> &str {
        if self.symbol.is_empty() {
            " "
        } else {
            self.symbol.as_str()
        }
    }

    /// The symbol exactly as stored — empty when this cell is the placeholder
    /// to the right of a wide grapheme.
    ///
    /// [`symbol`](Self::symbol) reports `" "` for both a blank cell and a
    /// placeholder; this is how a caller tells them apart.
    pub fn raw_symbol(&self) -> &str {
        self.symbol.as_str()
    }

    /// Set the grapheme cluster.
    pub fn set_symbol(&mut self, symbol: &str) -> &mut Self {
        self.symbol = Symbol::new(symbol);
        self
    }

    /// Set the cell to a single character.
    pub fn set_char(&mut self, ch: char) -> &mut Self {
        let mut buf = [0u8; 4];
        self.set_symbol(ch.encode_utf8(&mut buf));
        self
    }

    /// Set the foreground color.
    pub const fn set_fg(&mut self, color: Color) -> &mut Self {
        self.fg = color;
        self
    }

    /// Set the background color.
    pub const fn set_bg(&mut self, color: Color) -> &mut Self {
        self.bg = color;
        self
    }

    /// Apply `style`, leaving fields the style does not set untouched.
    pub fn set_style<S: Into<Style>>(&mut self, style: S) -> &mut Self {
        let style = style.into();
        if let Some(fg) = style.fg {
            self.fg = fg;
        }
        if let Some(bg) = style.bg {
            self.bg = bg;
        }
        if let Some(underline) = style.underline_color {
            self.underline_color = underline;
        }
        self.modifier = self
            .modifier
            .union(style.add_modifier)
            .difference(style.sub_modifier);
        self
    }

    /// This cell's attributes as a [`Style`].
    pub const fn style(&self) -> Style {
        Style {
            fg: Some(self.fg),
            bg: Some(self.bg),
            underline_color: Some(self.underline_color),
            add_modifier: self.modifier,
            sub_modifier: Modifier::empty(),
        }
    }

    /// Set how the diff treats this cell.
    pub const fn set_diff_option(&mut self, diff_option: CellDiffOption) -> &mut Self {
        self.diff_option = diff_option;
        self
    }

    /// Reset to blank and unstyled.
    pub fn reset(&mut self) {
        self.symbol = Symbol::new(" ");
        self.fg = Color::Reset;
        self.bg = Color::Reset;
        self.underline_color = Color::Reset;
        self.modifier = Modifier::empty();
        self.diff_option = CellDiffOption::None;
    }

    /// Display width in columns, honoring [`CellDiffOption::ForcedWidth`].
    pub fn cell_width(&self) -> u16 {
        match self.diff_option {
            CellDiffOption::ForcedWidth(width) => width.get(),
            _ => grapheme_cols(self.symbol()),
        }
    }

    /// Whether the diff must leave this cell alone.
    const fn is_skipped(&self) -> bool {
        matches!(self.diff_option, CellDiffOption::Skip)
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            symbol: Symbol::new(" "),
            fg: Color::Reset,
            bg: Color::Reset,
            underline_color: Color::Reset,
            modifier: Modifier::empty(),
            diff_option: CellDiffOption::None,
        }
    }
}

/// A rectangle of cells.
///
/// The area's `x`/`y` are the buffer's position on screen, so a buffer covering
/// an inline viewport reports its real screen rows. Indexing is by absolute
/// coordinate, not by offset into the rect.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Buffer {
    /// The region this buffer covers, in screen coordinates.
    pub area: Rect,
    /// Cells in row-major order, `area.area()` of them.
    pub content: Vec<Cell>,
}

impl Buffer {
    /// A buffer of blank cells covering `area`.
    pub fn empty(area: Rect) -> Self {
        Self::filled(area, Cell::default())
    }

    /// A buffer covering `area` with every cell a clone of `cell`.
    pub fn filled(area: Rect, cell: Cell) -> Self {
        let size = area.area() as usize;
        Self {
            area,
            content: vec![cell; size],
        }
    }

    /// A buffer built from one string per row, unstyled.
    ///
    /// Chiefly a test convenience — it is how the golden-snapshot suite states
    /// an expected screen.
    pub fn with_lines<'a, I>(lines: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<crate::text::Line<'a>>,
    {
        let lines: Vec<crate::text::Line<'a>> = lines.into_iter().map(Into::into).collect();
        let height = lines.len() as u16;
        let width = lines
            .iter()
            .map(crate::text::Line::width)
            .max()
            .unwrap_or(0);
        let mut buffer = Self::empty(Rect::new(0, 0, width, height));
        for (y, line) in lines.iter().enumerate() {
            buffer.set_line(0, y as u16, line, width);
        }
        buffer
    }

    /// Flat index of a screen position, or `None` when outside the area.
    fn index_of(&self, x: u16, y: u16) -> Option<usize> {
        if !self.area.contains(Position { x, y }) {
            return None;
        }
        let dx = (x - self.area.x) as usize;
        let dy = (y - self.area.y) as usize;
        Some(dy * self.area.width as usize + dx)
    }

    /// The screen position of a flat index.
    fn pos_of(&self, index: usize) -> (u16, u16) {
        let width = self.area.width.max(1) as usize;
        let x = index % width + self.area.x as usize;
        let y = index / width + self.area.y as usize;
        (x as u16, y as u16)
    }

    /// The cell at `position`, or `None` when outside the area.
    pub fn cell<P: Into<Position>>(&self, position: P) -> Option<&Cell> {
        let position = position.into();
        self.index_of(position.x, position.y)
            .map(|index| &self.content[index])
    }

    /// Mutable access to the cell at `position`, or `None` when outside.
    pub fn cell_mut<P: Into<Position>>(&mut self, position: P) -> Option<&mut Cell> {
        let position = position.into();
        self.index_of(position.x, position.y)
            .map(|index| &mut self.content[index])
    }

    /// The cells, row-major. Same data as the [`content`](Self::content)
    /// field; the accessor exists so a caller can chain off it.
    pub fn content(&self) -> &[Cell] {
        &self.content
    }

    /// Reset every cell to blank and unstyled.
    pub fn reset(&mut self) {
        for cell in &mut self.content {
            cell.reset();
        }
    }

    /// Resize to `area`, keeping as many cells as still fit.
    ///
    /// Deliberately not a reflow — cells are kept by flat index, so a width
    /// change shifts rows — because the frame is rebuilt from application state
    /// anyway and the diff repaints whatever moved.
    ///
    /// It must not *clear*, though: resizing to the same area is a no-op that
    /// several call sites rely on, and a
    /// [`Terminal`](crate::term::terminal::Terminal) skips its repaint when the
    /// dimensions are unchanged. Wiping here would blank the screen with nothing
    /// to restore it — which is exactly what the split-footer stress sweep
    /// caught.
    pub fn resize(&mut self, area: Rect) {
        let size = area.area() as usize;
        if self.content.len() > size {
            self.content.truncate(size);
        } else {
            self.content.resize(size, Cell::default());
        }
        self.area = area;
    }

    /// Overlay `other` onto this buffer, growing the area to cover both.
    ///
    /// Cells from `other` win where they overlap. Used by
    /// [`Surface::render_ratatui`](crate::Surface::render_ratatui) when a
    /// callback hands back a buffer larger than the one it was given.
    pub fn merge(&mut self, other: &Self) {
        let merged = self.area.union(other.area);
        let mut content = vec![Cell::default(); merged.area() as usize];
        let width = merged.width as usize;

        let mut place = |buffer: &Self| {
            for (index, cell) in buffer.content.iter().enumerate() {
                let (x, y) = buffer.pos_of(index);
                if !merged.contains(Position { x, y }) {
                    continue;
                }
                let dx = (x - merged.x) as usize;
                let dy = (y - merged.y) as usize;
                content[dy * width + dx] = cell.clone();
            }
        };
        place(self);
        place(other);

        self.area = merged;
        self.content = content;
    }

    /// Apply `style` to every cell in `area`, leaving symbols untouched.
    ///
    /// Only the part of `area` inside this buffer is touched, so a caller may
    /// pass a rect larger than the frame.
    pub fn set_style<S: Into<Style>>(&mut self, area: Rect, style: S) {
        let style = style.into();
        let target = area.intersection(self.area);
        for y in target.y..target.bottom() {
            for x in target.x..target.right() {
                if let Some(cell) = self.cell_mut((x, y)) {
                    cell.set_style(style);
                }
            }
        }
    }

    /// Write `string` starting at `x`, `y`, clipped to `max_width` columns and
    /// to the buffer's own area. Returns the column just past what was written.
    pub fn set_string<S: Into<Style>>(&mut self, x: u16, y: u16, string: &str, style: S) -> u16 {
        self.set_stringn(x, y, string, usize::MAX, style).0
    }

    /// Like [`set_string`](Self::set_string) but stopping after `max_width`
    /// columns. Returns the next free column and the columns consumed.
    pub fn set_stringn<S: Into<Style>>(
        &mut self,
        x: u16,
        y: u16,
        string: &str,
        max_width: usize,
        style: S,
    ) -> (u16, u16) {
        let style = style.into();
        let mut column = x;
        let mut used = 0u16;
        let limit = max_width.min(u16::MAX as usize) as u16;

        for grapheme in string.graphemes(true) {
            let width = grapheme_cols(grapheme);
            // A zero-width cluster (a lone combining mark) has no cell of its
            // own; dropping it is what keeps the column count honest.
            if width == 0 {
                continue;
            }
            if used.saturating_add(width) > limit || column >= self.area.right() {
                break;
            }
            let Some(index) = self.index_of(column, y) else {
                break;
            };
            self.content[index].set_symbol(grapheme);
            self.content[index].set_style(style);
            // Blank out the placeholder columns a wide cluster occupies, so a
            // narrower glyph previously there cannot show through.
            for offset in 1..width {
                if let Some(trailing) = self.index_of(column + offset, y) {
                    self.content[trailing].set_symbol("");
                    self.content[trailing].set_style(style);
                }
            }
            column = column.saturating_add(width);
            used = used.saturating_add(width);
        }
        (column, used)
    }

    /// Write a [`Span`](crate::text::Span), returning the next free column and
    /// the columns consumed.
    pub fn set_span(
        &mut self,
        x: u16,
        y: u16,
        span: &crate::text::Span<'_>,
        max_width: u16,
    ) -> (u16, u16) {
        self.set_stringn(x, y, span.content.as_ref(), max_width as usize, span.style)
    }

    /// Write a [`Line`](crate::text::Line), each span styled over the line's own
    /// style. Returns the next free column and the columns consumed.
    pub fn set_line(
        &mut self,
        x: u16,
        y: u16,
        line: &crate::text::Line<'_>,
        max_width: u16,
    ) -> (u16, u16) {
        let mut column = x;
        let mut used = 0u16;
        for span in &line.spans {
            let remaining = max_width.saturating_sub(used);
            if remaining == 0 {
                break;
            }
            let (next, consumed) = self.set_stringn(
                column,
                y,
                span.content.as_ref(),
                remaining as usize,
                line.style.patch(span.style),
            );
            column = next;
            used = used.saturating_add(consumed);
        }
        (column, used)
    }

    /// The cells that changed between `self` (the previous frame) and `next`.
    ///
    /// Yields `(x, y, &Cell)` in row-major order. Both buffers must share the
    /// same origin and width; height may differ, in which case the shorter one
    /// bounds the walk.
    ///
    /// # Panics
    ///
    /// Panics when the origins or widths differ — that is a caller bug, not a
    /// recoverable state, and silently diffing mismatched grids would corrupt
    /// the screen.
    pub fn diff<'next>(&self, next: &'next Self) -> Vec<(u16, u16, &'next Cell)> {
        assert!(
            self.area.x == next.area.x
                && self.area.y == next.area.y
                && self.area.width == next.area.width,
            "diffed buffers must share an origin and width: prev={:?}, next={:?}",
            self.area,
            next.area,
        );

        let len = self.content.len().min(next.content.len());
        let mut updates = Vec::new();
        let mut index = 0usize;

        while index < len {
            let current = &next.content[index];
            let previous = &self.content[index];
            let position = index;
            index += 1;

            if current.is_skipped() {
                continue;
            }

            match current.diff_option {
                CellDiffOption::Skip => {}
                CellDiffOption::ForcedWidth(width) => {
                    // The symbol carries escape bytes that occupy `width`
                    // columns on screen; step over the columns they cover.
                    index = index.saturating_add(width.get().saturating_sub(1) as usize);
                    if current != previous {
                        let (x, y) = next.pos_of(position);
                        updates.push((x, y, current));
                    }
                }
                CellDiffOption::None | CellDiffOption::AlwaysUpdate => {
                    let width = current.cell_width() as usize;
                    let unchanged =
                        matches!(current.diff_option, CellDiffOption::None) && current == previous;
                    if unchanged {
                        index += width.saturating_sub(1);
                        continue;
                    }

                    let previous_width = previous.cell_width() as usize;
                    let (x, y) = next.pos_of(position);
                    updates.push((x, y, current));

                    if width > 1 {
                        // The wide cluster covers its placeholder column; the
                        // terminal paints both from the one write.
                        index += width - 1;
                    } else if previous_width > width
                        && (previous.bg != Color::Reset
                            || previous.modifier.intersection(VISIBLE_ON_BLANK)
                                != Modifier::empty())
                    {
                        // A wide cluster with visible background or decoration
                        // was replaced by a narrow one. Some terminals leave the
                        // old styling on the freed column, so refresh every cell
                        // it used to cover whether or not the buffer changed.
                        let end = (position + previous_width).min(len);
                        for trailing in (position + 1)..end {
                            if !next.content[trailing].is_skipped() {
                                let (tx, ty) = next.pos_of(trailing);
                                updates.push((tx, ty, &next.content[trailing]));
                            }
                        }
                        index = end;
                    }
                }
            }
        }

        updates
    }
}

/// Modifiers that remain visible on a blank cell, so a stale one must be
/// actively cleared rather than left to be overwritten by a space.
const VISIBLE_ON_BLANK: Modifier = Modifier::REVERSED
    .union(Modifier::UNDERLINED)
    .union(Modifier::SLOW_BLINK)
    .union(Modifier::RAPID_BLINK)
    .union(Modifier::CROSSED_OUT);

impl<P: Into<Position>> std::ops::Index<P> for Buffer {
    type Output = Cell;

    #[track_caller]
    fn index(&self, position: P) -> &Self::Output {
        let position = position.into();
        self.cell(position)
            .expect("position out of the buffer's area")
    }
}

impl<P: Into<Position>> std::ops::IndexMut<P> for Buffer {
    #[track_caller]
    fn index_mut(&mut self, position: P) -> &mut Self::Output {
        let area = self.area;
        self.cell_mut(position).unwrap_or_else(|| {
            panic!("position out of the buffer's area {area:?}");
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(buffer: &Buffer, y: u16) -> String {
        (buffer.area.x..buffer.area.right())
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect()
    }

    #[test]
    fn empty_cells_render_as_spaces() {
        let buffer = Buffer::empty(Rect::new(0, 0, 3, 1));
        assert_eq!(row(&buffer, 0), "   ");
    }

    #[test]
    fn set_string_writes_and_returns_the_next_column() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 6, 1));
        let next = buffer.set_string(1, 0, "abc", Style::default());
        assert_eq!(next, 4);
        assert_eq!(row(&buffer, 0), " abc  ");
    }

    /// The wide-grapheme invariant: the cluster sits in its leading cell and the
    /// next cell is an empty placeholder, never a duplicate.
    #[test]
    fn a_wide_grapheme_blanks_its_trailing_cell() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
        buffer.set_string(0, 0, "日本", Style::default());
        assert_eq!(buffer[(0, 0)].symbol(), "日");
        assert_eq!(buffer[(1, 0)].raw_symbol(), "");
        assert_eq!(buffer[(2, 0)].symbol(), "本");
        assert_eq!(buffer[(3, 0)].raw_symbol(), "");
    }

    /// Half a wide grapheme must not be written: it would desynchronize every
    /// column after it.
    #[test]
    fn a_wide_grapheme_that_does_not_fit_is_dropped() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 1));
        let (next, used) = buffer.set_stringn(0, 0, "a日", 2, Style::default());
        assert_eq!(used, 1, "only the narrow grapheme fits in two columns");
        assert_eq!(next, 1);
        assert_eq!(buffer[(1, 0)].symbol(), " ");
    }

    #[test]
    fn set_string_clips_at_the_area_edge() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 1));
        buffer.set_string(1, 0, "abcdef", Style::default());
        assert_eq!(row(&buffer, 0), " ab");
    }

    #[test]
    fn zero_width_clusters_consume_no_columns() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
        let (next, used) = buffer.set_stringn(0, 0, "a\u{0301}b", 4, Style::default());
        assert_eq!(used, 2, "the combining acute joins its base grapheme");
        assert_eq!(next, 2);
    }

    /// The inline/heap split must be invisible: a cluster longer than the
    /// inline capacity still round-trips exactly.
    #[test]
    fn a_symbol_too_long_to_inline_spills_to_the_heap() {
        let long = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
        assert!(
            long.len() > SYMBOL_INLINE_CAP,
            "corpus must exceed the inline cap"
        );
        let mut cell = Cell::default();
        cell.set_symbol(long);
        assert_eq!(cell.symbol(), long);
        assert!(matches!(cell.symbol, Symbol::Heap(_)));
    }

    #[test]
    fn a_short_symbol_stays_inline() {
        let mut cell = Cell::default();
        cell.set_symbol("\u{1F600}");
        assert_eq!(cell.symbol(), "\u{1F600}");
        assert!(matches!(cell.symbol, Symbol::Inline { .. }));
    }

    /// Overwriting a heap symbol with a short one must return to inline storage,
    /// or a cell would keep an allocation it no longer needs.
    #[test]
    fn overwriting_a_heap_symbol_returns_to_inline() {
        let long = "x".repeat(SYMBOL_INLINE_CAP + 8);
        let mut cell = Cell::default();
        cell.set_symbol(&long);
        cell.set_symbol("a");
        assert_eq!(cell.symbol(), "a");
        assert!(matches!(cell.symbol, Symbol::Inline { .. }));
    }

    #[test]
    fn indexing_uses_absolute_coordinates() {
        let mut buffer = Buffer::empty(Rect::new(5, 5, 2, 2));
        buffer[(5, 5)].set_char('x');
        assert_eq!(buffer[(5, 5)].symbol(), "x");
        assert!(buffer.cell((0, 0)).is_none(), "outside the area");
    }

    #[test]
    fn an_unchanged_frame_diffs_to_nothing() {
        let buffer = Buffer::empty(Rect::new(0, 0, 4, 2));
        assert!(buffer.diff(&buffer.clone()).is_empty());
    }

    #[test]
    fn diff_yields_only_changed_cells() {
        let previous = Buffer::empty(Rect::new(0, 0, 4, 1));
        let mut next = previous.clone();
        next[(2, 0)].set_char('x');
        let updates = previous.diff(&next);
        assert_eq!(updates.len(), 1);
        assert_eq!((updates[0].0, updates[0].1), (2, 0));
    }

    /// A skipped cell is owned by something outside the grid (an image), so the
    /// diff must never write over it even when the buffer says it changed.
    #[test]
    fn a_skipped_cell_is_never_emitted() {
        let previous = Buffer::empty(Rect::new(0, 0, 3, 1));
        let mut next = previous.clone();
        next[(1, 0)].set_char('x');
        next[(1, 0)].set_diff_option(CellDiffOption::Skip);
        assert!(previous.diff(&next).is_empty());
    }

    #[test]
    fn always_update_emits_an_unchanged_cell() {
        let previous = Buffer::empty(Rect::new(0, 0, 2, 1));
        let mut next = previous.clone();
        next[(0, 0)].set_diff_option(CellDiffOption::AlwaysUpdate);
        let updates = previous.diff(&next);
        assert_eq!(updates.len(), 1);
    }

    /// The OSC 8 case: a symbol full of escape bytes that must be billed one
    /// column, not the width of the escape text.
    #[test]
    fn forced_width_bills_the_declared_columns() {
        let previous = Buffer::empty(Rect::new(0, 0, 3, 1));
        let mut next = previous.clone();
        next[(0, 0)].set_symbol("\u{1b}]8;;http://x\u{1b}\\a");
        next[(0, 0)].set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16::new(1).unwrap()));
        let updates = previous.diff(&next);
        assert_eq!(updates.len(), 1);
        assert_eq!(next[(0, 0)].cell_width(), 1);
    }

    /// Replacing a styled wide grapheme with a narrow one has to refresh the
    /// column it vacated, or the old background lingers there.
    #[test]
    fn a_narrowed_wide_cell_refreshes_its_vacated_column() {
        let mut previous = Buffer::empty(Rect::new(0, 0, 3, 1));
        previous[(0, 0)].set_symbol("日");
        previous[(0, 0)].set_bg(Color::Blue);
        let mut next = Buffer::empty(Rect::new(0, 0, 3, 1));
        next[(0, 0)].set_char('a');

        let updates = previous.diff(&next);
        let columns: Vec<u16> = updates.iter().map(|(x, _, _)| *x).collect();
        assert!(columns.contains(&0));
        assert!(
            columns.contains(&1),
            "the freed column must be refreshed, got {columns:?}"
        );
    }

    /// The same replacement with no visible styling needs no extra write —
    /// otherwise every frame would bloat with redundant updates.
    #[test]
    fn a_narrowed_unstyled_wide_cell_needs_no_extra_write() {
        let mut previous = Buffer::empty(Rect::new(0, 0, 3, 1));
        previous[(0, 0)].set_symbol("日");
        let mut next = Buffer::empty(Rect::new(0, 0, 3, 1));
        next[(0, 0)].set_char('a');

        let updates = previous.diff(&next);
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, 0);
    }

    #[test]
    fn set_style_repaints_an_area_without_touching_symbols() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 2));
        buffer.set_string(0, 0, "ab", Style::default());
        buffer.set_style(Rect::new(0, 0, 2, 1), Style::new().bg(Color::Blue));
        assert_eq!(buffer[(0, 0)].symbol(), "a");
        assert_eq!(buffer[(0, 0)].bg, Color::Blue);
        assert_eq!(buffer[(2, 0)].bg, Color::Reset, "outside the area");
    }

    #[test]
    fn set_style_clips_an_oversized_area() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 2, 1));
        buffer.set_style(Rect::new(0, 0, 99, 99), Style::new().bg(Color::Blue));
        assert_eq!(buffer[(1, 0)].bg, Color::Blue);
    }

    #[test]
    fn resize_changes_the_area_and_cell_count() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 2, 1));
        buffer.resize(Rect::new(0, 0, 3, 2));
        assert_eq!(buffer.area, Rect::new(0, 0, 3, 2));
        assert_eq!(buffer.content.len(), 6);
    }

    /// Resizing to the same area must not touch a single cell. A
    /// [`Terminal`](crate::term::terminal::Terminal) skips its repaint when the
    /// dimensions have not changed, so clearing here would blank the screen
    /// with nothing left to restore it.
    #[test]
    fn resizing_to_the_same_area_preserves_every_cell() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 2));
        buffer.set_string(0, 0, "abc", Style::default());
        buffer.resize(Rect::new(0, 0, 3, 2));
        assert_eq!(buffer[(0, 0)].symbol(), "a");
        assert_eq!(buffer[(2, 0)].symbol(), "c");
    }

    #[test]
    fn growing_keeps_existing_cells_and_blanks_the_rest() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 2, 1));
        buffer.set_string(0, 0, "ab", Style::default());
        buffer.resize(Rect::new(0, 0, 2, 2));
        assert_eq!(buffer[(0, 0)].symbol(), "a");
        assert_eq!(buffer[(1, 0)].symbol(), "b");
        assert_eq!(buffer[(0, 1)].symbol(), " ");
    }

    #[test]
    fn shrinking_truncates_rather_than_clearing() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 2, 2));
        buffer.set_string(0, 0, "ab", Style::default());
        buffer.resize(Rect::new(0, 0, 2, 1));
        assert_eq!(buffer.content.len(), 2);
        assert_eq!(buffer[(0, 0)].symbol(), "a");
    }

    #[test]
    fn merge_grows_the_area_and_lets_the_other_win() {
        let mut base = Buffer::empty(Rect::new(0, 0, 2, 1));
        base[(0, 0)].set_char('a');
        let mut overlay = Buffer::empty(Rect::new(1, 0, 2, 1));
        overlay[(1, 0)].set_char('b');

        base.merge(&overlay);
        assert_eq!(base.area, Rect::new(0, 0, 3, 1));
        assert_eq!(base[(0, 0)].symbol(), "a");
        assert_eq!(base[(1, 0)].symbol(), "b");
    }

    #[test]
    fn set_style_leaves_unset_fields_alone() {
        let mut cell = Cell::new("x");
        cell.set_bg(Color::Blue);
        cell.set_style(Style::new().fg(Color::Red));
        assert_eq!(cell.fg, Color::Red);
        assert_eq!(cell.bg, Color::Blue);
    }

    #[test]
    fn with_lines_builds_a_grid_sized_to_the_widest_row() {
        let buffer = Buffer::with_lines(["ab", "cdef"]);
        assert_eq!(buffer.area, Rect::new(0, 0, 4, 2));
        assert_eq!(row(&buffer, 0), "ab  ");
        assert_eq!(row(&buffer, 1), "cdef");
    }
}
