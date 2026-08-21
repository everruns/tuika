//! Responsive application chrome for tool-style terminal UIs.

use crate::geometry::Rect;

use crate::geometry::Size;
use crate::layout::{Dimension, FlexItemStyle, Item, LayoutStyle, solve};
use crate::style::Role;
use crate::surface::Surface;
use crate::view::{MeasureRequest, RenderCtx, ScopedElement, View, element};

enum Region<'view> {
    View {
        view: ScopedElement<'view>,
        style: FlexItemStyle,
    },
    Rule,
}

impl Region<'_> {
    fn style(&self, has_height: bool) -> FlexItemStyle {
        let mut style = match self {
            Self::View { style, .. } => *style,
            Self::Rule => FlexItemStyle::default().shrink(u16::MAX),
        };
        if !has_height {
            style.min_main = 0;
            style.max_main = Some(0);
        }
        style
    }

    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        match self {
            Self::View { view, .. } => view.measure(available, ctx),
            Self::Rule => Size::new(available.width, u16::from(available.height > 0)),
        }
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        match self {
            Self::View { view, .. } => view.render(area, surface, ctx),
            Self::Rule if area.height > 0 => {
                let style = ctx.sheet.resolve(Role::Rule).to_style();
                surface.set_string(area.x, area.y, &"─".repeat(area.width as usize), style);
            }
            Self::Rule => {}
        }
    }
}

/// A small responsive shell around one growing application content view.
///
/// Header-side and footer-side regions keep their builder call order. This
/// makes the conventional tool layout concise while still allowing custom
/// regions to be omitted or reordered:
///
/// ```
/// use tuika::prelude::*;
///
/// let shell = AppShell::new(Text::raw("content"))
///     .header(view_fn(
///         |available, _ctx| Size::new(available.width, available.height.min(2)),
///         |area, surface, ctx| {
///             surface.set_string(area.x, area.y, "Search: ", ctx.theme.muted_style());
///         },
///     ))
///     .top_rule()
///     .status(StatusBar::new())
///     .bottom_rule()
///     .footer(KeyHints::new([("q", "quit")]));
/// # let _ = shell;
/// ```
///
/// Chrome uses intrinsic height and shrinks when the terminal is short; rules
/// collapse first, followed by status/custom regions. The main content grows
/// into all remaining rows and keeps one row whenever any height exists. A
/// footer also keeps one row when there is room beside the main content.
/// Separators use the active stylesheet's [`Role::Rule`] style.
///
/// ![application shell demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/app_shell.png)
pub struct AppShell<'view> {
    before_main: Vec<Region<'view>>,
    main: Region<'view>,
    after_main: Vec<Region<'view>>,
}

impl<'view> AppShell<'view> {
    /// Start a shell around its growing main content.
    pub fn new<V: View + 'view>(main: V) -> Self {
        Self {
            before_main: Vec::new(),
            main: Region::View {
                view: element(main),
                style: FlexItemStyle::default()
                    .basis(Dimension::Fixed(0))
                    .grow(1)
                    .min_main(1),
            },
            after_main: Vec::new(),
        }
    }

    /// Append an intrinsic region before the main content.
    pub fn before_main<V: View + 'view>(mut self, view: V) -> Self {
        self.before_main.push(Self::chrome(view, 10, 0));
        self
    }

    /// Append a conventional header before the main content.
    pub fn header<V: View + 'view>(self, view: V) -> Self {
        self.before_main(view)
    }

    /// Append a theme-aware rule before the main content.
    pub fn top_rule(mut self) -> Self {
        self.before_main.push(Region::Rule);
        self
    }

    /// Append an intrinsic region after the main content.
    pub fn after_main<V: View + 'view>(mut self, view: V) -> Self {
        self.after_main.push(Self::chrome(view, 20, 0));
        self
    }

    /// Append a conventional status region after the main content.
    pub fn status<V: View + 'view>(self, view: V) -> Self {
        self.after_main(view)
    }

    /// Append a theme-aware rule after the main content.
    pub fn bottom_rule(mut self) -> Self {
        self.after_main.push(Region::Rule);
        self
    }

    /// Append a conventional footer after the main content.
    ///
    /// A footer keeps one row during height shrink, making this the fitting
    /// place for [`KeyHints`](super::KeyHints) or another primary action row.
    pub fn footer<V: View + 'view>(mut self, view: V) -> Self {
        self.after_main.push(Self::chrome(view, 1, 1));
        self
    }

    fn chrome<V: View + 'view>(view: V, shrink: u16, min_main: u16) -> Region<'view> {
        Region::View {
            view: element(view),
            style: FlexItemStyle::default().shrink(shrink).min_main(min_main),
        }
    }

    fn regions(&self) -> impl Iterator<Item = &Region<'view>> {
        self.before_main
            .iter()
            .chain(std::iter::once(&self.main))
            .chain(self.after_main.iter())
    }

    fn layout(&self, area: Rect, ctx: &RenderCtx) -> Vec<Rect> {
        let available = Size::from(area);
        let items = self
            .regions()
            .map(|region| {
                Item::styled(
                    region.style(area.height > 0),
                    region.measure(available, ctx),
                )
            })
            .collect::<Vec<_>>();
        solve(area, &LayoutStyle::column(), &items)
    }
}

impl View for AppShell<'_> {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        let mut width = 0;
        let mut height = 0u16;
        for region in self.regions() {
            let measured = region.measure(available, ctx);
            width = width.max(measured.width);
            height = height.saturating_add(measured.height);
        }
        Size::new(width, height).clamp_to(available)
    }

    fn measure_request(&self, request: MeasureRequest, ctx: &RenderCtx) -> Size {
        request.resolve(self.measure(request.fallback_available(), ctx))
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        for (region, rect) in self.regions().zip(self.layout(area, ctx)) {
            let mut child = surface.child(rect);
            region.render(rect, &mut child, ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Paragraph, Text};
    use crate::probe::RectProbe;
    use crate::style::{StyleBundle, StyleSheet};
    use crate::testing::{grid, render, render_sizes, render_with_sheet};
    use crate::{Theme, ui::Color, ui::Rect, ui::Style, view_fn};

    #[test]
    fn allocates_conventional_regions_at_normal_size() {
        let probes: Vec<_> = (0..4).map(|_| RectProbe::new()).collect();
        let shell = AppShell::new(probes[1].wrap(Text::raw("main")))
            .header(probes[0].wrap(Text::raw("header")))
            .top_rule()
            .status(probes[2].wrap(Text::raw("status")))
            .bottom_rule()
            .footer(probes[3].wrap(Text::raw("footer")));

        let _ = render(&shell, 20, 8, &Theme::default());

        assert_eq!(probes[0].rect(), Rect::new(0, 0, 20, 1));
        assert_eq!(probes[1].rect(), Rect::new(0, 2, 20, 3));
        assert_eq!(probes[2].rect(), Rect::new(0, 5, 20, 1));
        assert_eq!(probes[3].rect(), Rect::new(0, 7, 20, 1));
    }

    #[test]
    fn narrow_width_is_forwarded_to_width_sensitive_regions() {
        let main = RectProbe::new();
        let shell =
            AppShell::new(main.wrap(Paragraph::new("main content wraps", Style::default())))
                .header(Paragraph::new("header also wraps", Style::default()));

        let _ = render(&shell, 6, 6, &Theme::default());

        assert_eq!(main.rect(), Rect::new(0, 3, 6, 3));
    }

    #[test]
    fn short_layout_collapses_rules_and_status_before_main_and_footer() {
        let main = RectProbe::new();
        let status = RectProbe::new();
        let footer = RectProbe::new();
        let shell = AppShell::new(main.wrap(Text::raw("main")))
            .header(Text::raw("header"))
            .top_rule()
            .status(status.wrap(Text::raw("status")))
            .bottom_rule()
            .footer(footer.wrap(Text::raw("footer")));

        let _ = render(&shell, 10, 2, &Theme::default());

        assert_eq!(main.rect(), Rect::new(0, 0, 10, 1));
        assert_eq!(status.rect().height, 0);
        assert_eq!(footer.rect(), Rect::new(0, 1, 10, 1));
    }

    #[test]
    fn omitted_regions_leave_all_space_to_main() {
        let shell = AppShell::new(Text::raw("main"));
        let buffer = render(&shell, 8, 3, &Theme::default());
        assert_eq!(grid(&buffer), "main    \n        \n        ");
    }

    #[test]
    fn custom_regions_preserve_builder_order() {
        let first = RectProbe::new();
        let footer = RectProbe::new();
        let last = RectProbe::new();
        let shell = AppShell::new(Text::raw("main"))
            .after_main(first.wrap(Text::raw("first")))
            .footer(footer.wrap(Text::raw("footer")))
            .after_main(last.wrap(Text::raw("last")));

        let _ = render(&shell, 8, 4, &Theme::default());

        assert_eq!(first.rect().y, 1);
        assert_eq!(footer.rect().y, 2);
        assert_eq!(last.rect().y, 3);
    }

    #[test]
    fn rules_follow_the_active_stylesheet() {
        let theme = Theme::default();
        let sheet = StyleSheet {
            rule: StyleBundle::new().fg(Color::Green),
            ..StyleSheet::from_theme(&theme)
        };
        let shell = AppShell::new(Text::raw("main")).top_rule();

        let buffer = render_with_sheet(&shell, 5, 2, &theme, sheet);

        assert_eq!(buffer[(0, 0)].symbol(), "─");
        assert_eq!(buffer[(0, 0)].fg, Color::Green);
    }

    #[test]
    fn borrowed_views_can_be_composed_without_cloning() {
        let label = String::from("borrowed");
        let shell = AppShell::new(view_fn(
            |available, _ctx| Size::new(label.len().min(available.width as usize) as u16, 1),
            |area, surface, _ctx| {
                surface.set_string(area.x, area.y, &label, Style::default());
            },
        ))
        .footer(Text::raw("footer"));
        let buffer = render(&shell, 8, 2, &Theme::default());
        assert_eq!(grid(&buffer), "borrowed\nfooter  ");
    }

    #[test]
    fn degenerate_size_sweep_stays_inside_the_clip() {
        let shell = AppShell::new(Paragraph::new("main wraps", Style::default()))
            .header(Text::raw("header"))
            .top_rule()
            .status(Text::raw("status"))
            .bottom_rule()
            .footer(Text::raw("footer"));
        let sizes = (0..=8).flat_map(|width| (0..=6).map(move |height| (width, height)));

        let buffers = render_sizes(&shell, sizes, &Theme::default());

        assert_eq!(buffers.len(), 63);
    }
}
