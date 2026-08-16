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

use ratatui_core::layout::Rect;
use ratatui_core::style::{Color, Style};
use tuika::term::image::{ImageLayer, ImageSupport};
use tuika::{RenderCtx, Size, Surface, View};

mod axis;
mod cells;
mod graphics;
mod model;

pub use axis::Axis;
use axis::AxisLayout;
use cells::{render_cells, render_legend};
use graphics::render_pixels;
use model::PlotModel;
pub use model::{Domain, Point, Series, SeriesKind};

/// Which rendering path a chart used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderMode {
    /// High-resolution image emitted through a graphics protocol.
    Graphics,
    /// Unicode cell renderer, available in every terminal.
    Cells,
}

/// An adaptive line, bar, area, scatter, or step chart view.
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
        let Some(model) = PlotModel::new(self) else {
            render_empty(area, surface, ctx, &self.title);
            return;
        };
        surface.fill(Style::default().bg(ctx.theme.background));
        if !self.title.is_empty() {
            surface.set_string(area.x, area.y, &self.title, ctx.theme.accent_style());
        }
        render_legend(area, surface, self, ctx);

        let mut layout = AxisLayout::new(area, self, &model);
        let plot = plot_rect(area, self, &layout);
        layout.resolve(plot, area, self, &model);
        // Labels are cells in both modes: the graphics image covers the plot
        // only, so the same text lands beside it either way.
        layout.render(plot, surface, ctx);

        let (support, layer) = self.graphics(ctx);
        if self.render_mode_in(ctx) == RenderMode::Graphics {
            let data = render_pixels(plot.width, plot.height, self, &model, &layout, ctx);
            if let (Some(data), Some(layer)) = (data, layer) {
                // Image owns capability-gated placement; using it here keeps the
                // same protocol lifecycle and fallback semantics as core images.
                tuika::components::Image::new(data, plot.width, plot.height)
                    .support(support)
                    .in_layer(layer)
                    .render(plot, &mut surface.child(plot), ctx);
                return;
            }
        }
        render_cells(plot, surface, self, &model, &layout, ctx);
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
    Rect::new(
        area.x.saturating_add(layout.gutter),
        area.y.saturating_add(title),
        area.width.saturating_sub(layout.gutter + 1),
        area.height.saturating_sub(title + legend + 1),
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
    use crate::cells::{BrailleGrid, QuadrantGrid, draw_quadrant_area};
    use ratatui_core::buffer::Buffer;
    use tuika::Theme;

    fn is_braille(ch: char) -> bool {
        ('\u{2801}'..='\u{28ff}').contains(&ch)
    }

    fn is_quadrant(ch: char) -> bool {
        ratatui_core::symbols::pixel::QUADRANTS[1..].contains(&ch)
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
    fn quadrant_area_stops_at_the_edge_subcell() {
        let mut grid = QuadrantGrid::new(1, 2);
        draw_quadrant_area(&mut grid, &[(0, 1), (1, 1)], 3);
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
        let model = PlotModel::new(&chart).unwrap();
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
        let model = PlotModel::new(&chart).unwrap();
        assert!(model.x.min < model.x.max);
        assert!(model.y.min < model.y.max);
    }

    #[test]
    fn automatic_bar_domain_reserves_half_an_interval_at_each_edge() {
        let chart = Chart::new().series(Series::bar(
            "bars",
            [
                Point::new(0.0, 1.0),
                Point::new(1.0, 2.0),
                Point::new(2.0, 3.0),
            ],
        ));
        let model = PlotModel::new(&chart).unwrap();
        assert_eq!(
            model.x,
            Domain {
                min: -0.5,
                max: 2.5
            }
        );

        let explicit = chart.x_domain(Domain::new(0.0, 2.0).unwrap());
        assert_eq!(
            PlotModel::new(&explicit).unwrap().x,
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
        let mut graphics = ratatui_core::buffer::Buffer::empty(area);
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
    use super::*;

    fn domain_of(chart: &Chart) -> Domain {
        PlotModel::new(chart).unwrap().y
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
        assert!(domain.max > 4.0, "the bar must not touch the top edge");

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
        let mut buffer = ratatui_core::buffer::Buffer::empty(area);
        Chart::new()
            .legend(false)
            .series(Series::bar("bars", points([1.0, 2.0, 4.0])))
            .render(
                area,
                &mut Surface::new(&mut buffer, area),
                &RenderCtx::new(&tuika::Theme::default()),
            );

        // Column heights must grow with the values, and the smallest bar must
        // still be drawn rather than collapse onto the axis.
        let heights: Vec<usize> = (0..area.width)
            .map(|x| {
                (0..area.height)
                    .filter(|&y| buffer[(x, y)].symbol() == "█")
                    .count()
            })
            .filter(|&count| count > 0)
            .collect();
        assert_eq!(heights.len(), 3, "one column per bar: {heights:?}");
        assert!(heights[0] >= 2, "the smallest bar is visible: {heights:?}");
        assert!(
            heights[0] < heights[1] && heights[1] < heights[2],
            "{heights:?}"
        );
    }
}
