//! A one-cell status mark shared by tabs, trees, and activity rows.

use crate::geometry::{Rect, Size};
use crate::style::{Modifier, Style, Theme};
use crate::surface::Surface;
use crate::term::program_status::State;
use crate::text::Span;
use crate::view::{RenderCtx, View};

use super::{ActivityStatus, Spinner};

/// What a session, task, or tab is doing — the same five states OSC 7501
/// program status reports, so an app's own sessions can look the way a
/// terminal such as Rex shows its tabs, and one value can drive both the badge
/// and the report ([`State::from`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BadgeState {
    /// At rest, waiting for input.
    Idle,
    /// Running: an animated spinner.
    Working,
    /// Finished successfully.
    Done,
    /// Needs the user.
    Blocked,
    /// Failed.
    Error,
}

impl BadgeState {
    /// The one-column glyph for this state at animation `frame` (only
    /// [`Working`](Self::Working) moves).
    pub fn glyph(self, frame: u64) -> &'static str {
        match self {
            Self::Idle => "·",
            Self::Working => Spinner::new(frame).glyph(),
            Self::Done => "✓",
            Self::Blocked => "!",
            Self::Error => "×",
        }
    }

    /// The theme style for this state: muted, accent, success, warning (bold,
    /// so a blocked tab stands out), danger.
    pub fn style(self, theme: &Theme) -> Style {
        match self {
            Self::Idle => theme.muted_style(),
            Self::Working => theme.accent_style(),
            Self::Done => theme.success_style(),
            Self::Blocked => theme.warning_style().add_modifier(Modifier::BOLD),
            Self::Error => theme.danger_style(),
        }
    }

    /// The badge as a styled span, to prepend to a label of your own.
    pub fn span(self, frame: u64, theme: &Theme) -> Span<'static> {
        Span::styled(self.glyph(frame), self.style(theme))
    }
}

impl From<BadgeState> for State {
    fn from(state: BadgeState) -> Self {
        match state {
            BadgeState::Idle => State::Idle,
            BadgeState::Working => State::Working,
            BadgeState::Done => State::Done,
            BadgeState::Blocked => State::Blocked,
            BadgeState::Error => State::Error,
        }
    }
}

impl From<ActivityStatus> for BadgeState {
    /// Queued and skipped steps are at rest; the rest map one to one.
    fn from(status: ActivityStatus) -> Self {
        match status {
            ActivityStatus::Queued | ActivityStatus::Skipped => Self::Idle,
            ActivityStatus::Running => Self::Working,
            ActivityStatus::Succeeded => Self::Done,
            ActivityStatus::Failed => Self::Error,
        }
    }
}

/// A one-cell [`BadgeState`] mark. [`Tabs::badges`](super::Tabs::badges) and
/// [`TreeList::badges`](super::TreeList::badges) draw the same mark beside
/// their labels.
///
/// ![status badge demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/status_badge.gif)
///
/// Run it with `cargo run --example demo -- status_badge`.
///
/// ```
/// use tuika::prelude::*;
///
/// let badge = StatusBadge::new(BadgeState::Done);
/// let cells = tuika::testing::render(&badge, 1, 1, &Theme::default());
/// assert_eq!(tuika::testing::grid(&cells), "✓");
/// ```
#[derive(Clone, Copy, Debug)]
pub struct StatusBadge {
    state: BadgeState,
    frame: u64,
}

impl StatusBadge {
    /// A badge at animation frame zero.
    pub fn new(state: BadgeState) -> Self {
        Self { state, frame: 0 }
    }

    /// Set the host-driven animation frame (moves the working spinner).
    pub fn frame(mut self, frame: u64) -> Self {
        self.frame = frame;
        self
    }
}

impl View for StatusBadge {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        Size::new(available.width.min(1), available.height.min(1))
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.is_empty() {
            return;
        }
        surface.set_string(
            area.x,
            area.y,
            self.state.glyph(self.frame),
            self.state.style(ctx.theme),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{grid, render, render_sizes};

    const ALL: [BadgeState; 5] = [
        BadgeState::Idle,
        BadgeState::Working,
        BadgeState::Done,
        BadgeState::Blocked,
        BadgeState::Error,
    ];

    #[test]
    fn each_state_paints_its_glyph_in_its_theme_slot() {
        let theme = Theme::default();
        for state in ALL {
            let cells = render(&StatusBadge::new(state), 1, 1, &theme);
            assert_eq!(grid(&cells), state.glyph(0));
            assert_eq!(
                cells[(0, 0)].fg,
                state.style(&theme).fg.unwrap(),
                "{state:?}"
            );
        }
        assert_eq!(
            BadgeState::Blocked.style(&theme).fg,
            theme.warning_style().fg
        );
        assert_eq!(BadgeState::Error.style(&theme).fg, theme.danger_style().fg);
        assert_eq!(BadgeState::Done.style(&theme).fg, theme.success_style().fg);
    }

    #[test]
    fn working_animates() {
        assert_ne!(BadgeState::Working.glyph(0), BadgeState::Working.glyph(1));
        assert_eq!(BadgeState::Done.glyph(0), BadgeState::Done.glyph(7));
    }

    #[test]
    fn maps_to_program_status_and_from_activity() {
        assert_eq!(State::from(BadgeState::Blocked), State::Blocked);
        assert_eq!(State::from(BadgeState::Idle), State::Idle);
        assert_eq!(
            BadgeState::from(ActivityStatus::Running),
            BadgeState::Working
        );
        assert_eq!(BadgeState::from(ActivityStatus::Skipped), BadgeState::Idle);
        assert_eq!(BadgeState::from(ActivityStatus::Failed), BadgeState::Error);
    }

    #[test]
    fn survives_degenerate_sizes() {
        let sizes: [(u16, u16); 5] = [(0, 0), (0, 1), (1, 0), (1, 1), (3, 2)];
        for buffer in render_sizes(
            &StatusBadge::new(BadgeState::Working),
            sizes,
            &Theme::default(),
        ) {
            let _ = grid(&buffer);
        }
    }
}
