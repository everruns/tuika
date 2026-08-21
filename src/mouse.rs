//! Mouse interaction over the rendered cell grid: text selection, click
//! hit-testing, and click detection.
//!
//! Terminals don't hand captured applications selection or clickable regions —
//! once an app opts into mouse capture (see
//! [`AltScreen::enter_with_mouse_capture`](crate::host::AltScreen::enter_with_mouse_capture))
//! the terminal's own link activation and click-drag selection stop working, and every drag arrives as a
//! [`Mouse`] event instead. This module rebuilds those affordances *on top of*
//! the grid the app already rendered:
//!
//! - [`SelectionState`] turns a left button `Down -> Drag -> Up` gesture into a
//!   [`SelectionRange`]; [`selected_text`] reads the text back out of the
//!   [`Buffer`] and [`paint_selection`] paints the selection into it. Pair with
//!   [`crate::term::clipboard::write`] to copy.
//! - [`HitMap`] maps screen regions to values so a click resolves to whatever
//!   was drawn there (a button, a link, a list row); [`ClickTracker`] turns a
//!   `Down`/`Up` pair on the same cell into a click (and lets a drag cancel it).
//!
//! Selection is *linear* (stream) like a terminal's own: a multi-row selection
//! covers the tail of the first row, all of the middle rows, and the head of
//! the last row — not a rectangular block.
//!
//! **Touch:** terminal emulators deliver touch as mouse events (a tap is a
//! `Down`+`Up`, a swipe is `ScrollUp`/`ScrollDown` or a `Drag`), so touch input
//! flows through this same path — there is no separate touch event to model.

use crate::buffer::Buffer;
use crate::geometry::{Position, Rect};
use crate::style::Style;
use std::time::{Duration, Instant};

use crate::event::{Mouse, MouseButton, MouseKind};
use crate::view::SelectionSource;
use crate::{Clock, SystemClock};

/// A normalized text selection in reading order: `start` is at or before `end`
/// ordered by `(row, column)`. Both endpoints are inclusive cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionRange {
    /// `(column, row)` of the first selected cell.
    pub start: (u16, u16),
    /// `(column, row)` of the last selected cell (inclusive).
    pub end: (u16, u16),
}

impl SelectionRange {
    /// Order two cells into reading order (`(row, col)` ascending).
    fn between(a: (u16, u16), b: (u16, u16)) -> Self {
        let (start, end) = if (a.1, a.0) <= (b.1, b.0) {
            (a, b)
        } else {
            (b, a)
        };
        SelectionRange { start, end }
    }

    /// The inclusive `[left, right]` selected column span on `row`, clamped to
    /// `area`. Rows outside the selection — or outside `area`, so a range that
    /// outlived the geometry it was made in can never reach a cell the caller
    /// did not offer — return `None`.
    fn row_span(&self, row: u16, area: Rect) -> Option<(u16, u16)> {
        if row < self.start.1 || row > self.end.1 || row < area.y || row >= area.bottom() {
            return None;
        }
        let left = if row == self.start.1 {
            self.start.0
        } else {
            area.x
        };
        let right = if row == self.end.1 {
            self.end.0
        } else {
            area.right().saturating_sub(1)
        };
        let left = left.max(area.x);
        let right = right.min(area.right().saturating_sub(1));
        (left <= right).then_some((left, right))
    }

    /// Whether `(column, row)` falls inside the selection within `area`.
    pub fn contains(&self, column: u16, row: u16, area: Rect) -> bool {
        self.row_span(row, area)
            .is_some_and(|(l, r)| column >= l && column <= r)
    }
}

/// Tracks click-drag and double-click text selection across mouse events.
///
/// Left `Down` starts (and clears any previous selection); `Drag` extends;
/// `Up` finishes. [`confine`](Self::confine) restricts a gesture to one panel:
/// positions are clamped into that rect, so dragging out of the panel selects
/// to its edge rather than across whatever is beside it. Two plain clicks on
/// the same cell within the double-click interval queue a word selection. Call [`Self::resolve`] after rendering so
/// word boundaries can be read from the current buffer.
#[derive(Clone, Copy, Debug, Default)]
pub struct SelectionState {
    anchor: (u16, u16),
    cursor: (u16, u16),
    pressed: bool,
    selecting: bool,
    last_click: Option<((u16, u16), Instant)>,
    pending_word: Option<(u16, u16)>,
    region: Option<Rect>,
}

impl SelectionState {
    /// A state with no selection in progress.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a mouse event; returns `true` when the selection changed and a
    /// redraw is warranted.
    pub fn handle(&mut self, m: &Mouse) -> bool {
        self.handle_with_clock(m, &SystemClock)
    }

    /// Feed a mouse event using an explicit monotonic `clock`.
    ///
    /// This is behaviorally identical to [`handle`](Self::handle), but makes
    /// double-click timing deterministic in tests, replay systems, and hosts
    /// with a virtual clock.
    pub fn handle_with_clock(&mut self, m: &Mouse, clock: &dyn Clock) -> bool {
        let (column, row) = clamp_to_region(self.region, m.column, m.row);
        match m.kind {
            MouseKind::Down(MouseButton::Left) => {
                self.anchor = (column, row);
                self.cursor = (column, row);
                self.pressed = true;
                let had = self.selecting;
                self.selecting = false;
                // Redraw if we cleared an existing selection.
                had
            }
            MouseKind::Drag(MouseButton::Left) if self.pressed => {
                self.cursor = (column, row);
                self.selecting = self.cursor != self.anchor;
                self.last_click = None;
                self.pending_word = None;
                true
            }
            MouseKind::Up(MouseButton::Left) if self.pressed => {
                self.cursor = (column, row);
                self.pressed = false;
                self.selecting = self.cursor != self.anchor;
                if self.selecting {
                    self.last_click = None;
                } else {
                    let position = self.cursor;
                    let now = clock.now();
                    let is_double = self.last_click.is_some_and(|(previous, at)| {
                        previous == position
                            && now.saturating_duration_since(at) <= DOUBLE_CLICK_INTERVAL
                    });
                    self.last_click = Some((position, now));
                    self.pending_word = is_double.then_some(position);
                }
                true
            }
            _ => false,
        }
    }

    /// Resolve a pending double-click against the freshly rendered buffer.
    ///
    /// Returns `true` when a word was selected. Word characters are Unicode
    /// alphanumerics plus `_`; punctuation is selected as a contiguous run.
    pub fn resolve(&mut self, buffer: &Buffer, area: Rect) -> bool {
        let Some((column, row)) = self.pending_word.take() else {
            return false;
        };
        let Some(range) = word_at(buffer, area, column, row) else {
            return false;
        };
        self.anchor = range.start;
        self.cursor = range.end;
        self.selecting = true;
        true
    }

    /// Confine the gesture to `region` — a panel, list viewport, or any other
    /// rect the host considers one selectable surface.
    ///
    /// Set it on the `Down` that starts a gesture: every position is clamped
    /// into the rect, and [`region`](Self::region) reports it back so painting
    /// and text extraction use the same bounds (pass it as their `area`, so a
    /// multi-row selection wraps at the panel edge). `None` restores whole-screen
    /// selection.
    pub fn confine(&mut self, region: Option<Rect>) {
        self.region = region.filter(|r| r.width > 0 && r.height > 0);
    }

    /// The panel the current gesture is confined to, if any.
    pub fn region(&self) -> Option<Rect> {
        self.region
    }

    /// Clear the selection (e.g. on Esc or a new turn).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The current selection, or `None` when nothing is selected.
    pub fn range(&self) -> Option<SelectionRange> {
        self.selecting
            .then(|| SelectionRange::between(self.anchor, self.cursor))
    }

    /// Whether a non-empty selection currently exists.
    pub fn is_active(&self) -> bool {
        self.selecting
    }
}

/// Clamp a cell into `region` (a no-op when the gesture is unconfined).
fn clamp_to_region(region: Option<Rect>, column: u16, row: u16) -> (u16, u16) {
    let Some(r) = region else {
        return (column, row);
    };
    (
        column.clamp(r.x, r.right().saturating_sub(1)),
        row.clamp(r.y, r.bottom().saturating_sub(1)),
    )
}

/// The innermost region of `regions` containing `(column, row)`.
///
/// Regions are recorded parent-before-child while rendering, so the last match
/// is the innermost panel at that cell.
pub(crate) fn region_at(regions: &[Rect], column: u16, row: u16) -> Option<Rect> {
    regions
        .iter()
        .rev()
        .find(|r| in_rect(**r, column, row))
        .copied()
}

const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, PartialEq, Eq)]
enum CellClass {
    Word,
    Punctuation,
    Blank,
}

fn cell_class(buffer: &Buffer, column: u16, row: u16) -> CellClass {
    let symbol = buffer[(column, row)].symbol();
    let Some(ch) = symbol.chars().next() else {
        return CellClass::Blank;
    };
    if ch.is_whitespace() {
        CellClass::Blank
    } else if ch.is_alphanumeric() || ch == '_' {
        CellClass::Word
    } else {
        CellClass::Punctuation
    }
}

/// The word (or contiguous punctuation run) under `(column, row)` in `buffer`,
/// as an inclusive [`SelectionRange`] on that row. Word characters are Unicode
/// alphanumerics plus `_`. Returns `None` over blank cells or outside `area`.
pub fn word_at(buffer: &Buffer, area: Rect, column: u16, row: u16) -> Option<SelectionRange> {
    if !in_rect(area, column, row) {
        return None;
    }
    let class = cell_class(buffer, column, row);
    if class == CellClass::Blank {
        return None;
    }
    let mut left = column;
    while left > area.x && cell_class(buffer, left - 1, row) == class {
        left -= 1;
    }
    let mut right = column;
    while right + 1 < area.right() && cell_class(buffer, right + 1, row) == class {
        right += 1;
    }
    Some(SelectionRange::between((left, row), (right, row)))
}

/// Extract the selected text from `buffer`, reading the grid the way a terminal
/// does: the tail of the first row, whole middle rows, and the head of the last
/// row, joined with `\n`. Trailing blanks on each row are trimmed. Wide glyphs
/// come back once (their trailing spacer cell is empty in the buffer).
pub fn selected_text(buffer: &Buffer, area: Rect, sel: SelectionRange) -> String {
    selected_text_from_sources(buffer, area, sel, &[])
}

pub(crate) fn selected_text_from_sources(
    buffer: &Buffer,
    area: Rect,
    sel: SelectionRange,
    sources: &[SelectionSource],
) -> String {
    if let Some(source) = sources.iter().rev().find(|source| {
        source
            .area
            .contains(Position::new(sel.start.0, sel.start.1))
            && source.area.contains(Position::new(sel.end.0, sel.end.1))
            && source_matches_selection(buffer, area, sel, &source.text)
    }) && let Some(text) = extract_source_selection(buffer, area, sel, &source.text)
    {
        return text;
    }
    selected_grid_text(buffer, area, sel)
}

fn selected_grid_text(buffer: &Buffer, area: Rect, sel: SelectionRange) -> String {
    let mut out = String::new();
    let mut first = true;
    for row in sel.start.1..=sel.end.1 {
        let Some((left, right)) = sel.row_span(row, area) else {
            continue;
        };
        if !first {
            out.push('\n');
        }
        first = false;
        let mut line = String::new();
        for col in left..=right {
            line.push_str(buffer[(col, row)].symbol());
        }
        out.push_str(line.trim_end_matches(' '));
    }
    out
}

fn source_matches_selection(
    buffer: &Buffer,
    area: Rect,
    sel: SelectionRange,
    source: &str,
) -> bool {
    selected_grid_text(buffer, area, sel)
        .lines()
        .filter(|line| !line.is_empty())
        .all(|line| source.contains(line.trim()))
}

fn extract_source_selection(
    buffer: &Buffer,
    area: Rect,
    sel: SelectionRange,
    source: &str,
) -> Option<String> {
    let selected = selected_grid_text(buffer, area, sel);
    let first = selected.lines().next()?.trim_start();
    let last = selected.lines().last()?.trim_end();
    if first.is_empty() || last.is_empty() {
        return None;
    }
    let start = source.find(first)?;
    let end = source[start..].find(last)? + start + last.len();
    Some(source[start..end].to_string())
}

/// Paint `style` over the selected cells of `buffer` (patching, so glyphs and
/// foreground survive; typically a reversed or selection-background style).
pub fn paint_selection(buffer: &mut Buffer, area: Rect, sel: SelectionRange, style: Style) {
    for row in sel.start.1..=sel.end.1 {
        let Some((left, right)) = sel.row_span(row, area) else {
            continue;
        };
        for col in left..=right {
            buffer[(col, row)].set_style(style);
        }
    }
}

/// Maps screen regions to values for click hit-testing.
///
/// Push a region per clickable thing while laying out a frame, then resolve a
/// click position to the value drawn there. Later pushes win, so registering
/// children/overlays after their parents gives the innermost/topmost region
/// precedence.
#[derive(Clone, Debug)]
pub struct HitMap<T> {
    regions: Vec<(Rect, T)>,
}

impl<T> Default for HitMap<T> {
    fn default() -> Self {
        Self {
            regions: Vec::new(),
        }
    }
}

impl<T> HitMap<T> {
    /// An empty map with no registered regions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `value` at `area`. A zero-area rect never matches.
    pub fn push(&mut self, area: Rect, value: T) {
        self.regions.push((area, value));
    }

    /// Drop all registered regions, ready for the next frame.
    pub fn clear(&mut self) {
        self.regions.clear();
    }

    /// The value of the last-pushed region containing `(column, row)`.
    pub fn hit(&self, column: u16, row: u16) -> Option<&T> {
        self.regions
            .iter()
            .rev()
            .find(|(r, _)| in_rect(*r, column, row))
            .map(|(_, v)| v)
    }

    /// Resolve a mouse event's position through the map.
    pub fn hit_event(&self, m: &Mouse) -> Option<&T> {
        self.hit(m.column, m.row)
    }
}

fn in_rect(r: Rect, col: u16, row: u16) -> bool {
    r.width > 0 && r.height > 0 && col >= r.x && col < r.right() && row >= r.y && row < r.bottom()
}

/// A completed click: the cell and button of a `Down`/`Up` pair on one cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Click {
    /// Clicked cell column (0-based).
    pub column: u16,
    /// Clicked cell row (0-based).
    pub row: u16,
    /// The mouse button that was pressed and released.
    pub button: MouseButton,
}

/// Turns `Down`/`Up` mouse pairs into [`Click`]s. A press then release on the
/// *same* cell with the *same* button is a click; a drag in between cancels it
/// (that gesture is a selection, not a click). Feed every mouse event; `handle`
/// returns `Some(Click)` only on the completing `Up`.
#[derive(Clone, Copy, Debug, Default)]
pub struct ClickTracker {
    down: Option<(u16, u16, MouseButton)>,
}

impl ClickTracker {
    /// A tracker with no press pending.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a mouse event; returns `Some(Click)` on the `Up` that completes a
    /// press/release on the same cell with the same button.
    pub fn handle(&mut self, m: &Mouse) -> Option<Click> {
        match m.kind {
            MouseKind::Down(button) => {
                self.down = Some((m.column, m.row, button));
                None
            }
            MouseKind::Drag(_) | MouseKind::Moved => {
                // Movement disqualifies the pending press from being a click.
                self.down = None;
                None
            }
            MouseKind::Up(button) => match self.down.take() {
                Some((col, row, pressed))
                    if pressed == button && col == m.column && row == m.row =>
                {
                    Some(Click {
                        column: m.column,
                        row: m.row,
                        button,
                    })
                }
                _ => None,
            },
            _ => None,
        }
    }
}

/// Tracks which [`HitMap`] region the pointer is resting on, for hover styling.
///
/// Terminals report pointer motion as [`MouseKind::Moved`] events once mouse
/// capture is on; this pairs that stream with the hit-testing the app already
/// does for clicks. Feed every mouse event to [`handle`](Self::handle) (all of
/// them carry a cell, so drags and scrolls keep the position fresh too), then
/// after pushing the frame's regions call [`resolve`](Self::resolve) — it
/// returns `true` when the hovered value changed, which is the host's cue to
/// request a redraw. During rendering, style the hot region via
/// [`hovered`](Self::hovered) or [`is`](Self::is); to *animate* the change,
/// drive an [`anim::Transition`](crate::anim::Transition) from the resolved
/// state instead of switching styles directly.
///
/// ```
/// use tuika::mouse::{HitMap, HoverTracker};
/// use tuika::{Mouse, MouseKind};
/// use tuika::ui::Rect;
///
/// let mut hits: HitMap<u8> = HitMap::new();
/// hits.push(Rect::new(0, 0, 4, 1), 1);
/// hits.push(Rect::new(4, 0, 4, 1), 2);
///
/// let mut hover = HoverTracker::new();
/// hover.handle(&Mouse::at(MouseKind::Moved, 5, 0));
/// assert!(hover.resolve(&hits));      // entered region 2 → redraw
/// assert!(hover.is(&2));
/// assert!(!hover.resolve(&hits));     // still there → nothing to do
/// ```
#[derive(Clone, Debug)]
pub struct HoverTracker<T> {
    position: Option<(u16, u16)>,
    hovered: Option<T>,
}

impl<T> Default for HoverTracker<T> {
    fn default() -> Self {
        Self {
            position: None,
            hovered: None,
        }
    }
}

impl<T: Clone + PartialEq> HoverTracker<T> {
    /// A tracker that has not seen the pointer yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the pointer cell from a mouse event. Returns `true` when the
    /// pointer moved to a different cell — a hint that the hover may need
    /// re-resolving, not yet a style change.
    pub fn handle(&mut self, m: &Mouse) -> bool {
        let position = Some((m.column, m.row));
        let moved = position != self.position;
        self.position = position;
        moved
    }

    /// Re-resolve the pointer against `map` (typically right after the frame's
    /// regions were pushed — regions can move under a still pointer, so this
    /// matters even without a `Moved` event). Returns `true` when the hovered
    /// value changed and a redraw is warranted.
    pub fn resolve(&mut self, map: &HitMap<T>) -> bool {
        let hovered = self
            .position
            .and_then(|(col, row)| map.hit(col, row))
            .cloned();
        let changed = hovered != self.hovered;
        self.hovered = hovered;
        changed
    }

    /// The value of the region under the pointer, per the last
    /// [`resolve`](Self::resolve).
    pub fn hovered(&self) -> Option<&T> {
        self.hovered.as_ref()
    }

    /// Whether `value`'s region is the hovered one — the per-region question a
    /// renderer asks while styling.
    pub fn is(&self, value: &T) -> bool {
        self.hovered.as_ref() == Some(value)
    }

    /// The last pointer cell, if any event has been seen.
    pub fn position(&self) -> Option<(u16, u16)> {
        self.position
    }

    /// Forget the pointer (capture disabled, focus lost, pointer left the
    /// pane). Returns `true` when that cleared an active hover.
    pub fn clear(&mut self) -> bool {
        self.position = None;
        self.hovered.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Text;
    use crate::event::{Mouse, MouseButton, MouseKind};
    use crate::geometry::Rect;
    use crate::style::Theme;
    use crate::style::{Color, Style};

    fn down(col: u16, row: u16) -> Mouse {
        Mouse::at(MouseKind::Down(MouseButton::Left), col, row)
    }
    fn drag(col: u16, row: u16) -> Mouse {
        Mouse::at(MouseKind::Drag(MouseButton::Left), col, row)
    }
    fn up(col: u16, row: u16) -> Mouse {
        Mouse::at(MouseKind::Up(MouseButton::Left), col, row)
    }

    fn buffer_with_rows(rows: &[&str]) -> Buffer {
        let width = rows
            .iter()
            .map(|row| row.chars().count())
            .max()
            .unwrap_or(0) as u16;
        let area = Rect::new(0, 0, width, rows.len() as u16);
        let mut buffer = Buffer::empty(area);
        for (y, row) in rows.iter().enumerate() {
            buffer.set_string(0, y as u16, row, Style::default());
        }
        buffer
    }

    #[test]
    fn source_aware_copy_omits_soft_wraps() {
        let buffer = buffer_with_rows(&["hello", "world"]);
        let selection = SelectionRange::between((0, 0), (4, 1));

        assert_eq!(
            selected_text_from_sources(
                &buffer,
                buffer.area,
                selection,
                &[SelectionSource {
                    area: buffer.area,
                    text: "helloworld".to_string()
                }],
            ),
            "helloworld",
        );
    }

    #[test]
    fn source_aware_copy_preserves_explicit_newlines() {
        let buffer = buffer_with_rows(&["hello", "world"]);
        let selection = SelectionRange::between((0, 0), (4, 1));

        assert_eq!(
            selected_text_from_sources(
                &buffer,
                buffer.area,
                selection,
                &[SelectionSource {
                    area: buffer.area,
                    text: "hello\nworld".to_string()
                }],
            ),
            "hello\nworld",
        );
    }

    #[test]
    fn source_aware_copy_handles_unicode_and_clipped_endpoints() {
        let buffer = buffer_with_rows(&["pha α", "β omega"]);
        let selection = SelectionRange::between((0, 0), (0, 1));

        assert_eq!(
            selected_text_from_sources(
                &buffer,
                buffer.area,
                selection,
                &[SelectionSource {
                    area: buffer.area,
                    text: "alpha αβ omega".to_string()
                }],
            ),
            "pha αβ",
        );
    }

    #[test]
    fn source_area_disambiguates_identical_rendered_text() {
        let buffer = buffer_with_rows(&["hello", "world", "hello", "world"]);
        let selection = SelectionRange::between((0, 2), (4, 3));
        let sources = [
            SelectionSource {
                area: Rect::new(0, 0, 5, 2),
                text: "hello\nworld".to_string(),
            },
            SelectionSource {
                area: Rect::new(0, 2, 5, 2),
                text: "helloworld".to_string(),
            },
        ];

        assert_eq!(
            selected_text_from_sources(&buffer, buffer.area, selection, &sources),
            "helloworld",
        );
    }

    #[test]
    fn copy_falls_back_to_visual_rows_without_source_metadata() {
        let buffer = buffer_with_rows(&["hello", "world"]);
        let selection = SelectionRange::between((0, 0), (4, 1));

        assert_eq!(
            selected_text(&buffer, buffer.area, selection),
            "hello\nworld"
        );
    }

    #[test]
    fn selection_tracks_a_left_drag() {
        let mut sel = SelectionState::new();
        assert!(!sel.handle(&down(1, 0))); // fresh press: nothing to redraw yet
        assert!(sel.handle(&drag(4, 0)));
        assert!(sel.handle(&up(4, 0)));
        let range = sel.range().expect("a drag selects");
        assert_eq!(range.start, (1, 0));
        assert_eq!(range.end, (4, 0));
        assert!(sel.is_active());
    }

    #[test]
    fn a_range_reaching_past_its_area_paints_nothing_outside_it() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 2));
        let area = buffer.area;
        let sel = SelectionRange {
            start: (0, 0),
            end: (9, 6), // outlived the geometry it was made in
        };

        // Reading and hit-testing stay inside the offered area instead of
        // indexing a cell the buffer does not have.
        selected_text(&buffer, area, sel);
        assert!(!sel.contains(1, 5, area));
        paint_selection(&mut buffer, area, sel, Style::default().bg(Color::Red));
        for row in 0..2 {
            for column in 0..4 {
                assert_eq!(buffer[(column, row)].bg, Color::Red);
            }
        }
    }

    #[test]
    fn a_confined_drag_clamps_to_its_panel() {
        let panel = Rect::new(2, 1, 6, 2);
        let mut sel = SelectionState::new();
        sel.confine(Some(panel));
        sel.handle(&down(3, 1));
        sel.handle(&drag(19, 9)); // far outside the panel

        let range = sel.range().expect("a drag selects");
        assert_eq!(range.start, (3, 1));
        assert_eq!(range.end, (7, 2));
        assert_eq!(sel.region(), Some(panel));
        // Row 2 of the selection starts at the panel's left edge, not the screen's.
        assert!(!range.contains(1, 2, panel));
        assert!(range.contains(2, 2, panel));
    }

    #[test]
    fn a_zero_sized_region_leaves_the_gesture_unconfined() {
        let mut sel = SelectionState::new();
        sel.confine(Some(Rect::new(4, 4, 0, 0)));
        assert_eq!(sel.region(), None);
        sel.handle(&down(0, 0));
        sel.handle(&drag(9, 0));
        assert_eq!(sel.range().map(|r| r.end), Some((9, 0)));
    }

    #[test]
    fn the_innermost_region_wins_at_a_cell() {
        let outer = Rect::new(0, 0, 10, 4);
        let inner = Rect::new(2, 1, 4, 2);
        let regions = [outer, inner];
        assert_eq!(region_at(&regions, 3, 1), Some(inner));
        assert_eq!(region_at(&regions, 8, 1), Some(outer));
        assert_eq!(region_at(&regions, 12, 1), None);
    }

    #[test]
    fn selection_normalizes_a_backwards_drag() {
        let mut sel = SelectionState::new();
        sel.handle(&down(5, 1));
        sel.handle(&drag(2, 0));
        sel.handle(&up(2, 0));
        let range = sel.range().expect("selection");
        // Reading order: (col=2,row=0) comes before (col=5,row=1).
        assert_eq!(range.start, (2, 0));
        assert_eq!(range.end, (5, 1));
    }

    #[test]
    fn a_plain_click_leaves_no_selection() {
        let mut sel = SelectionState::new();
        sel.handle(&down(3, 0));
        sel.handle(&up(3, 0)); // released on the same cell, no drag
        assert!(sel.range().is_none());
        assert!(!sel.is_active());
    }

    #[test]
    fn a_new_press_clears_the_previous_selection() {
        let mut sel = SelectionState::new();
        sel.handle(&down(0, 0));
        sel.handle(&drag(3, 0));
        sel.handle(&up(3, 0));
        assert!(sel.range().is_some());
        // Pressing again to start a new gesture must drop the old selection.
        assert!(sel.handle(&down(1, 1)));
        assert!(sel.range().is_none());
    }

    #[test]
    fn double_click_selects_the_word_under_the_pointer() {
        let theme = Theme::default();
        let buf = crate::testing::render(&Text::raw("hello, brave_world!"), 19, 1, &theme);
        let mut sel = SelectionState::new();
        sel.handle(&down(9, 0));
        sel.handle(&up(9, 0));
        sel.handle(&down(9, 0));
        sel.handle(&up(9, 0));

        assert!(sel.resolve(&buf, buf.area));
        let range = sel.range().expect("double click selects a word");
        assert_eq!(range.start, (7, 0));
        assert_eq!(range.end, (17, 0));
        assert_eq!(selected_text(&buf, buf.area, range), "brave_world");
    }

    #[test]
    fn double_click_on_blank_space_does_not_select() {
        let theme = Theme::default();
        let buf = crate::testing::render(&Text::raw("hi"), 5, 1, &theme);
        let mut sel = SelectionState::new();
        sel.handle(&down(4, 0));
        sel.handle(&up(4, 0));
        sel.handle(&down(4, 0));
        sel.handle(&up(4, 0));

        assert!(!sel.resolve(&buf, buf.area));
        assert!(sel.range().is_none());
    }

    #[test]
    fn selected_text_reads_one_row() {
        let theme = Theme::default();
        let buf = crate::testing::render(&Text::raw("hello world"), 11, 1, &theme);
        let mut sel = SelectionState::new();
        sel.handle(&down(0, 0));
        sel.handle(&drag(4, 0));
        sel.handle(&up(4, 0));
        let text = selected_text(&buf, buf.area, sel.range().unwrap());
        assert_eq!(text, "hello");
    }

    #[test]
    fn selected_text_spans_rows_linearly() {
        use crate::text::Line;
        let theme = Theme::default();
        let lines = vec![Line::from("hello"), Line::from("world")];
        let buf = crate::testing::render(&Text::new(lines), 5, 2, &theme);
        let mut sel = SelectionState::new();
        // From the "llo" of row 0 through the "wo" of row 1.
        sel.handle(&down(2, 0));
        sel.handle(&drag(1, 1));
        sel.handle(&up(1, 1));
        let text = selected_text(&buf, buf.area, sel.range().unwrap());
        assert_eq!(text, "llo\nwo");
    }

    #[test]
    fn selected_text_trims_trailing_blanks() {
        let theme = Theme::default();
        let buf = crate::testing::render(&Text::raw("hi"), 10, 1, &theme);
        let mut sel = SelectionState::new();
        sel.handle(&down(0, 0));
        sel.handle(&drag(9, 0)); // drag well past the text into blank cells
        sel.handle(&up(9, 0));
        let text = selected_text(&buf, buf.area, sel.range().unwrap());
        assert_eq!(text, "hi");
    }

    #[test]
    fn highlight_patches_selected_cells_only() {
        let theme = Theme::default();
        let mut buf = crate::testing::render(&Text::raw("hello"), 5, 1, &theme);
        let area = buf.area;
        let mut sel = SelectionState::new();
        sel.handle(&down(0, 0));
        sel.handle(&drag(2, 0));
        sel.handle(&up(2, 0));
        paint_selection(
            &mut buf,
            area,
            sel.range().unwrap(),
            Style::default().bg(Color::Blue),
        );
        // Selected cells (0..=2) get the highlight bg; the glyph survives.
        for col in 0..=2 {
            assert_eq!(
                buf[(col, 0)].bg,
                Color::Blue,
                "cell {col} should be highlighted"
            );
        }
        assert_eq!(
            buf[(0, 0)].symbol(),
            "h",
            "highlight must not clobber the glyph"
        );
        // An unselected cell is untouched.
        assert_ne!(buf[(4, 0)].bg, Color::Blue);
    }

    #[test]
    fn hit_map_last_region_wins_and_misses_return_none() {
        let mut hits: HitMap<&str> = HitMap::new();
        hits.push(Rect::new(0, 0, 10, 10), "background");
        hits.push(Rect::new(2, 2, 3, 3), "panel"); // pushed later -> wins on overlap
        hits.push(Rect::new(0, 0, 0, 0), "zero"); // zero-area never matches
        assert_eq!(hits.hit(3, 3), Some(&"panel"));
        assert_eq!(hits.hit(8, 8), Some(&"background"));
        assert_eq!(hits.hit(50, 50), None);
        // hit_event resolves an event's coordinates.
        assert_eq!(hits.hit_event(&down(3, 3)), Some(&"panel"));
    }

    fn moved(col: u16, row: u16) -> Mouse {
        Mouse::at(MouseKind::Moved, col, row)
    }

    #[test]
    fn hover_tracks_enter_move_and_leave() {
        let mut hits: HitMap<&str> = HitMap::new();
        hits.push(Rect::new(0, 0, 4, 1), "left");
        hits.push(Rect::new(4, 0, 4, 1), "right");
        let mut hover: HoverTracker<&str> = HoverTracker::new();

        // Nothing seen yet: no hover, resolving changes nothing.
        assert_eq!(hover.hovered(), None);
        assert!(!hover.resolve(&hits));

        assert!(hover.handle(&moved(1, 0)));
        assert!(hover.resolve(&hits), "entering a region is a change");
        assert!(hover.is(&"left"));

        // Moving within the same region is not a style change.
        assert!(hover.handle(&moved(2, 0)));
        assert!(!hover.resolve(&hits));

        // Crossing into the neighbor is.
        hover.handle(&moved(5, 0));
        assert!(hover.resolve(&hits));
        assert!(hover.is(&"right"));
        assert!(!hover.is(&"left"));

        // Off every region: hover ends.
        hover.handle(&moved(20, 5));
        assert!(hover.resolve(&hits));
        assert_eq!(hover.hovered(), None);
    }

    #[test]
    fn hover_position_updates_from_any_event_kind() {
        let mut hover: HoverTracker<u8> = HoverTracker::new();
        assert!(hover.handle(&down(3, 1)), "a press carries the pointer too");
        assert_eq!(hover.position(), Some((3, 1)));
        assert!(!hover.handle(&up(3, 1)), "same cell: nothing moved");
        assert!(hover.handle(&drag(4, 1)));
        assert_eq!(hover.position(), Some((4, 1)));
    }

    #[test]
    fn hover_follows_regions_that_move_under_a_still_pointer() {
        let mut hover: HoverTracker<&str> = HoverTracker::new();
        hover.handle(&moved(2, 2));
        let mut hits: HitMap<&str> = HitMap::new();
        hits.push(Rect::new(0, 0, 8, 8), "panel");
        assert!(hover.resolve(&hits));
        // Next frame the layout shifted and the panel is elsewhere.
        hits.clear();
        hits.push(Rect::new(10, 10, 4, 4), "panel");
        assert!(hover.resolve(&hits), "region left the pointer");
        assert_eq!(hover.hovered(), None);
    }

    #[test]
    fn hover_clear_forgets_pointer_and_reports_the_drop() {
        let mut hits: HitMap<u8> = HitMap::new();
        hits.push(Rect::new(0, 0, 4, 1), 7);
        let mut hover: HoverTracker<u8> = HoverTracker::new();
        hover.handle(&moved(1, 0));
        hover.resolve(&hits);
        assert!(hover.clear(), "an active hover was dropped");
        assert_eq!(hover.position(), None);
        assert!(!hover.clear(), "already cleared");
    }

    #[test]
    fn click_tracker_detects_a_click() {
        let mut clicks = ClickTracker::new();
        assert!(clicks.handle(&down(4, 2)).is_none()); // press alone is not a click
        let click = clicks
            .handle(&up(4, 2))
            .expect("down+up on one cell is a click");
        assert_eq!(
            (click.column, click.row, click.button),
            (4, 2, MouseButton::Left)
        );
    }

    #[test]
    fn click_tracker_drag_cancels_the_click() {
        let mut clicks = ClickTracker::new();
        clicks.handle(&down(4, 2));
        clicks.handle(&drag(6, 2)); // moved -> this is a selection, not a click
        assert!(clicks.handle(&up(6, 2)).is_none());
    }

    #[test]
    fn click_tracker_release_on_another_cell_is_not_a_click() {
        let mut clicks = ClickTracker::new();
        clicks.handle(&down(4, 2));
        assert!(clicks.handle(&up(9, 9)).is_none());
    }
}
