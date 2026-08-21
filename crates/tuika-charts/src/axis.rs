//! Axis tick generation and label placement.
//!
//! Tick labels are laid out in cells in **both** render modes: the gutter left
//! of the plot and the row below it are never part of the graphics image. One
//! label renderer therefore serves both paths, so a chart cannot say different
//! things depending on whether the terminal has a graphics protocol.

use std::fmt;
use std::sync::Arc;

use tuika::ui::Rect;
use tuika::{RenderCtx, Surface};

use crate::Chart;
use crate::model::{Domain, PlotModel};

type Formatter = Arc<dyn Fn(f64) -> String>;

/// Tick labels along one chart axis.
///
/// Both axes are labelled by default. [`Axis::hidden`] drops the labels and
/// gives their rows and columns back to the plot, which is what a sparkline or
/// a chart embedded in a dense dashboard wants.
///
/// ```
/// use tuika_charts::{Axis, Chart};
///
/// let chart = Chart::new()
///     .y_axis(Axis::new().ticks(3).format(|value| format!("{value:.0}ms")))
///     .x_axis(Axis::hidden());
/// ```
#[derive(Clone)]
pub struct Axis {
    visible: bool,
    ticks: Option<usize>,
    format: Option<Formatter>,
    categories: Option<Vec<String>>,
}

impl Axis {
    /// A labelled axis with an automatic tick count.
    pub fn new() -> Self {
        Self {
            visible: true,
            ticks: None,
            format: None,
            categories: None,
        }
    }

    /// An axis with no tick labels.
    pub fn hidden() -> Self {
        Self {
            visible: false,
            ..Self::new()
        }
    }

    /// Aim for `count` tick intervals instead of deriving one from the space.
    ///
    /// The count is a target, not a guarantee: ticks land on round values, and
    /// labels that do not fit are dropped.
    pub fn ticks(mut self, count: usize) -> Self {
        self.ticks = Some(count);
        self
    }

    /// Format tick values, replacing the automatic numeric formatter.
    pub fn format(mut self, format: impl Fn(f64) -> String + 'static) -> Self {
        self.format = Some(Arc::new(format));
        self
    }

    /// Label positions `0, 1, 2, …` with names instead of numbers.
    ///
    /// This is what turns a numeric plot into the categorical one most bar and
    /// area charts actually want: series carry `Point::new(index, value)`, and
    /// the axis names each index. Ticks land on every category rather than on
    /// round numbers, and thinning drops whole names rather than truncating
    /// them, because half a month is not a shorter month.
    pub fn categories<T: Into<String>>(mut self, names: impl IntoIterator<Item = T>) -> Self {
        self.categories = Some(names.into_iter().map(Into::into).collect());
        self
    }

    /// The name this axis gives a position, when it names positions at all.
    pub(crate) fn category_name(&self, position: f64) -> Option<String> {
        let names = self.categories.as_ref()?;
        let index = usize::try_from(position.round() as i64).ok()?;
        names.get(index).cloned()
    }

    /// Whether this axis names positions instead of measuring them.
    pub(crate) fn categorical(&self) -> bool {
        self.categories
            .as_ref()
            .is_some_and(|names| !names.is_empty())
    }

    /// Tick values paired with their rendered labels.
    fn labels(&self, domain: Domain, target: usize) -> Vec<(f64, String)> {
        if !self.visible {
            return Vec::new();
        }
        if let Some(names) = self.categories.as_ref().filter(|names| !names.is_empty()) {
            return names
                .iter()
                .enumerate()
                .map(|(index, name)| (index as f64, name.clone()))
                .filter(|(value, _)| (domain.min..=domain.max).contains(value))
                .collect();
        }
        let target = self.ticks.unwrap_or(target).clamp(1, 12);
        let step = nice_step(domain.max - domain.min, target);
        if !step.is_finite() || step <= 0.0 {
            return Vec::new();
        }
        tick_values(domain, step)
            .into_iter()
            .map(|value| {
                let label = match &self.format {
                    Some(format) => format(value),
                    None => default_label(value, step),
                };
                (value, label)
            })
            .filter(|(_, label)| !label.is_empty())
            .collect()
    }
}

impl Default for Axis {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Axis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Axis")
            .field("visible", &self.visible)
            .field("ticks", &self.ticks)
            .field("format", &self.format.is_some())
            .field("categories", &self.categories)
            .finish()
    }
}

/// Round `span / target` up to a power of ten times 1, 2, 2.5, or 5, so ticks
/// land on values a reader recognizes rather than on the raw division.
///
/// 2.5 is on the ladder because dropping it forces the common `0..100 in 4`
/// case up to a step of 50, halving the labels a chart asked for.
fn nice_step(span: f64, target: usize) -> f64 {
    let raw = span / target.max(1) as f64;
    if !raw.is_finite() || raw <= 0.0 {
        return f64::NAN;
    }
    let magnitude = 10f64.powi(raw.log10().floor() as i32);
    let normalized = raw / magnitude;
    let factor = [1.0, 2.0, 2.5, 5.0, 10.0]
        .into_iter()
        .find(|&factor| normalized <= factor)
        .unwrap_or(10.0);
    factor * magnitude
}

fn tick_values(domain: Domain, step: f64) -> Vec<f64> {
    // Accumulate from the first tick rather than adding repeatedly, so a long
    // run of small steps cannot drift off the round values it started on.
    let first = (domain.min / step).ceil() * step;
    if !first.is_finite() {
        return Vec::new();
    }
    let mut values = Vec::new();
    let slack = step * 1e-9;
    for index in 0..64 {
        let value = first + step * f64::from(index);
        if value > domain.max + slack {
            break;
        }
        // Normalizes -0.0, which would otherwise print as "-0".
        values.push(if value == 0.0 { 0.0 } else { value });
    }
    values
}

/// Print with just enough decimals to distinguish neighbouring ticks. Every
/// tick is a multiple of the step, so the step's own precision is sufficient —
/// and its *magnitude* is not: a step of 0.25 needs two decimals, not one.
fn default_label(value: f64, step: f64) -> String {
    let mut decimals = 0;
    while decimals < 6 && (step * 10f64.powi(decimals)).fract().abs() > 1e-6 {
        decimals += 1;
    }
    let decimals = decimals as usize;
    format!("{value:.decimals$}")
}

/// The axes as the screen uses them: the one along the bottom, then the one up
/// the side. Horizontal orientation swaps which configured axis plays each
/// role, so every consumer below can stay in screen terms.
pub(crate) fn screen_axes(chart: &Chart) -> (&Axis, &Axis) {
    if chart.horizontal {
        (&chart.y_axis, &chart.x_axis)
    } else {
        (&chart.x_axis, &chart.y_axis)
    }
}

/// A tick resolved to a screen position.
pub(crate) struct Tick {
    pub(crate) value: f64,
    pub(crate) label: String,
    /// Screen row for a y tick, centre column for an x tick.
    pub(crate) at: u16,
    /// First screen column of the label.
    pub(crate) start: u16,
}

/// How much chrome the axes claim, and the labels they claim it for.
///
/// Resolved in two steps because the plot rect depends on the gutter width,
/// which depends on the widest y label, while tick *positions* depend on the
/// plot rect in turn.
pub(crate) struct AxisLayout {
    y_labels: Vec<(f64, String)>,
    /// Columns reserved left of the plot, at least the one the axis line uses.
    pub(crate) gutter: u16,
    /// Whether the margin row below the plot carries x tick labels.
    labelled_bottom: bool,
    pub(crate) y: Vec<Tick>,
    pub(crate) x: Vec<Tick>,
}

impl AxisLayout {
    /// Measure the chrome the axes need inside `area`.
    ///
    /// Only the gutter is new space. The row the x labels use is the margin
    /// [`plot_rect`](crate::plot_rect) has always reserved below the plot, so
    /// labelling the x axis costs no plot height.
    pub(crate) fn new(area: Rect, chart: &Chart, model: &PlotModel) -> Self {
        let mut layout = Self {
            y_labels: Vec::new(),
            gutter: 1,
            labelled_bottom: false,
            y: Vec::new(),
            x: Vec::new(),
        };
        let chrome = u16::from(!chart.title.is_empty())
            + u16::from(chart.legend && !chart.series.is_empty())
            + 1;
        let rows = area.height.saturating_sub(chrome);
        if rows < 3 {
            return layout;
        }
        let (across_axis, up_axis) = screen_axes(chart);
        layout.labelled_bottom = across_axis.visible;
        let (_, up_domain) = model.axes();
        let target = usize::from(rows / 2).clamp(2, 6);
        let labels = up_axis.labels(up_domain, target);
        let width = labels
            .iter()
            .map(|(_, label)| label.chars().count())
            .max()
            .unwrap_or(0);
        // A gutter wider than half the area would leave no plot worth drawing;
        // drop the labels rather than the chart.
        let Ok(width) = u16::try_from(width) else {
            return layout;
        };
        if width > 0 && width + 2 <= area.width / 2 {
            layout.gutter = width + 1;
            layout.y_labels = labels;
        }
        layout
    }

    /// Resolve tick positions once the plot rect is known.
    pub(crate) fn resolve(&mut self, plot: Rect, area: Rect, chart: &Chart, model: &PlotModel) {
        let data_width = u32::from(plot.width.saturating_sub(1));
        let data_height = u32::from(plot.height.saturating_sub(1));
        if data_width == 0 || data_height == 0 {
            return;
        }
        self.y = std::mem::take(&mut self.y_labels)
            .into_iter()
            .filter_map(|(value, label)| {
                let row = model.map(model.up_at(value), data_width, data_height).1;
                let row = plot.y.checked_add(u16::try_from(row).ok()?)?;
                let len = u16::try_from(label.chars().count()).ok()?;
                let start = plot.x.checked_sub(len + 1)?;
                (row < plot.bottom()).then_some(Tick {
                    value,
                    label,
                    at: row,
                    start,
                })
            })
            .collect();

        if !self.labelled_bottom || plot.bottom() >= area.bottom() {
            return;
        }
        // Thin left to right: a label that would touch the previous one is
        // dropped, so a narrow plot degrades to fewer labels, never to a
        // smear of overlapping ones.
        let (across_axis, _) = screen_axes(chart);
        let (across_domain, _) = model.axes();
        let target = usize::try_from(data_width / 10).unwrap_or(2).clamp(2, 8);
        let mut next = area.x;
        for (value, label) in across_axis.labels(across_domain, target) {
            let Ok(len) = u16::try_from(label.chars().count()) else {
                continue;
            };
            let column = model.map(model.across_at(value), data_width, data_height).0;
            let Some(centre) = u16::try_from(column).ok().map(|c| plot.x + 1 + c) else {
                continue;
            };
            let start = centre.saturating_sub(len / 2);
            if start < next || start + len > area.right() {
                continue;
            }
            next = start + len + 1;
            self.x.push(Tick {
                value,
                label,
                at: centre,
                start,
            });
        }
    }

    /// Paint both axes' labels. Shared by the cell and graphics paths.
    pub(crate) fn render(&self, plot: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        let style = ctx.theme.muted_style();
        for tick in &self.y {
            surface.set_string(tick.start, tick.at, &tick.label, style);
        }
        let row = plot.bottom();
        for tick in &self.x {
            surface.set_string(tick.start, row, &tick.label, style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(min: f64, max: f64, target: usize) -> Vec<String> {
        Axis::new()
            .ticks(target)
            .labels(Domain::new(min, max).unwrap(), target)
            .into_iter()
            .map(|(_, label)| label)
            .collect()
    }

    #[test]
    fn ticks_land_on_round_values() {
        assert_eq!(labels(0.0, 100.0, 4), ["0", "25", "50", "75", "100"]);
        assert_eq!(labels(0.0, 100.0, 5), ["0", "20", "40", "60", "80", "100"]);
        assert_eq!(labels(0.0, 10.0, 2), ["0", "5", "10"]);
    }

    #[test]
    fn ticks_stay_inside_the_domain() {
        for (min, max) in [(3.0, 97.0), (-7.5, 2.5), (1e6, 1e6 + 3.0)] {
            let domain = Domain::new(min, max).unwrap();
            for (value, _) in Axis::new().labels(domain, 4) {
                assert!((min..=max).contains(&value), "{value} outside {min}..{max}");
            }
        }
    }

    #[test]
    fn label_precision_follows_the_step_not_its_magnitude() {
        // A step of 0.25 needs two decimals; one would print 0.25 as "0.2".
        assert_eq!(
            labels(0.0, 1.0, 4),
            ["0.00", "0.25", "0.50", "0.75", "1.00"]
        );
        assert_eq!(
            labels(0.0, 1.0, 5),
            ["0.0", "0.2", "0.4", "0.6", "0.8", "1.0"]
        );
    }

    #[test]
    fn zero_never_prints_as_negative_zero() {
        assert!(labels(-1.0, 1.0, 2).iter().all(|label| label != "-0"));
    }

    #[test]
    fn a_hidden_axis_produces_no_labels() {
        assert!(
            Axis::hidden()
                .labels(Domain::new(0.0, 1.0).unwrap(), 4)
                .is_empty()
        );
    }

    #[test]
    fn degenerate_spans_produce_no_ticks_instead_of_looping() {
        assert!(nice_step(0.0, 4).is_nan(), "an empty span has no step");
        assert!(nice_step(f64::INFINITY, 4).is_nan());
        assert!(tick_values(Domain::new(0.0, 1.0).unwrap(), f64::NAN).is_empty());
        // A step far too small for the span stops at the iteration bound
        // instead of trying to enumerate the domain.
        assert_eq!(tick_values(Domain::new(0.0, 1e30).unwrap(), 1.0).len(), 64);
    }
}
