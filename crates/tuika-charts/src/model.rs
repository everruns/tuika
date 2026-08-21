use tuika::ui::Color;

use crate::Chart;
use crate::plan::Mark;

/// A numeric point in chart coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// Horizontal value.
    pub x: f64,
    /// Vertical value.
    pub y: f64,
}

impl Point {
    /// Construct a point.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// The shared chart grammar supported by both adaptive renderers.
#[derive(Clone, Debug, PartialEq)]
pub enum SeriesKind {
    /// Connect points in ascending input order.
    Line,
    /// Draw vertical bars from the zero baseline (or the visible domain edge).
    Bar,
    /// Connect points and fill the region down to the zero baseline.
    Area,
    /// Draw independent point markers without connecting them.
    Scatter,
    /// Connect points with horizontal-then-vertical segments.
    Step,
    /// A ring segment sized by the series' total, drawn as part of a donut.
    Donut,
}

/// One named data series.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    /// Name used in the legend.
    pub name: String,
    /// Geometry used by both renderers.
    pub kind: SeriesKind,
    /// Data points. Non-finite points are ignored.
    pub points: Vec<Point>,
    /// Explicit series color. The chart palette supplies one when absent.
    pub color: Option<Color>,
    /// Draw a glyph at every sample, not just the connecting geometry.
    pub markers: bool,
    /// Print each sample's value beside its mark.
    pub labelled: bool,
}

impl Series {
    /// Construct a line series.
    pub fn line(name: impl Into<String>, points: impl IntoIterator<Item = Point>) -> Self {
        Self::new(name, SeriesKind::Line, points)
    }

    /// Construct a bar series.
    pub fn bar(name: impl Into<String>, points: impl IntoIterator<Item = Point>) -> Self {
        Self::new(name, SeriesKind::Bar, points)
    }

    /// Construct a filled area series.
    pub fn area(name: impl Into<String>, points: impl IntoIterator<Item = Point>) -> Self {
        Self::new(name, SeriesKind::Area, points)
    }

    /// Construct a scatter series.
    pub fn scatter(name: impl Into<String>, points: impl IntoIterator<Item = Point>) -> Self {
        Self::new(name, SeriesKind::Scatter, points)
    }

    /// Construct a stepped line series.
    pub fn step(name: impl Into<String>, points: impl IntoIterator<Item = Point>) -> Self {
        Self::new(name, SeriesKind::Step, points)
    }

    /// Construct a donut segment. Its share of the ring is its total.
    pub fn donut(name: impl Into<String>, points: impl IntoIterator<Item = Point>) -> Self {
        Self::new(name, SeriesKind::Donut, points)
    }

    fn new(
        name: impl Into<String>,
        kind: SeriesKind,
        points: impl IntoIterator<Item = Point>,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            points: points.into_iter().collect(),
            color: None,
            markers: false,
            labelled: false,
        }
    }

    /// Override the palette color.
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Mark every sample, so individual readings stay visible on a line that
    /// would otherwise only show a trend.
    pub fn markers(mut self) -> Self {
        self.markers = true;
        self
    }

    /// Print each sample's value beside its mark. Labels that would overlap one
    /// already placed, or leave the plot, are dropped.
    pub fn labels(mut self) -> Self {
        self.labelled = true;
        self
    }
}

/// A fixed numeric domain. Values outside it are clipped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Domain {
    /// Inclusive lower edge.
    pub min: f64,
    /// Inclusive upper edge.
    pub max: f64,
}

impl Domain {
    /// Construct a valid increasing finite domain.
    pub fn new(min: f64, max: f64) -> Option<Self> {
        (min.is_finite() && max.is_finite() && min < max).then_some(Self { min, max })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct PlotModel {
    /// The category domain in vertical orientation, the value domain otherwise.
    pub(crate) x: Domain,
    pub(crate) y: Domain,
    pub(crate) horizontal: bool,
}

impl PlotModel {
    /// Size the plot around resolved geometry rather than raw points, so
    /// stacked totals and bar widths are inside the domain by construction.
    pub(crate) fn new(chart: &Chart, marks: &[Mark]) -> Option<Self> {
        let (x, y) = crate::plan::extents(marks)?;
        Some(Self {
            x: chart
                .x_domain
                .unwrap_or_else(|| padded_domain(x.min, x.max)),
            y: chart
                .y_domain
                .unwrap_or_else(|| padded_domain(y.min, y.max)),
            horizontal: chart.horizontal,
        })
    }

    /// A placeholder domain for charts that are not plotted against axes.
    pub(crate) fn unit() -> Self {
        Self {
            x: Domain { min: 0.0, max: 1.0 },
            y: Domain { min: 0.0, max: 1.0 },
            horizontal: false,
        }
    }

    /// Map a data point to a cell or pixel offset inside a `width × height`
    /// plot, with the origin at the bottom left.
    ///
    /// In horizontal orientation the two axes exchange roles: `x` carries the
    /// value and `y` the category, so one mapping serves both orientations and
    /// no caller has to branch.
    pub(crate) fn map(&self, point: Point, width: u32, height: u32) -> (i32, i32) {
        let (across, up) = if self.horizontal {
            (point.y, point.x)
        } else {
            (point.x, point.y)
        };
        let (across_domain, up_domain) = self.axes();
        // Clamp before converting to integer coordinates. Besides defining the
        // public clipping behavior, this bounds line work for extreme finite
        // input values against a small explicit domain.
        let normalized_x = ((across - across_domain.min) / (across_domain.max - across_domain.min))
            .clamp(0.0, 1.0);
        let normalized_y = ((up - up_domain.min) / (up_domain.max - up_domain.min)).clamp(0.0, 1.0);
        let x = (normalized_x * width.saturating_sub(1) as f64).round() as i32;
        let y = (height.saturating_sub(1) as f64 - normalized_y * height.saturating_sub(1) as f64)
            .round() as i32;
        (x, y)
    }

    /// The domains as the screen uses them: horizontal first, then vertical.
    pub(crate) fn axes(&self) -> (Domain, Domain) {
        if self.horizontal {
            (self.y, self.x)
        } else {
            (self.x, self.y)
        }
    }

    /// A point that maps to `value` on the screen's vertical axis.
    pub(crate) fn up_at(&self, value: f64) -> Point {
        if self.horizontal {
            Point::new(value, self.y.min)
        } else {
            Point::new(self.x.min, value)
        }
    }

    /// A point that maps to `value` on the screen's horizontal axis.
    pub(crate) fn across_at(&self, value: f64) -> Point {
        if self.horizontal {
            Point::new(self.x.min, value)
        } else {
            Point::new(value, self.y.min)
        }
    }
}

fn padded_domain(min: f64, max: f64) -> Domain {
    if min < max {
        Domain { min, max }
    } else {
        let pad = if min == 0.0 { 1.0 } else { min.abs() * 0.1 };
        Domain {
            min: min - pad,
            max: max + pad,
        }
    }
}
