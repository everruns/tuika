//! Series resolved into renderer-independent geometry.
//!
//! Stacking, bar grouping, and category placement are decided once here, in
//! data coordinates, so the cell and graphics renderers only map and paint.
//! Anything computed in a renderer instead would have to be computed twice and
//! could drift between the two paths.

use std::collections::HashMap;

use tuika::RenderCtx;
use tuika::ui::Color;

use crate::model::{Domain, PlotModel, Point};
use crate::{Chart, SeriesKind, chart_color};

/// How overlapping bar or area series combine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Stack {
    /// Draw each series against the baseline. Bars sit side by side.
    #[default]
    None,
    /// Sum series at each position, each starting where the last ended.
    Normal,
    /// Sum as [`Stack::Normal`], then scale every position to 100.
    Percent,
}

/// An axis-aligned rectangle in data coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BarRect {
    pub(crate) x0: f64,
    pub(crate) x1: f64,
    pub(crate) y0: f64,
    pub(crate) y1: f64,
    /// Centre of the band this bar belongs to. The domain is sized from the
    /// band rather than the drawn slot, so it lands on exact positions instead
    /// of accumulating the slot arithmetic's rounding.
    pub(crate) center: f64,
}

/// One series' drawable shape, in data coordinates.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Geometry {
    /// Connected samples; `stepped` uses horizontal-then-vertical segments.
    Path { points: Vec<Point>, stepped: bool },
    /// A filled region between an upper edge and whatever it rests on — the
    /// zero baseline when unstacked, the series below it when stacked.
    Band {
        upper: Vec<Point>,
        lower: Vec<Point>,
    },
    /// Independent marks.
    Points(Vec<Point>),
    /// Bars, already offset into their group slot and stacked if asked.
    /// `band` is the full width each position owns, which the domain uses so
    /// an edge bar keeps its gutter instead of sitting flush against the frame.
    Bars { bars: Vec<BarRect>, band: f64 },
    /// Ring segments, as (start, sweep) fractions of a turn.
    Arcs(Vec<Arc>),
}

/// One ring segment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Arc {
    pub(crate) start: f64,
    pub(crate) sweep: f64,
    pub(crate) value: f64,
}

/// A series resolved for drawing.
pub(crate) struct Mark {
    pub(crate) name: String,
    pub(crate) color: Color,
    pub(crate) geometry: Geometry,
    /// Sample values keyed by position, for the focus readout and value labels.
    pub(crate) samples: Vec<Point>,
    pub(crate) markers: bool,
    pub(crate) labelled: bool,
}

/// Every series' geometry plus the domains that contain it.
pub(crate) struct ChartPlan {
    pub(crate) marks: Vec<Mark>,
    pub(crate) model: PlotModel,
    /// True when the chart is a ring rather than a Cartesian plot.
    pub(crate) radial: bool,
}

impl ChartPlan {
    pub(crate) fn new(chart: &Chart, ctx: &RenderCtx) -> Option<Self> {
        let radial = chart
            .series
            .iter()
            .any(|series| matches!(series.kind, SeriesKind::Donut));
        let marks = if radial {
            arc_marks(chart, ctx)
        } else {
            cartesian_marks(chart, ctx)
        };
        if marks.is_empty() {
            return None;
        }
        // A ring is not plotted against domains, so it needs none — but the
        // struct still carries a model, and a unit one keeps every consumer
        // free of an Option it would only ever ignore.
        let model = if radial {
            PlotModel::unit()
        } else {
            PlotModel::new(chart, &marks)?
        };
        Some(Self {
            marks,
            model,
            radial,
        })
    }
}

/// Ring segments sized by each series' total, in series order.
fn arc_marks(chart: &Chart, ctx: &RenderCtx) -> Vec<Mark> {
    let totals: Vec<f64> = chart
        .series
        .iter()
        .map(|series| {
            series
                .points
                .iter()
                .filter(|point| point.y.is_finite() && point.y > 0.0)
                .map(|point| point.y)
                .sum()
        })
        .collect();
    let total: f64 = totals.iter().sum();
    if total <= 0.0 {
        return Vec::new();
    }
    let mut start = 0.0;
    let mut marks = Vec::new();
    for (index, (series, value)) in chart.series.iter().zip(totals).enumerate() {
        if !matches!(series.kind, SeriesKind::Donut) {
            continue;
        }
        let sweep = value / total;
        marks.push(Mark {
            name: series.name.clone(),
            color: chart_color(series, index, ctx),
            geometry: Geometry::Arcs(vec![Arc {
                start,
                sweep,
                value,
            }]),
            samples: vec![Point::new(index as f64, value)],
            markers: false,
            labelled: series.labelled,
        });
        start += sweep;
    }
    marks
}

fn cartesian_marks(chart: &Chart, ctx: &RenderCtx) -> Vec<Mark> {
    let bar_indices: Vec<usize> = chart
        .series
        .iter()
        .enumerate()
        .filter(|(_, series)| matches!(series.kind, SeriesKind::Bar))
        .map(|(index, _)| index)
        .collect();
    let band = band_width(chart);
    let scale = percent_scale(chart);

    // Running stack tops, keyed by x position, kept per kind so bars stack on
    // bars and areas on areas.
    let mut bar_tops: Accumulator = Accumulator::default();
    let mut area_tops: Accumulator = Accumulator::default();

    let mut marks = Vec::new();
    for (index, series) in chart.series.iter().enumerate() {
        let color = chart_color(series, index, ctx);
        let samples: Vec<Point> = series
            .points
            .iter()
            .copied()
            .filter(|point| point.x.is_finite() && point.y.is_finite())
            .map(|point| scale.apply(point))
            .collect();
        let geometry = match series.kind {
            SeriesKind::Line => Geometry::Path {
                points: samples.clone(),
                stepped: false,
            },
            SeriesKind::Step => Geometry::Path {
                points: samples.clone(),
                stepped: true,
            },
            SeriesKind::Scatter => Geometry::Points(samples.clone()),
            SeriesKind::Donut => continue,
            SeriesKind::Area => {
                let mut upper = Vec::with_capacity(samples.len());
                let mut lower = Vec::with_capacity(samples.len());
                for point in &samples {
                    let base = if chart.stack == Stack::None {
                        0.0
                    } else {
                        area_tops.take(point.x)
                    };
                    let top = base + point.y;
                    if chart.stack != Stack::None {
                        area_tops.put(point.x, top);
                    }
                    lower.push(Point::new(point.x, base));
                    upper.push(Point::new(point.x, top));
                }
                Geometry::Band { upper, lower }
            }
            SeriesKind::Bar => {
                // Unstacked bars split the band between them; stacked bars all
                // occupy it, since they never sit side by side.
                let slots = if chart.stack == Stack::None {
                    bar_indices.len().max(1)
                } else {
                    1
                };
                let slot = bar_indices.iter().position(|&i| i == index).unwrap_or(0);
                let slot = if chart.stack == Stack::None { slot } else { 0 };
                let filled = band * BAR_FILL;
                let width = filled / slots as f64;
                let offset = -filled / 2.0 + width * slot as f64;
                let bars = samples
                    .iter()
                    .map(|point| {
                        let base = if chart.stack == Stack::None {
                            0.0
                        } else {
                            bar_tops.take(point.x)
                        };
                        let top = base + point.y;
                        if chart.stack != Stack::None {
                            bar_tops.put(point.x, top);
                        }
                        BarRect {
                            x0: point.x + offset,
                            x1: point.x + offset + width,
                            y0: base,
                            y1: top,
                            center: point.x,
                        }
                    })
                    .collect();
                Geometry::Bars { bars, band }
            }
        };
        marks.push(Mark {
            name: series.name.clone(),
            color,
            geometry,
            samples,
            markers: series.markers,
            labelled: series.labelled,
        });
    }
    marks
}

/// Share of a category band the bars occupy, leaving a visible gutter between
/// neighbouring groups.
const BAR_FILL: f64 = 0.8;

/// The spacing between adjacent bar positions.
///
/// Categories are unit-spaced by construction. For numeric data the smallest
/// gap between distinct positions is the only spacing every bar fits inside.
fn band_width(chart: &Chart) -> f64 {
    if chart.x_axis.categorical() {
        return 1.0;
    }
    let mut positions: Vec<f64> = chart
        .series
        .iter()
        .filter(|series| matches!(series.kind, SeriesKind::Bar))
        .flat_map(|series| series.points.iter())
        .filter(|point| point.x.is_finite() && point.y.is_finite())
        .map(|point| point.x)
        .collect();
    positions.sort_by(f64::total_cmp);
    positions.dedup();
    positions
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|gap| gap.is_finite() && *gap > 0.0)
        .min_by(f64::total_cmp)
        .unwrap_or(1.0)
}

/// Per-position divisors that turn stacked values into percentages.
#[derive(Default)]
struct PercentScale {
    totals: Option<HashMap<u64, f64>>,
}

impl PercentScale {
    fn apply(&self, point: Point) -> Point {
        let Some(totals) = &self.totals else {
            return point;
        };
        match totals.get(&point.x.to_bits()) {
            Some(total) if *total > 0.0 => Point::new(point.x, point.y / total * 100.0),
            // A position whose series sum to nothing has no percentage to
            // state; drawing it at zero is the only honest answer.
            _ => Point::new(point.x, 0.0),
        }
    }
}

fn percent_scale(chart: &Chart) -> PercentScale {
    if chart.stack != Stack::Percent {
        return PercentScale::default();
    }
    let mut totals: HashMap<u64, f64> = HashMap::new();
    for series in &chart.series {
        if !matches!(series.kind, SeriesKind::Bar | SeriesKind::Area) {
            continue;
        }
        for point in &series.points {
            if point.x.is_finite() && point.y.is_finite() && point.y > 0.0 {
                *totals.entry(point.x.to_bits()).or_insert(0.0) += point.y;
            }
        }
    }
    PercentScale {
        totals: Some(totals),
    }
}

/// Running stack tops keyed by exact x position.
#[derive(Default)]
struct Accumulator {
    tops: HashMap<u64, f64>,
}

impl Accumulator {
    fn take(&mut self, x: f64) -> f64 {
        self.tops.get(&x.to_bits()).copied().unwrap_or(0.0)
    }

    fn put(&mut self, x: f64, top: f64) {
        self.tops.insert(x.to_bits(), top);
    }
}

impl Geometry {
    /// Every data coordinate this geometry occupies, for domain sizing.
    pub(crate) fn extents(&self) -> impl Iterator<Item = Point> + '_ {
        let boxed: Box<dyn Iterator<Item = Point>> = match self {
            Geometry::Path { points, .. } | Geometry::Points(points) => {
                Box::new(points.iter().copied())
            }
            Geometry::Band { upper, lower } => Box::new(upper.iter().chain(lower.iter()).copied()),
            Geometry::Bars { bars, band } => {
                let half = *band / 2.0;
                Box::new(bars.iter().flat_map(move |bar| {
                    [
                        Point::new(bar.center - half, bar.y0),
                        Point::new(bar.center + half, bar.y1),
                    ]
                }))
            }
            Geometry::Arcs(_) => Box::new(std::iter::empty()),
        };
        boxed
    }
}

/// Domain extents that keep a bar or area anchored to the baseline it is read
/// against. The geometry already carries that baseline as its own low edge, so
/// no separate rule is needed.
pub(crate) fn extents(marks: &[Mark]) -> Option<(Domain, Domain)> {
    let (mut xmin, mut xmax, mut ymin, mut ymax) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for point in marks
        .iter()
        .flat_map(|mark| mark.geometry.extents())
        .filter(|point| point.x.is_finite() && point.y.is_finite())
    {
        xmin = xmin.min(point.x);
        xmax = xmax.max(point.x);
        ymin = ymin.min(point.y);
        ymax = ymax.max(point.y);
    }
    xmin.is_finite().then_some((
        Domain {
            min: xmin,
            max: xmax,
        },
        Domain {
            min: ymin,
            max: ymax,
        },
    ))
}
