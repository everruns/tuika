use ratatui_core::style::Color;

use crate::Chart;

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
        }
    }

    /// Override the palette color.
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
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
    pub(crate) x: Domain,
    pub(crate) y: Domain,
}

impl PlotModel {
    pub(crate) fn new(chart: &Chart) -> Option<Self> {
        let points = chart
            .series
            .iter()
            .flat_map(|series| series.points.iter())
            .filter(|point| point.x.is_finite() && point.y.is_finite());
        let (mut xmin, mut xmax, mut ymin, mut ymax) = (
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        );
        for point in points {
            xmin = xmin.min(point.x);
            xmax = xmax.max(point.x);
            ymin = ymin.min(point.y);
            ymax = ymax.max(point.y);
        }
        if !xmin.is_finite() {
            return None;
        }
        let x = chart.x_domain.unwrap_or_else(|| {
            let domain = padded_domain(xmin, xmax);
            pad_automatic_bar_domain(chart, domain)
        });
        let y = chart.y_domain.unwrap_or_else(|| {
            let domain = padded_domain(ymin, ymax);
            include_baseline(chart, domain)
        });
        Some(Self { x, y })
    }

    pub(crate) fn map(&self, point: Point, width: u32, height: u32) -> (i32, i32) {
        // Clamp before converting to integer coordinates. Besides defining the
        // public clipping behavior, this bounds line work for extreme finite
        // input values against a small explicit domain.
        let normalized_x = ((point.x - self.x.min) / (self.x.max - self.x.min)).clamp(0.0, 1.0);
        let normalized_y = ((point.y - self.y.min) / (self.y.max - self.y.min)).clamp(0.0, 1.0);
        let x = (normalized_x * width.saturating_sub(1) as f64).round() as i32;
        let y = (height.saturating_sub(1) as f64 - normalized_y * height.saturating_sub(1) as f64)
            .round() as i32;
        (x, y)
    }
}

/// Extend an automatic y domain to the zero baseline when the chart draws
/// geometry that is read as a distance from it.
///
/// A bar or an area encodes its value in the filled span, not in the position
/// of its tip. Starting the domain at the data minimum makes a bar of 1 beside
/// a bar of 5 a sliver beside a full column, which states a ratio the data does
/// not contain. Lines, steps, and scatter marks are read as positions, so they
/// keep the tighter domain that resolves their variation.
///
/// Explicit domains are untouched: they remain exact clipping bounds.
fn include_baseline(chart: &Chart, domain: Domain) -> Domain {
    let baselined = chart
        .series
        .iter()
        .any(|series| matches!(series.kind, SeriesKind::Bar | SeriesKind::Area));
    if !baselined {
        return domain;
    }
    let (min, max) = (domain.min.min(0.0), domain.max.max(0.0));
    if min < max && min.is_finite() && max.is_finite() {
        Domain { min, max }
    } else {
        domain
    }
}

fn pad_automatic_bar_domain(chart: &Chart, domain: Domain) -> Domain {
    let mut positions = chart
        .series
        .iter()
        .filter(|series| matches!(series.kind, SeriesKind::Bar))
        .flat_map(|series| series.points.iter())
        .filter(|point| point.x.is_finite() && point.y.is_finite())
        .map(|point| point.x)
        .collect::<Vec<_>>();
    positions.sort_by(f64::total_cmp);
    positions.dedup();
    let spacing = positions
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|spacing| spacing.is_finite() && *spacing > 0.0)
        .min_by(f64::total_cmp);
    let Some(pad) = spacing.map(|spacing| spacing / 2.0) else {
        return domain;
    };
    let (min, max) = (domain.min - pad, domain.max + pad);
    if min.is_finite() && max.is_finite() {
        Domain { min, max }
    } else {
        domain
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
