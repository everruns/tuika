//! Adaptive charts for [`tuika`].
//!
//! [`Chart`] accepts one renderer-independent chart model. On a terminal with
//! a graphics protocol it rasterizes smooth lines and filled bars into an image;
//! everywhere else it renders the same axes, series, domains, and legend with
//! terminal cells, using dense 2×2 quadrant glyphs for connected geometry and
//! Braille subcells for scatter points. [`tuika::Runner`] supplies terminal
//! graphics automatically; custom hosts can provide
//! [`tuika::term::image::ImageSupport`] and an
//! [`tuika::term::image::ImageLayer`] through [`tuika::RenderCtx`].

use tuika::term::image::{ImageLayer, ImageSupport};
use tuika::ui::Rect;
use tuika::ui::{Color, Style};
use tuika::{RenderCtx, Size, Surface, View};

mod axis;
mod cells;
mod graphics;
mod model;
mod plan;

pub use axis::Axis;
use axis::AxisLayout;
use cells::{render_cells, render_legend};
use graphics::render_pixels;
use model::PlotModel;
pub use model::{Domain, Point, Series, SeriesKind};
use plan::ChartPlan;
pub use plan::Stack;

/// Which rendering path a chart used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderMode {
    /// High-resolution image emitted through a graphics protocol.
    Graphics,
    /// Unicode cell renderer, available in every terminal.
    Cells,
}

/// An adaptive line, bar, area, scatter, step, or donut chart view.
///
/// Bar and area series can be grouped or stacked, the axes can be labelled with
/// categories and swapped into a horizontal layout, and one position can be
/// focused to list every series' value there.
///
/// | Portable cells | Terminal graphics |
/// | --- | --- |
/// | <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/line-cells.png" alt="Line chart rendered with terminal cells" width="420"> | <img src="https://raw.githubusercontent.com/everruns/tuika/main/docs/charts/line-graphics.png" alt="Line chart rendered with terminal graphics" width="420"> |
pub struct Chart {
    title: String,
    series: Vec<Series>,
    x_domain: Option<Domain>,
    y_domain: Option<Domain>,
    x_axis: Axis,
    y_axis: Axis,
    stack: Stack,
    horizontal: bool,
    focus: Option<f64>,
    center: String,
    support: Option<ImageSupport>,
    layer: Option<ImageLayer>,
    legend: bool,
}

impl Chart {
    /// Construct an empty chart.
    pub fn new() -> Self {
        Self {
            title: String::new(),
            series: Vec::new(),
            x_domain: None,
            y_domain: None,
            x_axis: Axis::new(),
            y_axis: Axis::new(),
            stack: Stack::None,
            horizontal: false,
            focus: None,
            center: String::new(),
            support: None,
            layer: None,
            legend: true,
        }
    }

    /// Set the title rendered above the plot.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Append a series.
    pub fn series(mut self, series: Series) -> Self {
        self.series.push(series);
        self
    }

    /// Use an explicit horizontal domain.
    pub fn x_domain(mut self, domain: Domain) -> Self {
        self.x_domain = Some(domain);
        self
    }

    /// Use an explicit vertical domain.
    pub fn y_domain(mut self, domain: Domain) -> Self {
        self.y_domain = Some(domain);
        self
    }

    /// Configure the horizontal axis. Tick labels are shown by default.
    pub fn x_axis(mut self, axis: Axis) -> Self {
        self.x_axis = axis;
        self
    }

    /// Configure the vertical axis. Tick labels are shown by default.
    pub fn y_axis(mut self, axis: Axis) -> Self {
        self.y_axis = axis;
        self
    }

    /// Combine overlapping bar and area series instead of drawing each against
    /// the baseline. Unstacked bars sit side by side within their category.
    pub fn stack(mut self, stack: Stack) -> Self {
        self.stack = stack;
        self
    }

    /// Lay the chart out with categories down the side and values across.
    ///
    /// A terminal gives a category a whole row of width this way, instead of
    /// the single column a vertical bar leaves for its name, so long labels
    /// stay readable where they would otherwise be thinned away.
    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        self
    }

    /// Mark one position as focused: a rule is drawn through it and each
    /// series' value there is listed beneath the plot.
    ///
    /// This is the terminal's answer to a hover tooltip. The chart stays a pure
    /// view — the host owns the index and moves it in response to keys.
    pub fn focus(mut self, position: f64) -> Self {
        self.focus = Some(position);
        self
    }

    /// Text drawn in the middle of a donut, usually the total.
    pub fn center(mut self, text: impl Into<String>) -> Self {
        self.center = text.into();
        self
    }

    /// Show or hide the legend. It is shown by default.
    pub fn legend(mut self, visible: bool) -> Self {
        self.legend = visible;
        self
    }

    /// Override terminal graphics support supplied by the render context.
    pub fn support(mut self, support: ImageSupport) -> Self {
        self.support = Some(support);
        self
    }

    /// Override the image layer supplied by the render context.
    pub fn in_layer(mut self, layer: &ImageLayer) -> Self {
        self.layer = Some(layer.clone());
        self
    }

    /// Resolve the rendering path from explicit chart configuration only.
    pub fn render_mode(&self) -> RenderMode {
        if self.layer.is_some() && self.support.unwrap_or(ImageSupport::None) != ImageSupport::None
        {
            RenderMode::Graphics
        } else {
            RenderMode::Cells
        }
    }

    /// Resolve the rendering path including graphics supplied by the host.
    pub fn render_mode_in(&self, ctx: &RenderCtx<'_>) -> RenderMode {
        let (support, layer) = self.graphics(ctx);
        if layer.is_some() && support != ImageSupport::None {
            RenderMode::Graphics
        } else {
            RenderMode::Cells
        }
    }

    fn graphics<'a>(&'a self, ctx: &'a RenderCtx<'_>) -> (ImageSupport, Option<&'a ImageLayer>) {
        let inherited = ctx.image_graphics();
        let support = self
            .support
            .or_else(|| inherited.map(|(support, _)| support))
            .unwrap_or(ImageSupport::None);
        let layer = self
            .layer
            .as_ref()
            .or_else(|| inherited.map(|(_, layer)| layer));
        (support, layer)
    }
}

impl Default for Chart {
    fn default() -> Self {
        Self::new()
    }
}

impl View for Chart {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        Size::new(available.width.min(80), available.height.min(24))
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.is_empty() {
            return;
        }
        let Some(plan) = ChartPlan::new(self, ctx) else {
            render_empty(area, surface, ctx, &self.title);
            return;
        };
        surface.fill(Style::default().bg(ctx.theme.background));
        if !self.title.is_empty() {
            surface.set_string(area.x, area.y, &self.title, ctx.theme.accent_style());
        }
        render_legend(area, surface, self, ctx);

        // A ring has no axes to label and no cell/pixel plot rect to share, so
        // it takes the whole body and returns.
        if plan.radial {
            let body = body_rect(area, self);
            let (support, layer) = self.graphics(ctx);
            if self.render_mode_in(ctx) == RenderMode::Graphics {
                let data = graphics::render_donut_pixels(body.width, body.height, &plan);
                if let (Some(data), Some(layer)) = (data, layer) {
                    tuika::components::Image::new(data, body.width, body.height)
                        .support(support)
                        .in_layer(layer)
                        .render(body, &mut surface.child(body), ctx);
                    cells::render_center(body, surface, self, ctx);
                    return;
                }
            }
            cells::render_donut(body, surface, self, &plan, ctx);
            return;
        }

        let model = plan.model;
        let mut layout = AxisLayout::new(area, self, &model);
        let plot = plot_rect(area, self, &layout);
        layout.resolve(plot, area, self, &model);
        // Labels are cells in both modes: the graphics image covers the plot
        // only, so the same text lands beside it either way.
        layout.render(plot, surface, ctx);
        render_readout(area, surface, self, &plan, ctx);

        let (support, layer) = self.graphics(ctx);
        if self.render_mode_in(ctx) == RenderMode::Graphics {
            let data = render_pixels(plot.width, plot.height, self, &plan, &layout, ctx);
            if let (Some(data), Some(layer)) = (data, layer) {
                // Image owns capability-gated placement; using it here keeps the
                // same protocol lifecycle and fallback semantics as core images.
                tuika::components::Image::new(data, plot.width, plot.height)
                    .support(support)
                    .in_layer(layer)
                    .render(plot, &mut surface.child(plot), ctx);
                // Value labels and the focus rule are text and thin rules, so
                // they stay cells over the image for the same reason the tick
                // labels do.
                cells::render_overlays(plot, surface, self, &plan, ctx, false);
                return;
            }
        }
        render_cells(plot, surface, self, &plan, &layout, ctx);
    }
}

/// The area left for a chart body once the title and legend have taken theirs.
fn body_rect(area: Rect, chart: &Chart) -> Rect {
    let title = u16::from(!chart.title.is_empty());
    let legend = u16::from(chart.legend && !chart.series.is_empty());
    Rect::new(
        area.x,
        area.y.saturating_add(title),
        area.width,
        area.height.saturating_sub(title + legend),
    )
}

/// One line beneath the plot naming the focused position and every series'
/// value there — the terminal's stand-in for a hover tooltip.
fn render_readout(
    area: Rect,
    surface: &mut Surface,
    chart: &Chart,
    plan: &ChartPlan,
    ctx: &RenderCtx,
) {
    let Some(focus) = chart.focus else {
        return;
    };
    let legend = u16::from(chart.legend && !chart.series.is_empty());
    if area.height < legend + 3 {
        return;
    }
    // The row plot_rect reserved for it, below the x tick labels.
    let row = area.bottom() - legend - 1;
    let mut x = area.x;
    x = surface.set_string(x, row, &focus_label(chart, focus), ctx.theme.accent_style());
    for mark in &plan.marks {
        if x >= area.right() {
            break;
        }
        let Some(sample) = sample_at(&mark.samples, focus) else {
            continue;
        };
        x = surface.set_string(x, row, "  ", ctx.theme.muted_style());
        surface.set(x, row, '▪', Style::default().fg(mark.color));
        x = x.saturating_add(2);
        x = surface.set_string(
            x,
            row,
            &format!("{} {}", mark.name, trim_number(sample.y)),
            ctx.theme.muted_style(),
        );
    }
}

fn focus_label(chart: &Chart, focus: f64) -> String {
    chart
        .x_axis
        .category_name(focus)
        .unwrap_or_else(|| trim_number(focus))
}

/// The sample a series carries at `position`, if it has one there.
fn sample_at(samples: &[Point], position: f64) -> Option<Point> {
    samples
        .iter()
        .copied()
        .find(|point| (point.x - position).abs() < f64::EPSILON.max(position.abs() * 1e-9))
}

/// Print a value without trailing zeros, so a readout stays narrow.
fn trim_number(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        format!("{value:.0}")
    } else {
        let text = format!("{value:.2}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn render_empty(area: Rect, surface: &mut Surface, ctx: &RenderCtx, title: &str) {
    surface.fill(Style::default().bg(ctx.theme.background));
    if !title.is_empty() {
        surface.set_string(area.x, area.y, title, ctx.theme.accent_style());
    }
    let y = area.y + usize::from(!title.is_empty()) as u16;
    if y < area.bottom() {
        surface.set_string(area.x, y, "No chart data", ctx.theme.muted_style());
    }
}

fn chart_color(series: &Series, index: usize, ctx: &RenderCtx) -> Color {
    series.color.unwrap_or(match index % 4 {
        0 => ctx.theme.accent,
        1 => ctx.theme.accent_alt,
        2 => ctx.theme.code.link,
        _ => ctx.theme.code.string,
    })
}

/// The plot rect: the axis column and every data cell, but no chrome.
///
/// The trailing row it always leaves below the plot is where x tick labels go,
/// so labelling that axis costs no plot height.
fn plot_rect(area: Rect, chart: &Chart, layout: &AxisLayout) -> Rect {
    let title = u16::from(!chart.title.is_empty());
    let legend = u16::from(chart.legend && !chart.series.is_empty());
    let readout = u16::from(chart.focus.is_some());
    Rect::new(
        area.x.saturating_add(layout.gutter),
        area.y.saturating_add(title),
        area.width.saturating_sub(layout.gutter + 1),
        area.height.saturating_sub(title + legend + readout + 1),
    )
}

fn draw_line(mut from: (i32, i32), to: (i32, i32), mut draw: impl FnMut(i32, i32)) {
    let dx = (to.0 - from.0).abs();
    let sx = if from.0 < to.0 { 1 } else { -1 };
    let dy = -(to.1 - from.1).abs();
    let sy = if from.1 < to.1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        draw(from.0, from.1);
        if from == to {
            break;
        }
        let twice = error * 2;
        if twice >= dy {
            error += dy;
            from.0 += sx;
        }
        if twice <= dx {
            error += dx;
            from.1 += sy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cells::{BrailleGrid, QuadrantGrid, draw_quadrant_band};
    use tuika::Theme;
    use tuika::ui::Buffer;

    fn is_braille(ch: char) -> bool {
        ('\u{2801}'..='\u{28ff}').contains(&ch)
    }

    fn is_quadrant(ch: char) -> bool {
        tuika::framebuffer::QUADRANTS[1..].contains(&ch)
    }

    pub(super) fn grid_of(buffer: &Buffer, area: Rect) -> String {
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The column the vertical axis line occupies, i.e. the gutter width.
    pub(super) fn axis_column(buffer: &Buffer, area: Rect) -> Option<u16> {
        (0..area.width).find(|&x| (0..area.height).any(|y| buffer[(x, y)].symbol() == "│"))
    }

    /// The resolved plan, which is what decides domains now that geometry —
    /// bar widths, stacked tops — is what the plot has to contain.
    pub(super) fn plan_of(chart: &Chart) -> ChartPlan {
        ChartPlan::new(chart, &RenderCtx::new(&Theme::default())).expect("a plan")
    }

    pub(super) fn render_to(chart: &Chart, width: u16, height: u16) -> (Buffer, Rect) {
        let area = Rect::new(0, 0, width, height);
        let mut buffer = Buffer::empty(area);
        chart.render(
            area,
            &mut Surface::new(&mut buffer, area),
            &RenderCtx::new(&Theme::default()),
        );
        (buffer, area)
    }

    fn sample() -> Chart {
        Chart::new()
            .title("Traffic")
            .series(Series::line(
                "requests",
                [
                    Point::new(0.0, 2.0),
                    Point::new(1.0, 5.0),
                    Point::new(2.0, 3.0),
                ],
            ))
            .series(Series::bar(
                "errors",
                [
                    Point::new(0.0, 1.0),
                    Point::new(1.0, 2.0),
                    Point::new(2.0, 1.0),
                ],
            ))
    }

    #[test]
    fn domain_rejects_degenerate_and_non_finite_ranges() {
        assert_eq!(Domain::new(0.0, 1.0), Some(Domain { min: 0.0, max: 1.0 }));
        assert_eq!(Domain::new(1.0, 1.0), None);
        assert_eq!(Domain::new(f64::NAN, 1.0), None);
    }

    #[test]
    fn graphics_requires_capability_and_layer() {
        let layer = ImageLayer::new();
        assert_eq!(
            sample().support(ImageSupport::Kitty).render_mode(),
            RenderMode::Cells
        );
        assert_eq!(sample().in_layer(&layer).render_mode(), RenderMode::Cells);
        assert_eq!(
            sample()
                .support(ImageSupport::Kitty)
                .in_layer(&layer)
                .render_mode(),
            RenderMode::Graphics
        );
    }

    #[test]
    fn chart_uses_graphics_from_the_render_context() {
        let theme = Theme::default();
        let layer = ImageLayer::new();
        let ctx = RenderCtx::new(&theme).with_image_graphics(ImageSupport::Kitty, &layer);
        let chart = sample();

        assert_eq!(chart.render_mode(), RenderMode::Cells);
        assert_eq!(chart.render_mode_in(&ctx), RenderMode::Graphics);
        tuika::testing::render_with_context(&chart, 40, 12, &ctx);
        assert_eq!(layer.len(), 1);
    }

    #[test]
    fn cell_renderer_draws_title_axes_series_and_legend() {
        let area = Rect::new(0, 0, 32, 10);
        let mut buffer = Buffer::empty(area);
        sample().render(
            area,
            &mut Surface::new(&mut buffer, area),
            &RenderCtx::new(&Theme::default()),
        );
        let grid = grid_of(&buffer, area);
        assert!(grid.contains("Traffic"));
        assert!(grid.contains('└'));
        assert!(grid.chars().any(is_quadrant));
        assert!(grid.contains('█'));
        assert!(grid.contains("requests"));
    }

    #[test]
    fn cell_lines_use_dense_quadrant_subcells() {
        let area = Rect::new(0, 0, 10, 6);
        let mut buffer = Buffer::empty(area);
        Chart::new()
            .legend(false)
            .x_domain(Domain::new(0.0, 1.0).unwrap())
            .y_domain(Domain::new(0.0, 1.0).unwrap())
            .series(Series::line(
                "line",
                [Point::new(0.0, 0.0), Point::new(1.0, 1.0)],
            ))
            .render(
                area,
                &mut Surface::new(&mut buffer, area),
                &RenderCtx::new(&Theme::default()),
            );

        assert!(
            buffer
                .content
                .iter()
                .any(|cell| cell.symbol().chars().next().is_some_and(is_quadrant)),
            "portable lines should use dense quadrant subcells"
        );
        assert!(
            !buffer
                .content
                .iter()
                .any(|cell| cell.symbol().chars().next().is_some_and(is_braille)),
            "connected lines should not expose separated Braille dots"
        );
    }

    #[test]
    fn cell_renderer_uses_documented_theme_slots() {
        let area = Rect::new(0, 0, 32, 10);
        let theme = Theme::default();
        let mut buffer = Buffer::empty(area);
        sample().render(
            area,
            &mut Surface::new(&mut buffer, area),
            &RenderCtx::new(&theme),
        );

        assert_eq!(buffer[(0, 0)].fg, theme.accent, "title uses accent");
        let axis = axis_column(&buffer, area).expect("axis column");
        assert_eq!(buffer[(axis, 1)].fg, theme.border, "axis uses border");
        let (label_x, label_y) = (0..area.height)
            .flat_map(|y| (0..axis).map(move |x| (x, y)))
            .find(|&(x, y)| {
                buffer[(x, y)]
                    .symbol()
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_digit())
            })
            .expect("a y tick label");
        assert_eq!(
            buffer[(label_x, label_y)].fg,
            theme.muted,
            "tick labels use muted text"
        );
        assert_eq!(
            buffer[(0, 9)].fg,
            theme.accent,
            "first legend mark uses series color"
        );
        assert_eq!(
            buffer[(2, 9)].fg,
            theme.muted,
            "legend label uses muted text"
        );
    }

    #[test]
    fn additional_cell_series_have_distinct_marks() {
        let cases = [
            (
                Series::area("area", [Point::new(0.0, 1.0), Point::new(1.0, 3.0)]),
                false,
            ),
            (
                Series::scatter("scatter", [Point::new(0.0, 1.0), Point::new(1.0, 3.0)]),
                true,
            ),
        ];
        for (series, expected_braille) in cases {
            let area = Rect::new(0, 0, 24, 8);
            let mut buffer = Buffer::empty(area);
            Chart::new().series(series).render(
                area,
                &mut Surface::new(&mut buffer, area),
                &RenderCtx::new(&Theme::default()),
            );
            let has_braille = buffer
                .content
                .iter()
                .any(|cell| cell.symbol().chars().next().is_some_and(is_braille));
            assert_eq!(has_braille, expected_braille);
            assert!(
                buffer
                    .content
                    .iter()
                    .any(|cell| cell
                        .symbol()
                        .chars()
                        .next()
                        .is_some_and(if expected_braille {
                            is_braille
                        } else {
                            is_quadrant
                        }))
            );
        }
    }

    #[test]
    fn a_band_fills_down_to_its_floor_subcell() {
        let mut grid = QuadrantGrid::new(1, 2);
        draw_quadrant_band(&mut grid, &[(0, 1), (1, 1)], &[(0, 3), (1, 3)]);
        assert_eq!(
            grid.masks,
            [0b1100, 0b1111],
            "fill should include every subcell below the edge and none above it"
        );
    }

    #[test]
    fn braille_grid_packs_all_eight_subcells() {
        let mut grid = BrailleGrid::new(1, 1);
        for y in 0..4 {
            for x in 0..2 {
                grid.set(x, y);
            }
        }
        assert_eq!(grid.masks, [u8::MAX]);
    }

    #[test]
    fn quadrant_grid_packs_all_four_subcells() {
        let mut grid = QuadrantGrid::new(1, 1);
        for y in 0..2 {
            for x in 0..2 {
                grid.set(x, y);
            }
        }
        assert_eq!(grid.masks, [0b1111]);
    }

    #[test]
    fn area_fills_columns_between_sparse_points() {
        let area = Rect::new(0, 0, 16, 8);
        let mut buffer = Buffer::empty(area);
        Chart::new()
            .legend(false)
            .series(Series::area(
                "area",
                [Point::new(0.0, 1.0), Point::new(10.0, 3.0)],
            ))
            .render(
                area,
                &mut Surface::new(&mut buffer, area),
                &RenderCtx::new(&Theme::default()),
            );
        assert!(
            (5..10).any(|x| {
                (0..area.height).any(|y| {
                    buffer[(x, y)]
                        .symbol()
                        .chars()
                        .next()
                        .is_some_and(is_quadrant)
                })
            }),
            "area must fill between data points rather than draw isolated columns"
        );
    }

    #[test]
    fn step_series_uses_horizontal_then_vertical_segments() {
        let area = Rect::new(0, 0, 12, 7);
        let mut buffer = Buffer::empty(area);
        Chart::new()
            .legend(false)
            .x_domain(Domain::new(0.0, 1.0).unwrap())
            .y_domain(Domain::new(0.0, 1.0).unwrap())
            .series(Series::step(
                "step",
                [Point::new(0.0, 0.0), Point::new(1.0, 1.0)],
            ))
            .render(
                area,
                &mut Surface::new(&mut buffer, area),
                &RenderCtx::new(&Theme::default()),
            );
        let grid = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(
                "
",
            );
        assert!(
            grid.lines()
                .any(|line| line.chars().filter(|&ch| is_quadrant(ch)).count() >= 5)
        );
    }

    #[test]
    fn every_series_kind_records_graphics() {
        let points = [Point::new(0.0, 1.0), Point::new(1.0, 3.0)];
        let series = [
            Series::line("line", points),
            Series::bar("bar", points),
            Series::area("area", points),
            Series::scatter("scatter", points),
            Series::step("step", points),
        ];
        for series in series {
            let area = Rect::new(0, 0, 20, 8);
            let mut buffer = Buffer::empty(area);
            let layer = ImageLayer::new();
            Chart::new()
                .series(series)
                .support(ImageSupport::Kitty)
                .in_layer(&layer)
                .render(
                    area,
                    &mut Surface::new(&mut buffer, area),
                    &RenderCtx::new(&Theme::default()),
                );
            assert_eq!(layer.len(), 1);
        }
    }

    #[test]
    fn graphics_renderer_records_plot_while_title_and_legend_stay_cells() {
        let area = Rect::new(0, 0, 20, 8);
        let mut buffer = Buffer::empty(area);
        let layer = ImageLayer::new();
        sample()
            .support(ImageSupport::Kitty)
            .in_layer(&layer)
            .render(
                area,
                &mut Surface::new(&mut buffer, area),
                &RenderCtx::new(&Theme::default()),
            );
        assert_eq!(layer.len(), 1);
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Traffic"));
        assert!(text.contains("requests"));
    }

    #[test]
    fn empty_and_non_finite_data_render_a_safe_message() {
        let area = Rect::new(0, 0, 20, 3);
        let mut buffer = Buffer::empty(area);
        Chart::new()
            .title("Empty")
            .series(Series::line("bad", [Point::new(f64::NAN, 1.0)]))
            .render(
                area,
                &mut Surface::new(&mut buffer, area),
                &RenderCtx::new(&Theme::default()),
            );
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("No chart data"));
    }

    #[test]
    fn explicit_domains_clip_extreme_values_to_plot_bounds() {
        let chart = Chart::new()
            .x_domain(Domain::new(0.0, 1.0).unwrap())
            .y_domain(Domain::new(0.0, 1.0).unwrap())
            .series(Series::line(
                "extreme",
                [
                    Point::new(-f64::MAX, f64::MAX),
                    Point::new(f64::MAX, -f64::MAX),
                ],
            ));
        let model = plan_of(&chart).model;
        assert_eq!(model.map(chart.series[0].points[0], 20, 10), (0, 0));
        assert_eq!(model.map(chart.series[0].points[1], 20, 10), (19, 9));
    }

    #[test]
    fn tiny_sizes_are_safe_in_both_renderers() {
        for width in 0..=3 {
            for height in 0..=3 {
                let area = Rect::new(0, 0, width, height);
                let mut buffer = Buffer::empty(area);
                sample().render(
                    area,
                    &mut Surface::new(&mut buffer, area),
                    &RenderCtx::new(&Theme::default()),
                );

                let layer = ImageLayer::new();
                sample()
                    .support(ImageSupport::Kitty)
                    .in_layer(&layer)
                    .render(
                        area,
                        &mut Surface::new(&mut buffer, area),
                        &RenderCtx::new(&Theme::default()),
                    );
            }
        }
    }

    #[test]
    fn constant_series_gets_a_nonzero_automatic_domain() {
        let chart = Chart::new().series(Series::line("flat", [Point::new(1.0, 2.0)]));
        let model = plan_of(&chart).model;
        assert!(model.x.min < model.x.max);
        assert!(model.y.min < model.y.max);
    }

    #[test]
    fn automatic_bar_domain_reserves_half_a_band_at_each_edge() {
        let chart = Chart::new().series(Series::bar(
            "bars",
            [
                Point::new(0.0, 1.0),
                Point::new(1.0, 2.0),
                Point::new(2.0, 3.0),
            ],
        ));
        let model = plan_of(&chart).model;
        assert_eq!(
            model.x,
            Domain {
                min: -0.5,
                max: 2.5
            }
        );

        let explicit = chart.x_domain(Domain::new(0.0, 2.0).unwrap());
        assert_eq!(
            plan_of(&explicit).model.x,
            Domain { min: 0.0, max: 2.0 },
            "explicit bounds remain exact clipping bounds"
        );
    }
}

#[cfg(test)]
mod axis_tests {
    use super::tests::{axis_column, grid_of, render_to};
    use super::*;

    fn labelled() -> Chart {
        Chart::new().legend(false).series(Series::line(
            "latency",
            (0..=10).map(|i| Point::new(f64::from(i), f64::from(i) * 10.0)),
        ))
    }

    #[test]
    fn both_axes_are_labelled_by_default() {
        let (buffer, area) = render_to(&labelled(), 40, 12);
        let grid = grid_of(&buffer, area);

        let gutter = axis_column(&buffer, area).expect("axis column");
        assert!(gutter > 1, "y labels claim a gutter, got {gutter}");
        assert!(grid.contains('┤'), "y ticks mark the vertical axis");
        assert!(grid.contains('┬'), "x ticks mark the horizontal axis");
        assert!(
            grid.lines().last().is_some_and(|row| row.contains('0')),
            "x labels sit on the margin row below the plot"
        );
    }

    #[test]
    fn hidden_axes_give_their_space_back_to_the_plot() {
        let bare = labelled().x_axis(Axis::hidden()).y_axis(Axis::hidden());
        let (plain, area) = render_to(&bare, 40, 12);
        let (labelled, _) = render_to(&labelled(), 40, 12);

        assert_eq!(
            axis_column(&plain, area),
            Some(1),
            "no gutter without y labels"
        );
        assert!(axis_column(&labelled, area).is_some_and(|gutter| gutter > 1));
        let grid = grid_of(&plain, area);
        assert!(!grid.contains('┤') && !grid.contains('┬'));
        assert!(
            !grid.chars().any(|ch| ch.is_ascii_digit()),
            "a hidden axis prints no tick labels"
        );
    }

    #[test]
    fn ticks_land_on_round_values() {
        let chart = labelled()
            .y_domain(Domain::new(0.0, 100.0).unwrap())
            .y_axis(Axis::new().ticks(4));
        let (buffer, area) = render_to(&chart, 40, 14);
        let gutter = axis_column(&buffer, area).expect("axis column");
        let labels = (0..area.height)
            .map(|y| {
                (0..gutter)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim()
                    .to_string()
            })
            .filter(|label| !label.is_empty())
            .collect::<Vec<_>>();
        assert_eq!(labels, ["100", "75", "50", "25", "0"]);
    }

    #[test]
    fn the_format_hook_replaces_the_numeric_label() {
        let chart = labelled().y_axis(Axis::new().ticks(2).format(|value| format!("{value:.0}ms")));
        let (buffer, area) = render_to(&chart, 40, 12);
        assert!(grid_of(&buffer, area).contains("ms"));
    }

    #[test]
    fn crowded_x_labels_are_thinned_rather_than_overlapped() {
        let chart = Chart::new().legend(false).series(Series::line(
            "wide",
            (0..=50).map(|i| Point::new(f64::from(i) * 1000.0, f64::from(i))),
        ));
        let (buffer, area) = render_to(&chart, 30, 10);
        let row = grid_of(&buffer, area)
            .lines()
            .last()
            .expect("label row")
            .to_string();

        // Every label is separated by whitespace: none was written over another.
        let labels = row.split_whitespace().collect::<Vec<_>>();
        assert!(!labels.is_empty(), "at least one x label survives");
        for label in labels {
            assert!(label.chars().all(|ch| ch.is_ascii_digit()), "{label:?}");
        }
    }

    #[test]
    fn labels_are_dropped_before_the_plot_is_crowded_out() {
        let chart = Chart::new().legend(false).series(Series::line(
            "huge",
            [Point::new(0.0, 0.0), Point::new(1.0, 123_456_789.0)],
        ));
        for width in 1..=14 {
            let (buffer, area) = render_to(&chart, width, 8);
            // Below a handful of columns there is no plot at all, which is the
            // pre-existing degenerate case rather than anything labels caused.
            let Some(gutter) = axis_column(&buffer, area) else {
                continue;
            };
            assert!(
                gutter <= width / 2,
                "gutter {gutter} must not swallow a {width}-wide chart"
            );
        }
    }

    #[test]
    fn both_render_modes_place_labels_in_the_same_cells() {
        let chart = labelled();
        let (cells, area) = render_to(&chart, 36, 12);

        let layer = ImageLayer::new();
        let mut graphics = tuika::ui::Buffer::empty(area);
        chart.support(ImageSupport::Kitty).in_layer(&layer).render(
            area,
            &mut Surface::new(&mut graphics, area),
            &RenderCtx::new(&tuika::Theme::default()),
        );

        let gutter = axis_column(&cells, area).expect("axis column");
        for y in 0..area.height {
            for x in 0..gutter {
                assert_eq!(
                    cells[(x, y)].symbol(),
                    graphics[(x, y)].symbol(),
                    "gutter cell ({x}, {y}) differs between render modes"
                );
            }
        }
    }

    #[test]
    fn tiny_areas_stay_safe_with_labels_enabled() {
        for width in 0..=12 {
            for height in 0..=12 {
                render_to(&labelled(), width, height);
            }
        }
    }
}

#[cfg(test)]
mod baseline_tests {
    use super::tests::plan_of;
    use super::*;

    fn domain_of(chart: &Chart) -> Domain {
        plan_of(chart).model.y
    }

    fn points(values: [f64; 3]) -> [Point; 3] {
        [
            Point::new(0.0, values[0]),
            Point::new(1.0, values[1]),
            Point::new(2.0, values[2]),
        ]
    }

    #[test]
    fn bars_and_areas_reach_the_zero_baseline() {
        for series in [
            Series::bar("bars", points([1.0, 4.0, 5.0])),
            Series::area("area", points([1.0, 4.0, 5.0])),
        ] {
            let domain = domain_of(&Chart::new().series(series));
            assert_eq!(domain, Domain { min: 0.0, max: 5.0 });
        }
    }

    #[test]
    fn positional_series_keep_the_tighter_domain() {
        for series in [
            Series::line("line", points([12.0, 18.0, 16.0])),
            Series::step("step", points([12.0, 18.0, 16.0])),
            Series::scatter("scatter", points([12.0, 18.0, 16.0])),
        ] {
            let domain = domain_of(&Chart::new().series(series));
            assert_eq!(
                domain,
                Domain {
                    min: 12.0,
                    max: 18.0
                },
                "a positional series should not be flattened toward zero"
            );
        }
    }

    #[test]
    fn one_baselined_series_anchors_the_whole_chart() {
        // The shared domain has to satisfy the bar, or the bar would lie.
        let chart = Chart::new()
            .series(Series::line("line", points([12.0, 18.0, 16.0])))
            .series(Series::bar("bars", points([2.0, 5.0, 3.0])));
        assert_eq!(
            domain_of(&chart),
            Domain {
                min: 0.0,
                max: 18.0
            }
        );
    }

    #[test]
    fn negative_values_extend_to_zero_from_the_other_side() {
        let chart = Chart::new().series(Series::bar("debt", points([-5.0, -2.0, -1.0])));
        assert_eq!(
            domain_of(&chart),
            Domain {
                min: -5.0,
                max: 0.0
            }
        );

        let straddling = Chart::new().series(Series::bar("delta", points([-3.0, 2.0, 4.0])));
        assert_eq!(
            domain_of(&straddling),
            Domain {
                min: -3.0,
                max: 4.0
            },
            "a domain already containing zero is unchanged"
        );
    }

    #[test]
    fn a_constant_bar_series_still_gets_a_usable_domain() {
        let chart = Chart::new().series(Series::bar("flat", points([4.0, 4.0, 4.0])));
        let domain = domain_of(&chart);
        assert_eq!(domain.min, 0.0);
        // The tallest bar reaches the top of the plot, which is where the
        // top tick label puts its value too.
        assert_eq!(domain.max, 4.0);

        let zeroed = Chart::new().series(Series::bar("zero", points([0.0, 0.0, 0.0])));
        let domain = domain_of(&zeroed);
        assert!(domain.min < domain.max, "a degenerate domain stays valid");
    }

    #[test]
    fn an_explicit_domain_is_never_widened() {
        let chart = Chart::new()
            .series(Series::bar("bars", points([12.0, 18.0, 16.0])))
            .y_domain(Domain::new(10.0, 20.0).unwrap());
        assert_eq!(
            domain_of(&chart),
            Domain {
                min: 10.0,
                max: 20.0
            }
        );
    }

    #[test]
    fn bars_render_proportional_heights_from_the_baseline() {
        let area = Rect::new(0, 0, 30, 12);
        let mut buffer = tuika::ui::Buffer::empty(area);
        Chart::new()
            .legend(false)
            .series(Series::bar("bars", points([1.0, 2.0, 4.0])))
            .render(
                area,
                &mut Surface::new(&mut buffer, area),
                &RenderCtx::new(&tuika::Theme::default()),
            );

        // Bars occupy a slice of their band, so compare runs of filled columns
        // rather than individual columns.
        let columns: Vec<usize> = (0..area.width)
            .map(|x| {
                (0..area.height)
                    .filter(|&y| buffer[(x, y)].symbol() == "█")
                    .count()
            })
            .collect();
        let mut heights: Vec<usize> = Vec::new();
        let mut run: Option<usize> = None;
        for count in columns {
            if count == 0 {
                heights.extend(run.take());
            } else {
                run = Some(run.unwrap_or(0).max(count));
            }
        }
        heights.extend(run);
        assert_eq!(heights.len(), 3, "one run per bar: {heights:?}");
        assert!(heights[0] >= 2, "the smallest bar is visible: {heights:?}");
        assert!(
            heights[0] < heights[1] && heights[1] < heights[2],
            "{heights:?}"
        );
    }
}

#[cfg(test)]
mod grammar_tests {
    use super::tests::{axis_column, grid_of, plan_of, render_to};
    use super::*;
    use crate::plan::Geometry;

    fn at(values: [f64; 3]) -> Vec<Point> {
        values
            .iter()
            .enumerate()
            .map(|(index, &value)| Point::new(index as f64, value))
            .collect()
    }

    fn months() -> Axis {
        Axis::new().categories(["Jan", "Feb", "Mar"])
    }

    fn bars(chart: &Chart, index: usize) -> Vec<crate::plan::BarRect> {
        match &plan_of(chart).marks[index].geometry {
            Geometry::Bars { bars, .. } => bars.clone(),
            other => panic!("expected bars, got {other:?}"),
        }
    }

    #[test]
    fn a_categorical_axis_names_positions() {
        let chart = Chart::new()
            .legend(false)
            .x_axis(months())
            .series(Series::bar("visits", at([3.0, 5.0, 4.0])));
        let (buffer, area) = render_to(&chart, 44, 10);
        let grid = grid_of(&buffer, area);
        for name in ["Jan", "Feb", "Mar"] {
            assert!(grid.contains(name), "missing {name} in\n{grid}");
        }
    }

    #[test]
    fn unstacked_bars_share_the_band_without_touching() {
        let chart = Chart::new()
            .x_axis(months())
            .series(Series::bar("a", at([3.0, 5.0, 4.0])))
            .series(Series::bar("b", at([2.0, 1.0, 3.0])));
        let (first, second) = (bars(&chart, 0), bars(&chart, 1));

        // Two slots inside one band, in series order, with a gutter left over.
        assert!(first[0].x1 <= second[0].x0, "{first:?} {second:?}");
        assert!(first[0].x0 > -0.5 && second[0].x1 < 0.5, "inside the band");
        for bar in first.iter().chain(second.iter()) {
            assert_eq!(bar.y0, 0.0, "an unstacked bar starts at the baseline");
        }
    }

    #[test]
    fn stacked_bars_rest_on_the_series_below() {
        let chart = Chart::new()
            .x_axis(months())
            .stack(Stack::Normal)
            .series(Series::bar("a", at([3.0, 5.0, 4.0])))
            .series(Series::bar("b", at([2.0, 1.0, 3.0])));
        let (first, second) = (bars(&chart, 0), bars(&chart, 1));

        assert_eq!((first[0].y0, first[0].y1), (0.0, 3.0));
        assert_eq!((second[0].y0, second[0].y1), (3.0, 5.0));
        assert_eq!(
            (first[0].x0, first[0].x1),
            (second[0].x0, second[0].x1),
            "stacked bars occupy the same slot rather than sitting side by side"
        );
        // Totals are 5, 6 and 7; the domain has to hold the tallest stack.
        assert_eq!(plan_of(&chart).model.y.max, 7.0);
    }

    #[test]
    fn percent_stacking_scales_every_position_to_a_hundred() {
        let chart = Chart::new()
            .stack(Stack::Percent)
            .series(Series::bar("a", at([3.0, 5.0, 1.0])))
            .series(Series::bar("b", at([1.0, 5.0, 3.0])));
        let (first, second) = (bars(&chart, 0), bars(&chart, 1));
        for (index, bar) in second.iter().enumerate() {
            assert!(
                (bar.y1 - 100.0).abs() < 1e-9,
                "position {index} sums to {}",
                bar.y1
            );
        }
        assert!((first[0].y1 - 75.0).abs() < 1e-9, "{first:?}");
        assert!((first[1].y1 - 50.0).abs() < 1e-9, "{first:?}");
    }

    #[test]
    fn a_position_with_nothing_in_it_has_no_percentage() {
        let chart = Chart::new().stack(Stack::Percent).series(Series::bar(
            "a",
            [Point::new(0.0, 0.0), Point::new(1.0, 4.0)],
        ));
        let bars = bars(&chart, 0);
        assert_eq!(
            bars[0].y1, 0.0,
            "an empty position cannot be a share of itself"
        );
        assert!((bars[1].y1 - 100.0).abs() < 1e-9);
    }

    #[test]
    fn stacked_areas_rest_on_one_another() {
        let chart = Chart::new()
            .stack(Stack::Normal)
            .series(Series::area("a", at([3.0, 5.0, 4.0])))
            .series(Series::area("b", at([2.0, 1.0, 3.0])));
        let plan = plan_of(&chart);
        let Geometry::Band { upper, lower } = &plan.marks[1].geometry else {
            panic!("expected a band");
        };
        assert_eq!(
            lower[0].y, 3.0,
            "the second area starts where the first ended"
        );
        assert_eq!(upper[0].y, 5.0);
        let Geometry::Band { lower: first, .. } = &plan.marks[0].geometry else {
            panic!("expected a band");
        };
        assert_eq!(first[0].y, 0.0, "the first still rests on the baseline");
    }

    #[test]
    fn horizontal_orientation_swaps_the_axes() {
        let names = ["Chrome", "Safari", "Firefox"];
        let chart = Chart::new()
            .legend(false)
            .horizontal()
            .x_axis(Axis::new().categories(names))
            .series(Series::bar("share", at([42.0, 26.0, 15.0])));
        let (buffer, area) = render_to(&chart, 40, 10);
        let grid = grid_of(&buffer, area);

        // Category names claim the gutter, which is what horizontal buys: a
        // full row of width per name instead of one column.
        let gutter = axis_column(&buffer, area).expect("axis column");
        assert!(gutter >= 7, "names need real width, got {gutter}");
        for name in names {
            assert!(grid.contains(name), "missing {name} in\n{grid}");
        }
    }

    #[test]
    fn a_domain_straddling_zero_draws_the_baseline_rule() {
        let chart = Chart::new()
            .legend(false)
            .series(Series::bar("delta", at([4.0, -3.0, 6.0])));
        let (buffer, area) = render_to(&chart, 40, 12);
        let grid = grid_of(&buffer, area);
        assert!(
            grid.lines()
                .any(|line| line.matches('─').count() > 3 && !line.contains('└')),
            "a zero rule should cross the plot:\n{grid}"
        );

        let positive = Chart::new()
            .legend(false)
            .series(Series::bar("delta", at([4.0, 3.0, 6.0])));
        let (buffer, area) = render_to(&positive, 40, 12);
        assert!(
            !grid_of(&buffer, area)
                .lines()
                .any(|line| line.matches('─').count() > 3 && !line.contains('└')),
            "no rule when zero is the domain edge"
        );
    }

    #[test]
    fn markers_and_labels_annotate_without_covering_data() {
        let chart = Chart::new().legend(false).series(
            Series::line("visits", at([18.0, 30.0, 23.0]))
                .markers()
                .labels(),
        );
        let (buffer, area) = render_to(&chart, 46, 12);
        let grid = grid_of(&buffer, area);
        assert!(grid.contains('●'), "markers should be drawn:\n{grid}");
        assert!(grid.contains("30"), "values should be labelled:\n{grid}");

        // The marker at each sample survives: no label was written over one.
        let markers = grid.chars().filter(|&ch| ch == '●').count();
        assert_eq!(markers, 3, "every sample keeps its marker:\n{grid}");
    }

    #[test]
    fn the_focus_readout_lists_every_series_at_that_position() {
        let chart = Chart::new()
            .x_axis(months())
            .focus(1.0)
            .series(Series::line("visits", at([18.0, 30.0, 23.0])))
            .series(Series::line("signups", at([8.0, 14.0, 11.0])));
        let (buffer, area) = render_to(&chart, 52, 12);
        let grid = grid_of(&buffer, area);
        let readout = grid
            .lines()
            .find(|line| line.contains("visits 30"))
            .unwrap_or_else(|| panic!("no readout in\n{grid}"));
        assert!(readout.contains("Feb"), "the position is named: {readout}");
        assert!(readout.contains("signups 14"), "{readout}");
    }

    #[test]
    fn the_readout_takes_its_own_row_rather_than_the_label_row() {
        let focused = Chart::new()
            .x_axis(months())
            .focus(1.0)
            .series(Series::line("visits", at([18.0, 30.0, 23.0])));
        let (buffer, area) = render_to(&focused, 52, 12);
        let grid = grid_of(&buffer, area);
        let label_row = grid
            .lines()
            .position(|line| line.contains("Jan"))
            .expect("x labels");
        let readout_row = grid
            .lines()
            .position(|line| line.contains("visits 30"))
            .expect("readout");
        assert!(
            readout_row > label_row,
            "readout must not sit on the labels"
        );
    }

    #[test]
    fn a_donut_draws_a_ring_with_its_centre_text() {
        let chart = Chart::new()
            .center("1.2k")
            .series(Series::donut("desktop", [Point::new(0.0, 55.0)]))
            .series(Series::donut("mobile", [Point::new(0.0, 45.0)]));
        let plan = plan_of(&chart);
        assert!(plan.radial);
        let Geometry::Arcs(first) = &plan.marks[0].geometry else {
            panic!("expected arcs");
        };
        assert!((first[0].sweep - 0.55).abs() < 1e-9, "{first:?}");
        assert_eq!(first[0].start, 0.0);

        let (buffer, area) = render_to(&chart, 40, 16);
        let grid = grid_of(&buffer, area);
        assert!(grid.contains("1.2k"), "centre text:\n{grid}");
        assert!(grid.contains('█'), "ring:\n{grid}");
        // A ring, not a disc: the middle row has background inside it.
        let middle = grid.lines().nth(8).expect("a middle row");
        assert!(
            middle.trim_end().contains("  "),
            "the hole should survive: {middle:?}"
        );
    }

    #[test]
    fn a_donut_with_no_positive_total_renders_the_empty_state() {
        let chart = Chart::new().series(Series::donut("none", [Point::new(0.0, 0.0)]));
        let (buffer, area) = render_to(&chart, 30, 8);
        assert!(grid_of(&buffer, area).contains("No chart data"));
    }

    #[test]
    fn every_new_option_is_safe_at_degenerate_sizes() {
        let charts = [
            Chart::new()
                .x_axis(months())
                .stack(Stack::Percent)
                .series(Series::bar("a", at([3.0, 5.0, 4.0])))
                .series(Series::bar("b", at([2.0, 1.0, 3.0]))),
            Chart::new()
                .horizontal()
                .focus(1.0)
                .series(Series::bar("a", at([3.0, -5.0, 4.0])).labels()),
            Chart::new()
                .center("x")
                .series(Series::donut("a", [Point::new(0.0, 1.0)])),
        ];
        for chart in charts {
            for width in 0..=14 {
                for height in 0..=14 {
                    render_to(&chart, width, height);
                }
            }
        }
    }
}
