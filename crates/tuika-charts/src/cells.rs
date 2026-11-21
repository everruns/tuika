use tuika::framebuffer::QUADRANTS;
use tuika::ui::Rect;
use tuika::ui::Style;
use tuika::{RenderCtx, Surface};

use crate::axis::AxisLayout;
use crate::plan::{ChartPlan, Geometry, Mark};
use crate::reference::{Reference, ReferenceAt};
use crate::{
    Chart, PlotModel, Point, chart_color, draw_dashed, draw_line, fill_polygon, trim_number,
};

pub(super) fn render_cells(
    plot: Rect,
    surface: &mut Surface,
    chart: &Chart,
    plan: &ChartPlan,
    layout: &AxisLayout,
    ctx: &RenderCtx,
) {
    if plot.width == 0 || plot.height == 0 {
        return;
    }
    let model = &plan.model;
    let axis = Style::default().fg(ctx.theme.border);
    for y in plot.y..plot.bottom() {
        surface.set(plot.x, y, '│', axis);
    }
    for x in plot.x..plot.right() {
        surface.set(x, plot.bottom() - 1, '─', axis);
    }
    // Tick marks before the corner, so a tick on the origin cannot erase it.
    for tick in &layout.y {
        surface.set(plot.x, tick.at, '┤', axis);
    }
    for tick in &layout.x {
        surface.set(tick.at, plot.bottom() - 1, '┬', axis);
    }
    surface.set(plot.x, plot.bottom() - 1, '└', axis);

    let data_width = plot.width.saturating_sub(1) as u32;
    let data_height = plot.height.saturating_sub(1) as u32;
    if data_width == 0 || data_height == 0 {
        return;
    }
    // Reference geometry first, so every mark lands on top of it.
    render_grid(plot, surface, chart, layout, ctx);
    render_zero_rule(plot, surface, model, data_width, data_height, ctx);
    for reference in chart.references.iter().filter(|r| r.finite()) {
        render_reference_line(
            plot,
            surface,
            model,
            reference,
            data_width,
            data_height,
            ctx,
        );
    }

    for mark in &plan.marks {
        let style = Style::default().fg(mark.color);
        match &mark.geometry {
            Geometry::Path { points, stepped } => {
                let mapped = map_all(points, model, data_width * 2, data_height * 2);
                let mut quadrants = QuadrantGrid::new(data_width, data_height);
                if mark.dashed {
                    let path = if *stepped {
                        stepped_path(&mapped)
                    } else {
                        mapped
                    };
                    // Subcells: three on, two off reads as a dash at any slope.
                    draw_dashed(&path, 3, 2, |x, y| quadrants.set(x, y));
                } else {
                    draw_quadrant_polyline(&mut quadrants, &mapped, *stepped);
                }
                quadrants.render(surface, plot, style);
            }
            Geometry::Fill { upper, lower } => {
                let upper = map_all(upper, model, data_width * 2, data_height * 2);
                let lower = map_all(lower, model, data_width * 2, data_height * 2);
                let mut quadrants = QuadrantGrid::new(data_width, data_height);
                fill_polygon(&upper, &lower, |x, y| quadrants.set(x, y));
                quadrants.render(surface, plot, style);
            }
            Geometry::Points(points) => {
                let mapped = map_all(points, model, data_width * 2, data_height * 4);
                let mut braille = BrailleGrid::new(data_width, data_height);
                for (x, y) in mapped {
                    braille.set(x, y);
                }
                braille.render(surface, plot, style);
            }
            Geometry::Band { upper, lower } => {
                let edge = map_all(upper, model, data_width * 2, data_height * 2);
                let floor = map_all(lower, model, data_width * 2, data_height * 2);
                let mut quadrants = QuadrantGrid::new(data_width, data_height);
                draw_quadrant_band(&mut quadrants, &edge, &floor);
                quadrants.render(surface, plot, style);
            }
            Geometry::Bars { bars, .. } => {
                for bar in bars {
                    // A bar is a rectangle in data space; painting its mapped
                    // corners keeps grouped slots and stacked segments exact
                    // without the renderer knowing either rule.
                    let (x0, y0) = model.map(Point::new(bar.x0, bar.y0), data_width, data_height);
                    let (x1, y1) = model.map(Point::new(bar.x1, bar.y1), data_width, data_height);
                    let (lo, hi) = half_open(x0, x1, y0, y1, model.horizontal);
                    for x in lo.0..=hi.0 {
                        for y in lo.1..=hi.1 {
                            set_plot(surface, plot, x, y, '█', style);
                        }
                    }
                }
            }
            Geometry::Arcs(_) => {}
        }
        if mark.markers {
            // A whole glyph, not a Braille dot: a dot beside a quadrant stroke
            // reads as an artifact of the line rather than as a sample.
            for sample in &mark.samples {
                let (x, y) = model.map(*sample, data_width, data_height);
                set_plot(surface, plot, x, y, '●', style);
            }
        }
    }
    render_overlays(plot, surface, chart, plan, ctx, true);
}

/// Insert the corner of every horizontal-then-vertical step, so a stepped path
/// can be walked as an ordinary polyline.
pub(crate) fn stepped_path(points: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut path = Vec::with_capacity(points.len() * 2);
    for pair in points.windows(2) {
        if path.is_empty() {
            path.push(pair[0]);
        }
        path.push((pair[1].0, pair[0].1));
        path.push(pair[1]);
    }
    if path.is_empty() {
        path.extend(points.first());
    }
    path
}

/// Grid lines at every labelled tick of an axis that asked for them.
///
/// Drawn from the resolved ticks rather than recomputed, so a thinned label
/// takes its line with it and the grid can never disagree with the labels.
fn render_grid(
    plot: Rect,
    surface: &mut Surface,
    chart: &Chart,
    layout: &AxisLayout,
    ctx: &RenderCtx,
) {
    let (across, up) = crate::axis::screen_axes(chart);
    let style = Style::default().fg(ctx.theme.dim);
    let (left, right) = (plot.x + 1, plot.right());
    let bottom = plot.bottom() - 1;
    if up.grid {
        for tick in layout.y.iter().filter(|tick| tick.at < bottom) {
            for x in left..right {
                surface.set(x, tick.at, '┈', style);
            }
        }
    }
    if across.grid {
        for tick in layout.x.iter().filter(|tick| tick.at >= left) {
            for y in plot.y..bottom {
                let crossing = up.grid && layout.y.iter().any(|t| t.at == y);
                surface.set(tick.at, y, if crossing { '┼' } else { '┊' }, style);
            }
        }
    }
}

/// The screen line a reference runs along, in plot-local cells: `true` and a
/// row for a line across the plot, `false` and a column for one up it.
fn reference_track(
    model: &PlotModel,
    reference: &Reference,
    data_width: u32,
    data_height: u32,
) -> (bool, i32) {
    // A value line runs along the category axis, which is the screen's x axis
    // unless the chart is horizontal.
    let across = matches!(reference.at, ReferenceAt::Value(_)) != model.horizontal;
    let point = match reference.at {
        ReferenceAt::Value(v) => model.value_point(v),
        ReferenceAt::Position(p) => model.position_point(p),
    };
    let (x, y) = model.map(point, data_width, data_height);
    (across, if across { y } else { x })
}

/// Whether a reference coordinate is inside its axis' domain. Explicit
/// domains clip references the way they clip data, rather than pinning an
/// off-scale threshold to the frame where it would read as a real value.
pub(crate) fn reference_visible(model: &PlotModel, reference: &Reference) -> bool {
    match reference.at {
        ReferenceAt::Value(v) => (model.y.min..=model.y.max).contains(&v),
        ReferenceAt::Position(p) => (model.x.min..=model.x.max).contains(&p),
    }
}

fn reference_style(reference: &Reference, ctx: &RenderCtx) -> Style {
    Style::default().fg(reference.color.unwrap_or(ctx.theme.border))
}

fn render_reference_line(
    plot: Rect,
    surface: &mut Surface,
    model: &PlotModel,
    reference: &Reference,
    data_width: u32,
    data_height: u32,
    ctx: &RenderCtx,
) {
    if !reference_visible(model, reference) {
        return;
    }
    let style = reference_style(reference, ctx);
    let (across, at) = reference_track(model, reference, data_width, data_height);
    // Where two references cross, a `┼` keeps both lines continuous instead
    // of the later one cutting the earlier.
    let crosses = |surface: &mut Surface, x: i32, y: i32, other: [&str; 2]| {
        let (col, row) = (plot.x as i32 + 1 + x, plot.y as i32 + y);
        let (Ok(col), Ok(row)) = (u16::try_from(col), u16::try_from(row)) else {
            return false;
        };
        let buffer = surface.buffer_mut();
        let area = buffer.area;
        col >= area.x
            && row >= area.y
            && col < area.right()
            && row < area.bottom()
            && other.contains(&buffer[(col, row)].symbol())
    };
    if across {
        let glyph = if reference.dashed { '╌' } else { '─' };
        for column in 0..data_width as i32 {
            let glyph = if crosses(surface, column, at, ["│", "╎"]) {
                '┼'
            } else {
                glyph
            };
            set_plot(surface, plot, column, at, glyph, style);
        }
    } else {
        let glyph = if reference.dashed { '╎' } else { '│' };
        for row in 0..data_height as i32 {
            let glyph = if crosses(surface, at, row, ["─", "╌"]) {
                '┼'
            } else {
                glyph
            };
            set_plot(surface, plot, at, row, glyph, style);
        }
    }
}

/// Reference labels, placed after every mark so they can avoid them: a value
/// line's label sits just above its right end, a position line's at the top of
/// the plot beside it. A label with no clear cell is dropped.
fn render_reference_labels(
    plot: Rect,
    surface: &mut Surface,
    chart: &Chart,
    model: &PlotModel,
    ctx: &RenderCtx,
) {
    let data_width = plot.width.saturating_sub(1) as u32;
    let data_height = plot.height.saturating_sub(1) as u32;
    if data_width == 0 || data_height == 0 {
        return;
    }
    for reference in &chart.references {
        let Some(label) = reference.label.as_deref().filter(|label| !label.is_empty()) else {
            continue;
        };
        if !reference.finite() || !reference_visible(model, reference) {
            continue;
        }
        let Ok(len) = u16::try_from(label.chars().count()) else {
            continue;
        };
        let style = reference_style(reference, ctx);
        let (across, at) = reference_track(model, reference, data_width, data_height);
        let Ok(at) = u16::try_from(at) else {
            continue;
        };
        // Above the line's end first; on the line itself when that row is
        // taken or off the plot (a threshold at the top of the domain); then
        // below it.
        let candidates: [Option<(u16, u16)>; 3] = if across {
            let row = plot.y + at;
            let col = plot.right().saturating_sub(len);
            [
                row.checked_sub(1).map(|row| (col, row)),
                Some((col, row)),
                Some((col, row + 1)),
            ]
        } else {
            let col = plot.x + 1 + at;
            [
                Some((col + 1, plot.y)),
                col.checked_sub(len).map(|col| (col, plot.y)),
                None,
            ]
        };
        for (col, row) in candidates.into_iter().flatten() {
            if col <= plot.x || col + len > plot.right() || row < plot.y || row >= plot.bottom() - 1
            {
                continue;
            }
            if (col..col + len).all(|x| is_blank(surface, x, row) || is_rule(surface, x, row)) {
                surface.set_string(col, row, label, style);
                break;
            }
        }
    }
}

/// Whether a cell holds only reference geometry — a grid or rule glyph a label
/// may replace without hiding data.
fn is_rule(surface: &mut Surface, x: u16, y: u16) -> bool {
    let buffer = surface.buffer_mut();
    let area = buffer.area;
    if x < area.x || y < area.y || x >= area.right() || y >= area.bottom() {
        return false;
    }
    matches!(
        buffer[(x, y)].symbol(),
        "┈" | "┊" | "┼" | "─" | "│" | "╌" | "╎"
    )
}

/// Draw a rule along zero when the value domain straddles it, so negative
/// geometry is read against the baseline rather than against the frame.
///
/// Values always live on the chart's y axis; only which way the rule runs
/// depends on orientation.
fn render_zero_rule(
    plot: Rect,
    surface: &mut Surface,
    model: &PlotModel,
    data_width: u32,
    data_height: u32,
    ctx: &RenderCtx,
) {
    if !(model.y.min < 0.0 && model.y.max > 0.0) {
        return;
    }
    let style = Style::default().fg(ctx.theme.border);
    if model.horizontal {
        let column = model.map(model.across_at(0.0), data_width, data_height).0;
        for row in 0..data_height as i32 {
            set_plot_dim(surface, plot, column, row, '│', style);
        }
    } else {
        let row = model.map(model.up_at(0.0), data_width, data_height).1;
        for column in 0..data_width as i32 {
            set_plot_dim(surface, plot, column, row, '─', style);
        }
    }
}

/// Value labels and the focus rule. Text over an image for the same reason the
/// tick labels are: cells stay crisp and identical in both render modes.
pub(super) fn render_overlays(
    plot: Rect,
    surface: &mut Surface,
    chart: &Chart,
    plan: &ChartPlan,
    ctx: &RenderCtx,
    focus_rule: bool,
) {
    let data_width = plot.width.saturating_sub(1) as u32;
    let data_height = plot.height.saturating_sub(1) as u32;
    if data_width == 0 || data_height == 0 {
        return;
    }
    let model = &plan.model;

    if let Some(focus) = chart.focus.filter(|_| focus_rule) {
        let style = Style::default().fg(ctx.theme.accent);
        if model.horizontal {
            let row = model.map(model.up_at(focus), data_width, data_height).1;
            for column in 0..data_width as i32 {
                set_plot_dim(surface, plot, column, row, '─', style);
            }
        } else {
            let column = model.map(model.across_at(focus), data_width, data_height).0;
            for row in 0..data_height as i32 {
                set_plot_dim(surface, plot, column, row, '│', style);
            }
        }
    }

    for mark in plan.marks.iter().filter(|mark| mark.labelled) {
        render_value_labels(plot, surface, model, mark, data_width, data_height, ctx);
    }
    render_reference_labels(plot, surface, chart, model, ctx);
}

/// Print each sample's value beside its mark.
///
/// A label annotates a reading, so it may never cover one: each is tried in a
/// few positions around its mark and dropped if none is clear. Dropping is the
/// right failure — a label sitting on the geometry it describes is worse than
/// no label at all.
fn render_value_labels(
    plot: Rect,
    surface: &mut Surface,
    model: &PlotModel,
    mark: &Mark,
    data_width: u32,
    data_height: u32,
    ctx: &RenderCtx,
) {
    let style = ctx.theme.muted_style();
    let mut taken: Vec<(u16, u16, u16)> = Vec::new();
    for sample in &mark.samples {
        let text = trim_number(sample.y);
        let Ok(len) = u16::try_from(text.chars().count()) else {
            continue;
        };
        let (x, y) = model.map(*sample, data_width, data_height);
        if x < 0 || y < 0 {
            continue;
        }
        let (col, row) = (plot.x + 1 + x as u16, plot.y + y as u16);
        let centered = col.saturating_sub(len / 2);
        let candidates: [(u16, u16); 4] = if model.horizontal {
            // Past the bar's end first, then just inside it.
            [
                (col + 1, row),
                (col.saturating_sub(len + 1), row),
                (col + 1, row.saturating_sub(1)),
                (col + 1, row + 1),
            ]
        } else {
            [
                (centered, row.saturating_sub(1)),
                (centered, row + 1),
                (col + 1, row),
                (col.saturating_sub(len + 1), row),
            ]
        };
        for (col, row) in candidates {
            if col < plot.x + 1
                || col + len > plot.right()
                || row < plot.y
                || row >= plot.bottom() - 1
            {
                continue;
            }
            if taken
                .iter()
                .any(|&(r, start, end)| r == row && col < end && start < col + len)
            {
                continue;
            }
            // Grid and rule glyphs are reference geometry, not readings, so
            // a label may sit on them.
            if (col..col + len).any(|x| !is_blank(surface, x, row) && !is_rule(surface, x, row)) {
                continue;
            }
            taken.push((row, col, col + len));
            surface.set_string(col, row, &text, style);
            break;
        }
    }
}

/// A donut: one ring of arcs with optional centre text.
pub(super) fn render_donut(
    body: Rect,
    surface: &mut Surface,
    chart: &Chart,
    plan: &ChartPlan,
    ctx: &RenderCtx,
) {
    if body.width < 3 || body.height < 3 {
        return;
    }
    // Cells are about twice as tall as they are wide, so a ring drawn on a
    // square grid of cells would read as an ellipse. Scaling x by two restores
    // the circle a reader expects.
    let radius = f64::from(body.height.min(body.width / 2)) / 2.0 - 0.5;
    if radius < 1.0 {
        return;
    }
    let cx = f64::from(body.x) + f64::from(body.width) / 2.0;
    let cy = f64::from(body.y) + f64::from(body.height) / 2.0;
    let thickness = (radius * 0.42).max(1.0);

    for mark in &plan.marks {
        let Geometry::Arcs(arcs) = &mark.geometry else {
            continue;
        };
        let style = Style::default().fg(mark.color);
        for arc in arcs {
            // Sample densely enough that the thinnest ring has no gaps.
            let steps = ((arc.sweep * radius * 24.0).ceil() as usize).max(2);
            for step in 0..=steps {
                let turn = arc.start + arc.sweep * step as f64 / steps as f64;
                let angle = turn * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2;
                let (sin, cos) = angle.sin_cos();
                let mut inward = 0.0;
                while inward <= thickness {
                    let r = radius - inward;
                    let x = cx + cos * r * 2.0;
                    let y = cy + sin * r;
                    if x >= f64::from(body.x)
                        && y >= f64::from(body.y)
                        && x < f64::from(body.right())
                        && y < f64::from(body.bottom())
                    {
                        surface.set(x as u16, y as u16, '█', style);
                    }
                    inward += 0.5;
                }
            }
        }
    }

    render_center(body, surface, chart, ctx);
}

/// The donut's centre text. Cells in both modes, like every other label.
pub(super) fn render_center(body: Rect, surface: &mut Surface, chart: &Chart, ctx: &RenderCtx) {
    if chart.center.is_empty() || body.width == 0 || body.height == 0 {
        return;
    }
    let Ok(len) = u16::try_from(chart.center.chars().count()) else {
        return;
    };
    let x = (body.x + body.width / 2).saturating_sub(len / 2);
    let y = body.y + body.height / 2;
    if x + len <= body.right() && y < body.bottom() {
        surface.set_string(x, y, &chart.center, ctx.theme.accent_style());
    }
}

/// A bar's cell span, half-open across the category axis.
///
/// A band's gutter can be under a cell wide, and two bars rounded to touching
/// columns read as one wide bar. Dropping the far column keeps the gap the
/// grouping intended, at the cost of a cell of width no reader can miss.
fn half_open(x0: i32, x1: i32, y0: i32, y1: i32, horizontal: bool) -> ((i32, i32), (i32, i32)) {
    let (lox, mut hix) = (x0.min(x1), x0.max(x1));
    let (loy, mut hiy) = (y0.min(y1), y0.max(y1));
    // Shrink across the category axis only: the value direction carries the
    // reading, and taking a cell off it would understate the bar.
    if horizontal {
        hiy = hiy.max(loy + 1) - 1;
    } else {
        hix = hix.max(lox + 1) - 1;
    }
    ((lox, loy), (hix, hiy))
}

fn map_all(points: &[Point], model: &PlotModel, width: u32, height: u32) -> Vec<(i32, i32)> {
    points
        .iter()
        .filter(|point| point.x.is_finite() && point.y.is_finite())
        .map(|point| model.map(*point, width, height))
        .collect()
}

fn set_plot(surface: &mut Surface, plot: Rect, x: i32, y: i32, ch: char, mut style: Style) {
    if x >= 0
        && y >= 0
        && x < plot.width.saturating_sub(1) as i32
        && y < plot.height.saturating_sub(1) as i32
    {
        let (col, row) = (plot.x + 1 + x as u16, plot.y + y as u16);
        // A partial glyph over a solid fill cell would punch a background-
        // colored hole in a band or bar. Carry the fill into the new glyph's
        // background instead, so a line drawn over a band stays on the band.
        if ch != '█'
            && style.bg.is_none()
            && let Some(fill) = solid_fill(surface, col, row)
        {
            style = style.bg(fill);
        }
        surface.set(col, row, ch, style);
    }
}

/// The color of a solid fill cell at `(x, y)`, if that is what it holds.
fn solid_fill(surface: &mut Surface, x: u16, y: u16) -> Option<tuika::ui::Color> {
    let clip = surface.area();
    if x < clip.x || y < clip.y || x >= clip.right() || y >= clip.bottom() {
        return None;
    }
    let buffer = surface.buffer_mut();
    let area = buffer.area;
    if x < area.x || y < area.y || x >= area.right() || y >= area.bottom() {
        return None;
    }
    let cell = &buffer[(x, y)];
    (cell.symbol() == "█").then_some(cell.fg)
}

/// Like [`set_plot`] but never paints over data already drawn there.
fn set_plot_dim(surface: &mut Surface, plot: Rect, x: i32, y: i32, ch: char, style: Style) {
    if x < 0 || y < 0 {
        return;
    }
    let (col, row) = (plot.x + 1 + x as u16, plot.y + y as u16);
    if x >= plot.width.saturating_sub(1) as i32 || y >= plot.height.saturating_sub(1) as i32 {
        return;
    }
    if !is_blank(surface, col, row) {
        return;
    }
    surface.set(col, row, ch, style);
}

/// Whether a cell still holds background, so a rule can be drawn there.
///
/// Rules are reference lines, not data: they must never paint over a mark the
/// reader is trying to measure against them.
fn is_blank(surface: &mut Surface, x: u16, y: u16) -> bool {
    let clip = surface.area();
    if x < clip.x || y < clip.y || x >= clip.right() || y >= clip.bottom() {
        return false;
    }
    let buffer = surface.buffer_mut();
    let area = buffer.area;
    if x < area.x || y < area.y || x >= area.right() || y >= area.bottom() {
        return false;
    }
    buffer[(x, y)].symbol().trim().is_empty()
}

/// Fill between an upper edge and the floor it rests on.
pub(crate) fn draw_quadrant_band(
    grid: &mut QuadrantGrid,
    upper: &[(i32, i32)],
    lower: &[(i32, i32)],
) {
    let floor_at = |index: usize| lower.get(index).map_or(0, |&(_, y)| y);
    for (index, pair) in upper.windows(2).enumerate() {
        let base = floor_at(index).max(floor_at(index + 1));
        draw_line(pair[0], pair[1], |x, y| {
            for row in y.min(base)..=y.max(base) {
                grid.set(x, row);
            }
        });
    }
    if let Some(&(x, y)) = upper.first() {
        let base = floor_at(0);
        for row in y.min(base)..=y.max(base) {
            grid.set(x, row);
        }
    }
}

pub(crate) struct BrailleGrid {
    cols: u32,
    rows: u32,
    pub(crate) masks: Vec<u8>,
}

impl BrailleGrid {
    pub(crate) fn new(cols: u32, rows: u32) -> Self {
        Self {
            cols,
            rows,
            masks: vec![0; (cols as usize).saturating_mul(rows as usize)],
        }
    }

    pub(crate) fn set(&mut self, x: i32, y: i32) {
        if x < 0 || y < 0 || x >= (self.cols * 2) as i32 || y >= (self.rows * 4) as i32 {
            return;
        }
        const DOTS: [[u8; 2]; 4] = [
            [0b0000_0001, 0b0000_1000],
            [0b0000_0010, 0b0001_0000],
            [0b0000_0100, 0b0010_0000],
            [0b0100_0000, 0b1000_0000],
        ];
        let cell_x = x as u32 / 2;
        let cell_y = y as u32 / 4;
        let index = (cell_y * self.cols + cell_x) as usize;
        self.masks[index] |= DOTS[y as usize % 4][x as usize % 2];
    }

    fn render(self, surface: &mut Surface, plot: Rect, style: Style) {
        for (index, mask) in self.masks.into_iter().enumerate() {
            if mask == 0 {
                continue;
            }
            let x = index as u32 % self.cols;
            let y = index as u32 / self.cols;
            let glyph = char::from_u32(0x2800 + u32::from(mask)).expect("valid Braille mask");
            set_plot(surface, plot, x as i32, y as i32, glyph, style);
        }
    }
}

pub(crate) struct QuadrantGrid {
    cols: u32,
    rows: u32,
    pub(crate) masks: Vec<u8>,
}

impl QuadrantGrid {
    pub(crate) fn new(cols: u32, rows: u32) -> Self {
        Self {
            cols,
            rows,
            masks: vec![0; (cols as usize).saturating_mul(rows as usize)],
        }
    }

    pub(crate) fn set(&mut self, x: i32, y: i32) {
        if x < 0 || y < 0 || x >= (self.cols * 2) as i32 || y >= (self.rows * 2) as i32 {
            return;
        }
        let cell_x = x as u32 / 2;
        let cell_y = y as u32 / 2;
        let index = (cell_y * self.cols + cell_x) as usize;
        self.masks[index] |= 1 << ((x as usize % 2) + 2 * (y as usize % 2));
    }

    fn render(self, surface: &mut Surface, plot: Rect, style: Style) {
        for (index, mask) in self.masks.into_iter().enumerate() {
            if mask == 0 {
                continue;
            }
            let x = index as u32 % self.cols;
            let y = index as u32 / self.cols;
            set_plot(
                surface,
                plot,
                x as i32,
                y as i32,
                QUADRANTS[mask as usize],
                style,
            );
        }
    }
}

fn draw_quadrant_polyline(grid: &mut QuadrantGrid, points: &[(i32, i32)], stepped: bool) {
    for pair in points.windows(2) {
        if stepped {
            let corner = (pair[1].0, pair[0].1);
            draw_line(pair[0], corner, |x, y| grid.set(x, y));
            draw_line(corner, pair[1], |x, y| grid.set(x, y));
        } else {
            draw_line(pair[0], pair[1], |x, y| grid.set(x, y));
        }
    }
    for &(x, y) in points {
        grid.set(x, y);
    }
}

pub(super) fn render_legend(area: Rect, surface: &mut Surface, chart: &Chart, ctx: &RenderCtx) {
    if !chart.has_legend() || area.height == 0 {
        return;
    }
    let mut x = area.x;
    let y = area.bottom() - 1;
    for (index, series) in chart.series.iter().enumerate() {
        if !series.legend {
            continue;
        }
        let color = chart_color(series, index, ctx);
        surface.set(x, y, '■', Style::default().fg(color));
        x = x.saturating_add(2);
        x = surface.set_string(x, y, &series.name, ctx.theme.muted_style());
        x = x.saturating_add(2);
        if x >= area.right() {
            break;
        }
    }
}
