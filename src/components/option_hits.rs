//! Click-to-choose support shared by the pickers.
//!
//! A picker knows where it painted each option only while it renders, and the
//! host gets the click one frame later. Before [`OptionHits`] the host had to
//! re-derive that geometry — the panel border, padding, header lines, the
//! scroll window — and keep it in step with the renderer by hand. Instead the
//! picker records the cell rect of every option it painted into a shared
//! handle, and the host's state resolves the click against exactly those rects.
//!
//! [`ClickAction`] picks between the two common click models, and
//! [`ClickConfirm`] holds a one-click choice for a brief highlight before the
//! host acts on it, so the user sees which option they hit.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::event::{Event, MouseButton, MouseKind};
use crate::geometry::Rect;

/// What a plain left click on a picker option does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClickAction {
    /// The first click moves the highlight ([`Changed`]); a click on the
    /// option that is already highlighted confirms it ([`Submitted`]). This is
    /// the safe default for choices that are hard to undo.
    ///
    /// [`Changed`]: crate::event::InputOutcome::Changed
    /// [`Submitted`]: crate::event::InputOutcome::Submitted
    #[default]
    Highlight,
    /// One click highlights *and* confirms the option ([`Submitted`]). Pair it
    /// with [`ClickConfirm`] to show the new highlight briefly before acting.
    ///
    /// [`Submitted`]: crate::event::InputOutcome::Submitted
    Choose,
}

/// The screen cells of each option a picker painted in its last frame.
///
/// Create one per picker, hand it to the view with `.hits(&hits)`
/// ([`SelectList::hits`](super::SelectList::hits),
/// [`SelectionScreen::hits`](super::SelectionScreen::hits),
/// [`TabSelect::hits`](super::TabSelect::hits)), and pass the same handle to
/// the state's `handle_click`. The view clears and refills it on every render,
/// recording only what was actually visible: rows scrolled out of a window or
/// clipped by a parent are absent, so a click there resolves to nothing.
///
/// The handle is cheap to clone (a shared cell). Like
/// [`RectProbe`](crate::probe::RectProbe), it keeps the last painted geometry
/// until the picker paints again; [`clear`](Self::clear) it when the picker
/// closes if clicks could still reach the state.
///
/// ```
/// use tuika::prelude::*;
/// use tuika::event::{Mouse, MouseButton, MouseKind};
///
/// let hits = OptionHits::new();
/// let mut state = SelectState::new();
/// let items = vec![Line::from("one"), Line::from("two")];
/// let _ = tuika::testing::render(&SelectList::new(items, &state).hits(&hits), 8, 2, &Theme::default());
///
/// let click = Event::Mouse(Mouse::at(MouseKind::Down(MouseButton::Left), 3, 1));
/// assert_eq!(hits.hit(&click), Some(1));
/// assert_eq!(state.handle_click(&click, 2, &hits, ClickAction::Choose), InputOutcome::Submitted);
/// assert_eq!(state.selected(), Some(1));
/// ```
#[derive(Clone, Debug, Default)]
pub struct OptionHits(Rc<RefCell<Vec<(Rect, usize)>>>);

impl OptionHits {
    /// Create an empty handle; it resolves nothing until a picker paints it.
    pub fn new() -> Self {
        Self::default()
    }

    /// The absolute option index painted at a cell, if any.
    pub fn option_at(&self, column: u16, row: u16) -> Option<usize> {
        self.0
            .borrow()
            .iter()
            .find(|(rect, _)| {
                column >= rect.x && column < rect.right() && row >= rect.y && row < rect.bottom()
            })
            .map(|&(_, index)| index)
    }

    /// The option under a plain left-button press, if the event is one.
    ///
    /// Modified clicks (Shift, Ctrl, Alt, Super) are left alone: terminals and
    /// hosts commonly reserve them for text selection and other gestures.
    pub fn hit(&self, event: &Event) -> Option<usize> {
        let Event::Mouse(mouse) = event else {
            return None;
        };
        if !mouse.plain() || mouse.kind != MouseKind::Down(MouseButton::Left) {
            return None;
        }
        self.option_at(mouse.column, mouse.row)
    }

    /// The recorded `(rect, index)` pairs, in paint order.
    pub fn regions(&self) -> Vec<(Rect, usize)> {
        self.0.borrow().clone()
    }

    /// Whether the last paint recorded no option.
    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }

    /// Forget the recorded geometry.
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    /// Record one painted option, clipped to what the surface can show.
    pub(crate) fn record(&self, rect: Rect, clip: Rect, index: usize) {
        let visible = rect.intersection(clip);
        if !visible.is_empty() {
            self.0.borrow_mut().push((visible, index));
        }
    }
}

/// Holds a one-click choice for a short highlight before the host acts on it.
///
/// A picker that confirms on the first click ([`ClickAction::Choose`]) would
/// otherwise close in the same frame, before the user sees which option they
/// hit. Arm this with the chosen index when a click returns
/// [`Submitted`](crate::event::InputOutcome::Submitted), repaint, and poll it
/// from [`Signal::Tick`](crate::Signal::Tick); it hands the index back
/// once the delay has passed. Time comes from the caller, so the host's
/// [`Clock`](crate::Clock) keeps it deterministic in tests.
///
/// ```
/// use std::time::{Duration, Instant};
/// use tuika::prelude::*;
///
/// let start = Instant::now();
/// let mut confirm = ClickConfirm::default();
/// confirm.arm(2, start);
/// assert_eq!(confirm.pending(), Some(2));
/// assert_eq!(confirm.poll(start + Duration::from_millis(50)), None);
/// assert_eq!(confirm.poll(start + ClickConfirm::DEFAULT_DELAY), Some(2));
/// assert_eq!(confirm.pending(), None);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct ClickConfirm {
    delay: Duration,
    pending: Option<(usize, Instant)>,
}

impl Default for ClickConfirm {
    fn default() -> Self {
        Self::new(Self::DEFAULT_DELAY)
    }
}

impl ClickConfirm {
    /// Long enough to register the new highlight, short enough not to feel
    /// like lag.
    pub const DEFAULT_DELAY: Duration = Duration::from_millis(150);

    /// A confirmer that waits `delay` after [`arm`](Self::arm).
    pub const fn new(delay: Duration) -> Self {
        Self {
            delay,
            pending: None,
        }
    }

    /// Start (or restart) the highlight for `index` at `now`.
    pub fn arm(&mut self, index: usize, now: Instant) {
        self.pending = Some((index, now));
    }

    /// The index waiting to be confirmed, if any.
    pub fn pending(&self) -> Option<usize> {
        self.pending.map(|(index, _)| index)
    }

    /// When the pending index becomes due, for hosts that schedule a wake-up.
    pub fn deadline(&self) -> Option<Instant> {
        self.pending
            .map(|(_, armed)| armed.checked_add(self.delay).unwrap_or(armed))
    }

    /// Return the pending index once its delay has elapsed at `now`, clearing
    /// it; `None` while still highlighting or when nothing is pending.
    pub fn poll(&mut self, now: Instant) -> Option<usize> {
        let deadline = self.deadline()?;
        if now >= deadline {
            self.pending.take().map(|(index, _)| index)
        } else {
            None
        }
    }

    /// Drop the pending choice, e.g. when a key moves the highlight first.
    pub fn cancel(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Mouse;

    fn click(column: u16, row: u16) -> Event {
        Event::Mouse(Mouse::at(MouseKind::Down(MouseButton::Left), column, row))
    }

    #[test]
    fn records_only_the_clipped_part_and_resolves_plain_presses() {
        let hits = OptionHits::new();
        hits.record(Rect::new(0, 0, 10, 1), Rect::new(0, 0, 5, 5), 7);
        hits.record(Rect::new(0, 9, 10, 1), Rect::new(0, 0, 5, 5), 8);
        assert_eq!(hits.regions(), vec![(Rect::new(0, 0, 5, 1), 7)]);
        assert_eq!(hits.hit(&click(4, 0)), Some(7));
        assert_eq!(hits.hit(&click(5, 0)), None, "clipped away");
        let mut shifted = Mouse::at(MouseKind::Down(MouseButton::Left), 1, 0);
        shifted.shift = true;
        assert_eq!(hits.hit(&Event::Mouse(shifted)), None);
        let release = Event::Mouse(Mouse::at(MouseKind::Up(MouseButton::Left), 1, 0));
        assert_eq!(hits.hit(&release), None);
        hits.clear();
        assert!(hits.is_empty());
    }

    fn labels(count: usize) -> Vec<crate::text::Line<'static>> {
        (0..count)
            .map(|index| crate::text::Line::from(format!("option {index}")))
            .collect()
    }

    #[test]
    fn select_list_in_a_panel_records_its_scrolled_rows() {
        use crate::components::{Boxed, SelectList, SelectState};
        use crate::event::InputOutcome;
        let hits = OptionHits::new();
        let mut state = SelectState::new();
        state.select(Some(8));
        // A bordered panel with a 3-row viewport centred on row 8: rows 7..=9
        // land on screen rows 1..=3, inside the border and its one-cell padding.
        let view = Boxed::new(SelectList::new(labels(20), &state).viewport(3).hits(&hits));
        let buffer = crate::testing::render(&view, 16, 5, &crate::Theme::default());
        assert!(crate::tests::support::row(&buffer, 1).contains("option 7"));
        assert_eq!(hits.option_at(0, 1), None, "the border is not a row");
        assert_eq!(hits.option_at(1, 1), None, "nor is the panel padding");
        assert_eq!(hits.option_at(2, 1), Some(7));
        assert_eq!(hits.option_at(13, 2), None, "nor is the scrollbar");
        assert_eq!(hits.option_at(5, 3), Some(9));
        assert_eq!(hits.option_at(5, 4), None, "the bottom border is not a row");

        // Highlight: the first click moves, the second confirms.
        assert_eq!(
            state.handle_click(&click(4, 1), 20, &hits, ClickAction::Highlight),
            InputOutcome::Changed
        );
        assert_eq!(state.selected(), Some(7));
        assert_eq!(
            state.handle_click(&click(4, 1), 20, &hits, ClickAction::Highlight),
            InputOutcome::Submitted
        );
        // Choose: one click confirms a new row.
        assert_eq!(
            state.handle_click(&click(4, 3), 20, &hits, ClickAction::Choose),
            InputOutcome::Submitted
        );
        assert_eq!(state.selected(), Some(9));
        // A shrunk collection rejects a stale index instead of selecting past it.
        assert_eq!(
            state.handle_click(&click(4, 3), 5, &hits, ClickAction::Choose),
            InputOutcome::Ignored
        );
        assert_eq!(
            state.handle_click(&click(4, 0), 20, &hits, ClickAction::Choose),
            InputOutcome::Ignored
        );
    }

    #[test]
    fn selection_screen_records_rows_below_its_chrome() {
        use crate::components::{SelectState, SelectionScreen};
        use crate::event::InputOutcome;
        let hits = OptionHits::new();
        let rows = labels(3);
        let mut state = SelectState::new();
        let screen = SelectionScreen::borrowed("Choose", &rows, &state)
            .leading_rule()
            .hits(&hits);
        let buffer = crate::testing::render(&screen, 20, 8, &crate::Theme::default());
        // Rule, title, rule, then the rows.
        assert!(crate::tests::support::row(&buffer, 4).contains("option 1"));
        assert_eq!(hits.option_at(0, 1), None, "the title is not an option");
        assert_eq!(
            state.handle_click(&click(10, 4), 3, &hits, ClickAction::Choose),
            InputOutcome::Submitted
        );
        assert_eq!(state.selected(), Some(1));
    }

    #[test]
    fn tab_select_records_each_pill() {
        use crate::components::{TabSelect, TabSelectState};
        use crate::event::InputOutcome;
        let hits = OptionHits::new();
        let mut state = TabSelectState::new();
        let names = vec!["low".into(), "mid".into(), "high".into()];
        let view = TabSelect::new(names, &state).hits(&hits);
        let _ = crate::testing::render(&view, 30, 1, &crate::Theme::default());
        // " low " covers 0..5, " mid " 5..10, " high " 10..16.
        assert_eq!(hits.option_at(4, 0), Some(0));
        assert_eq!(hits.option_at(5, 0), Some(1));
        assert_eq!(hits.option_at(15, 0), Some(2));
        assert_eq!(hits.option_at(16, 0), None);
        assert_eq!(
            state.handle_click(&click(6, 0), 3, &hits, ClickAction::Highlight),
            InputOutcome::Changed
        );
        assert_eq!(state.selected(), 1);
        assert_eq!(
            state.handle_click(&click(6, 0), 3, &hits, ClickAction::Highlight),
            InputOutcome::Submitted
        );
        assert_eq!(
            state.handle_click(&click(12, 0), 3, &hits, ClickAction::Choose),
            InputOutcome::Submitted
        );
        assert_eq!(state.selected(), 2);
    }

    #[test]
    fn multi_and_viewport_states_follow_clicks() {
        use crate::components::{MultiSelectState, SelectList, SelectViewportState};
        use crate::event::InputOutcome;
        let hits = OptionHits::new();
        let mut multi = MultiSelectState::new();
        let view = SelectList::new(labels(4), multi.cursor()).hits(&hits);
        let _ = crate::testing::render(&view, 12, 4, &crate::Theme::default());
        assert_eq!(
            multi.handle_click(&click(3, 2), 4, &hits, ClickAction::Choose),
            InputOutcome::Changed
        );
        assert!(multi.contains(2));
        assert_eq!(
            multi.handle_click(&click(3, 1), 4, &hits, ClickAction::Highlight),
            InputOutcome::Changed
        );
        assert!(
            !multi.contains(1),
            "a highlight click only moves the cursor"
        );

        let mut viewport = SelectViewportState::new();
        let window = viewport.resolve(30, 4);
        let view = SelectList::new(labels(30), viewport.selection())
            .visible_window(window)
            .hits(&hits);
        let _ = crate::testing::render(&view, 12, 4, &crate::Theme::default());
        assert_eq!(
            viewport.handle_click(&click(3, 3), 30, &hits, ClickAction::Choose),
            InputOutcome::Submitted
        );
        assert_eq!(viewport.selected(), Some(3));
        assert_eq!(viewport.resolve(30, 4).start(), 0, "the window holds still");
    }

    #[test]
    fn degenerate_sizes_record_nothing_outside_the_area() {
        use crate::components::{SelectList, SelectState, TabSelect, TabSelectState};
        let hits = OptionHits::new();
        let state = SelectState::new();
        for (width, height) in [(0, 0), (1, 1), (3, 0), (0, 3), (2, 2)] {
            let view = SelectList::new(labels(5), &state).hits(&hits);
            let _ = crate::testing::render(&view, width, height, &crate::Theme::default());
            for (rect, _) in hits.regions() {
                assert!(rect.right() <= width && rect.bottom() <= height);
            }
            let tabs =
                TabSelect::new(vec!["a".into(), "bb".into()], &TabSelectState::new()).hits(&hits);
            let _ = crate::testing::render(&tabs, width, height, &crate::Theme::default());
            for (rect, _) in hits.regions() {
                assert!(rect.right() <= width && rect.bottom() <= height);
            }
        }
    }

    #[test]
    fn confirm_waits_for_its_delay_and_can_be_cancelled() {
        let start = Instant::now();
        let mut confirm = ClickConfirm::new(Duration::from_millis(100));
        assert_eq!(confirm.poll(start), None);
        confirm.arm(3, start);
        assert_eq!(confirm.deadline(), Some(start + Duration::from_millis(100)));
        assert_eq!(confirm.poll(start + Duration::from_millis(99)), None);
        assert_eq!(confirm.poll(start + Duration::from_millis(100)), Some(3));
        assert_eq!(confirm.poll(start + Duration::from_secs(1)), None);
        confirm.arm(1, start);
        confirm.cancel();
        assert_eq!(confirm.poll(start + Duration::from_secs(1)), None);
    }
}
