//! Reference lines: a threshold, a target, or an event marker drawn across the
//! plot rather than plotted as data.

use tuika::ui::Color;

/// Which way a [`Reference`] runs, named by the axis it marks rather than by
/// screen direction, so the same annotation stays correct when a chart is
/// laid out [horizontally](crate::Chart::horizontal).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReferenceAt {
    /// A line through every position at one value — an SLO, a budget, a limit.
    Value(f64),
    /// A line through every value at one position — a deploy, an incident.
    Position(f64),
}

/// A straight reference line with an optional label.
///
/// References are annotations, not data: both renderers draw them beneath every
/// series and they never paint over a mark. An explicit domain still clips them;
/// an automatic domain grows to keep them visible.
///
/// ```
/// use tuika_charts::{Chart, Reference};
///
/// let chart = Chart::new()
///     .reference(Reference::value(250.0).label("SLO").dashed())
///     .reference(Reference::position(19.0).label("incident"));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Reference {
    /// Where the line is drawn.
    pub at: ReferenceAt,
    /// Text printed at the line's end — the right edge for a value line, the
    /// top for a position line. Dropped when it would cover a mark.
    pub label: Option<String>,
    /// Line color. The theme's border color when absent.
    pub color: Option<Color>,
    /// Draw a dash pattern instead of a solid line.
    pub dashed: bool,
}

impl Reference {
    /// A line through every position at `value`.
    pub fn value(value: f64) -> Self {
        Self::new(ReferenceAt::Value(value))
    }

    /// A line through every value at `position`.
    pub fn position(position: f64) -> Self {
        Self::new(ReferenceAt::Position(position))
    }

    fn new(at: ReferenceAt) -> Self {
        Self {
            at,
            label: None,
            color: None,
            dashed: false,
        }
    }

    /// Print `label` at the line's end.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Override the line color.
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Draw the line dashed.
    pub fn dashed(mut self) -> Self {
        self.dashed = true;
        self
    }

    /// Whether the line has a finite coordinate to be drawn at.
    pub(crate) fn finite(&self) -> bool {
        match self.at {
            ReferenceAt::Value(v) | ReferenceAt::Position(v) => v.is_finite(),
        }
    }
}
