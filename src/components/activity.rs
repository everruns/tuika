//! Multi-step activity and task status.
//!
//! An [`ActivityList`] describes *which* steps are queued, running, complete,
//! failed, or skipped. [`ProgressBar`](super::ProgressBar) answers the separate
//! question of *how much* of one measurable step is complete; an activity item
//! may include that fraction and the list composes a bar beneath the status row.

use crate::geometry::Rect;
use crate::style::Style;

use crate::geometry::Size;
use crate::style::SemanticRole;
use crate::surface::Surface;
use crate::view::{RenderCtx, View};
use crate::width::str_cols;

use super::{ProgressBar, Spinner};

/// Lifecycle state of one [`ActivityItem`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityStatus {
    /// Waiting for an earlier step or external condition.
    Queued,
    /// Work is currently in progress.
    Running,
    /// Work completed successfully.
    Succeeded,
    /// Work terminated unsuccessfully.
    Failed,
    /// Work was deliberately not run.
    Skipped,
}

impl ActivityStatus {
    fn glyph(self, frame: u64) -> &'static str {
        match self {
            Self::Queued => "○",
            Self::Running => Spinner::new(frame).glyph(),
            Self::Succeeded => "✓",
            Self::Failed => "×",
            Self::Skipped => "−",
        }
    }

    fn style(self, ctx: &RenderCtx) -> Style {
        match self {
            Self::Queued => ctx.theme.muted_style(),
            Self::Running => ctx.theme.accent_style(),
            Self::Succeeded => ctx.theme.semantic_style(SemanticRole::Success),
            Self::Failed => ctx.theme.semantic_style(SemanticRole::Danger),
            Self::Skipped => ctx.theme.muted_style(),
        }
    }
}

/// One labeled operation in an [`ActivityList`].
#[derive(Clone, Debug, PartialEq)]
pub struct ActivityItem {
    label: String,
    detail: Option<String>,
    status: ActivityStatus,
    progress: Option<f32>,
}

impl ActivityItem {
    /// Create a labeled item in `status`.
    pub fn new(label: impl Into<String>, status: ActivityStatus) -> Self {
        Self {
            label: label.into(),
            detail: None,
            status,
            progress: None,
        }
    }

    /// Add secondary text to the status row.
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Add determinate progress beneath the status row, clamped to `0.0..=1.0`.
    pub fn progress(mut self, fraction: f32) -> Self {
        self.progress = Some(fraction.clamp(0.0, 1.0));
        self
    }

    /// Current lifecycle state.
    pub fn status(&self) -> ActivityStatus {
        self.status
    }

    /// Optional determinate completion fraction.
    pub fn progress_fraction(&self) -> Option<f32> {
        self.progress
    }
}

/// Vertical task/activity history with optional per-step progress bars.
///
/// The host supplies a frame counter for running spinners. Items are snapshots
/// of host state for the current frame; the list owns no scheduler or workflow.
///
/// ![activity list demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/activity_list.gif)
///
/// Run it with `cargo run --example demo -- activity_list`.
///
/// ```
/// use tuika::prelude::*;
///
/// let tasks = vec![
///     ActivityItem::new("Resolve dependencies", ActivityStatus::Succeeded),
///     ActivityItem::new("Compile", ActivityStatus::Running).progress(0.42),
/// ];
/// let view = ActivityList::new(tasks).frame(12).gap(1);
/// let rendered = tuika::testing::render(&view, 40, 4, &Theme::default());
/// assert!(tuika::testing::grid(&rendered).contains("Compile"));
/// ```
pub struct ActivityList {
    items: Vec<ActivityItem>,
    frame: u64,
    gap: u16,
    show_percent: bool,
}

impl ActivityList {
    /// Create a list at animation frame zero.
    pub fn new(items: Vec<ActivityItem>) -> Self {
        Self {
            items,
            frame: 0,
            gap: 0,
            show_percent: true,
        }
    }

    /// Set the host-driven animation frame for running rows.
    pub fn frame(mut self, frame: u64) -> Self {
        self.frame = frame;
        self
    }

    /// Set blank rows between activity items.
    pub fn gap(mut self, rows: u16) -> Self {
        self.gap = rows;
        self
    }

    /// Show percentage labels beside embedded progress bars (default true).
    pub fn percent(mut self, show: bool) -> Self {
        self.show_percent = show;
        self
    }

    fn height(&self) -> u16 {
        let rows = self.items.iter().fold(0u16, |height, item| {
            height.saturating_add(1 + u16::from(item.progress.is_some()))
        });
        rows.saturating_add(
            self.gap
                .saturating_mul(self.items.len().saturating_sub(1) as u16),
        )
    }
}

impl View for ActivityList {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        let content_width = self
            .items
            .iter()
            .map(|item| {
                let detail = item
                    .detail
                    .as_deref()
                    .map(|detail| str_cols(detail).saturating_add(2))
                    .unwrap_or(0);
                str_cols(&item.label)
                    .saturating_add(detail)
                    .saturating_add(2)
            })
            .max()
            .unwrap_or(0);
        let width = if self.items.iter().any(|item| item.progress.is_some()) {
            available.width
        } else {
            content_width.min(available.width)
        };
        Size::new(width, self.height().min(available.height))
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.is_empty() {
            return;
        }
        let mut y = area.y;
        for (index, item) in self.items.iter().enumerate() {
            if y >= area.bottom() {
                break;
            }
            let status_style = item.status.style(ctx);
            surface.set_string(area.x, y, item.status.glyph(self.frame), status_style);
            let mut x = surface.set_string(
                area.x.saturating_add(2),
                y,
                &item.label,
                ctx.theme.text_style(),
            );
            if let Some(detail) = &item.detail
                && x < area.right()
            {
                x = surface.set_string(x, y, "  ", ctx.theme.muted_style());
                let _ = surface.set_string(x, y, detail, ctx.theme.muted_style());
            }
            y = y.saturating_add(1);

            if let Some(progress) = item.progress
                && y < area.bottom()
            {
                let bar_area =
                    Rect::new(area.x.saturating_add(2), y, area.width.saturating_sub(2), 1);
                ProgressBar::determinate(progress)
                    .percent(self.show_percent)
                    .render(bar_area, surface, ctx);
                y = y.saturating_add(1);
            }

            if index + 1 < self.items.len() {
                y = y.saturating_add(self.gap);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{grid, render};
    use crate::tests::support::rainbow_theme;

    #[test]
    fn statuses_render_semantic_glyphs_and_colors() {
        let theme = rainbow_theme();
        let view = ActivityList::new(vec![
            ActivityItem::new("queued", ActivityStatus::Queued),
            ActivityItem::new("running", ActivityStatus::Running),
            ActivityItem::new("done", ActivityStatus::Succeeded),
            ActivityItem::new("failed", ActivityStatus::Failed),
            ActivityItem::new("skipped", ActivityStatus::Skipped),
        ])
        .frame(0);
        let buffer = render(&view, 24, 5, &theme);
        let rendered = grid(&buffer);
        let rows: Vec<_> = rendered.lines().map(str::trim_end).collect();
        assert_eq!(
            rows,
            ["○ queued", "⠋ running", "✓ done", "× failed", "− skipped"]
        );
        assert_eq!(buffer[(0, 1)].fg, theme.accent);
        assert_eq!(
            buffer[(0, 2)].fg,
            theme.semantic_color(SemanticRole::Success)
        );
        assert_eq!(
            buffer[(0, 3)].fg,
            theme.semantic_color(SemanticRole::Danger)
        );
    }

    #[test]
    fn progress_is_a_second_row_with_clamped_fraction() {
        let item = ActivityItem::new("download", ActivityStatus::Running)
            .detail("archive")
            .progress(1.5);
        assert_eq!(item.progress_fraction(), Some(1.0));
        let view = ActivityList::new(vec![item]).percent(true);
        let rendered = grid(&render(&view, 24, 2, &rainbow_theme()));
        assert!(rendered.contains("download  archive"));
        assert!(rendered.contains("100%"));
    }

    #[test]
    fn gap_and_progress_determine_height() {
        let view = ActivityList::new(vec![
            ActivityItem::new("one", ActivityStatus::Succeeded),
            ActivityItem::new("two", ActivityStatus::Running).progress(0.5),
        ])
        .gap(1);
        assert_eq!(view.height(), 4);
        assert_eq!(
            view.measure(Size::new(20, 10), &RenderCtx::new(&rainbow_theme())),
            Size::new(20, 4)
        );
    }

    #[test]
    fn activity_list_handles_empty_and_tiny_areas() {
        let theme = rainbow_theme();
        let _ = render(&ActivityList::new(Vec::new()), 0, 0, &theme);
        let view = ActivityList::new(vec![
            ActivityItem::new("running", ActivityStatus::Running).progress(0.3),
        ]);
        for width in 0..4 {
            for height in 0..3 {
                let _ = render(&view, width, height, &theme);
            }
        }
    }
}
