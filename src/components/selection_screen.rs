//! Responsive full-screen selection flows composed from Tuika primitives.

use crate::geometry::Rect;
use crate::style::Style;
use crate::text::Line;

use super::select::SelectRows;
use super::{AppShell, SelectState, VirtualWindow};
use crate::geometry::Size;
use crate::style::Role;
use crate::surface::Surface;
use crate::view::{MeasureRequest, RenderCtx, ScopedElement, View, element};

enum Rows<'items> {
    Owned(Vec<Line<'static>>),
    Borrowed(&'items [Line<'static>]),
}

impl Rows<'_> {
    fn as_slice(&self) -> &[Line<'static>] {
        match self {
            Self::Owned(rows) => rows,
            Self::Borrowed(rows) => rows,
        }
    }
}

enum Header<'view> {
    Title(Line<'static>),
    Custom(ScopedElement<'view>),
}

struct HeaderView<'a> {
    header: &'a Header<'a>,
    style: Option<Style>,
}

impl View for HeaderView<'_> {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        match self.header {
            Header::Title(_) => Size::new(available.width, u16::from(available.height > 0)),
            Header::Custom(view) => view.measure(available, ctx),
        }
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        match self.header {
            Header::Title(title) if area.height > 0 => {
                let base = ctx.sheet.resolve(Role::Heading).to_style();
                let style = self
                    .style
                    .map_or(base, |override_style| base.patch(override_style));
                let mut x = area.x;
                for span in &title.spans {
                    if x >= area.right() {
                        break;
                    }
                    x = surface.set_string(
                        x,
                        area.y,
                        span.content.as_ref(),
                        style.patch(title.style).patch(span.style),
                    );
                }
            }
            Header::Custom(view) => view.render(area, surface, ctx),
            Header::Title(_) => {}
        }
    }
}

struct ViewRef<'a>(&'a dyn View);

impl View for ViewRef<'_> {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        self.0.measure(available, ctx)
    }

    fn measure_request(&self, request: MeasureRequest, ctx: &RenderCtx) -> Size {
        self.0.measure_request(request, ctx)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        self.0.render(area, surface, ctx);
    }
}

/// A responsive selection page with conventional tool chrome.
///
/// `SelectionScreen` composes [`AppShell`], the same borrowed row renderer as
/// [`SelectList`](super::SelectList), and an optional footer such as
/// [`KeyHints`](super::KeyHints). Its list viewport follows the body rows that
/// remain after chrome collapses, so the selected item stays visible even on a
/// short terminal. Use [`borrowed`](Self::borrowed) to reuse an application's
/// row vector without cloning it for each frame.
///
/// ```
/// use tuika::prelude::*;
///
/// let rows = vec![Line::from("approve"), Line::from("deny")];
/// let state = SelectState::new();
/// let screen = SelectionScreen::borrowed("Permission", &rows, &state)
///     .leading_rule()
///     .trailing_rule()
///     .footer(KeyHints::new([("enter", "choose"), ("esc", "back")]));
/// # let _ = screen;
/// ```
///
/// ![selection screen demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/selection_screen.png)
pub struct SelectionScreen<'view> {
    rows: Rows<'view>,
    source_window: Option<VirtualWindow>,
    selected: SelectState,
    header: Header<'view>,
    footer: Option<ScopedElement<'view>>,
    leading_rule: bool,
    trailing_rule: bool,
    scrollbar: bool,
    header_style: Option<Style>,
    selection_style: Option<Style>,
}

impl SelectionScreen<'static> {
    /// Build a selection screen that owns its rows.
    pub fn new(
        title: impl Into<Line<'static>>,
        rows: Vec<Line<'static>>,
        state: &SelectState,
    ) -> Self {
        Self::from_rows(title, Rows::Owned(rows), state)
    }
}

impl<'view> SelectionScreen<'view> {
    /// Build a selection screen borrowing an existing row slice.
    pub fn borrowed(
        title: impl Into<Line<'static>>,
        rows: &'view [Line<'static>],
        state: &SelectState,
    ) -> Self {
        Self::from_rows(title, Rows::Borrowed(rows), state)
    }

    /// Build from only the rows in `window` rather than the whole collection.
    ///
    /// `rows` correspond to `window.range()` in order. The screen can shorten
    /// that supplied window to the actual body height without losing absolute
    /// selection or scrollbar geometry.
    pub fn windowed(
        title: impl Into<Line<'static>>,
        rows: &'view [Line<'static>],
        window: VirtualWindow,
        state: &SelectState,
    ) -> Self {
        let mut screen = Self::from_rows(title, Rows::Borrowed(rows), state);
        screen.source_window = Some(window);
        screen
    }

    fn from_rows(title: impl Into<Line<'static>>, rows: Rows<'view>, state: &SelectState) -> Self {
        Self {
            rows,
            source_window: None,
            selected: *state,
            header: Header::Title(title.into()),
            footer: None,
            leading_rule: false,
            trailing_rule: false,
            scrollbar: true,
            header_style: None,
            selection_style: None,
        }
    }

    /// Replace the styled title with any owned or frame-borrowed header view.
    pub fn header<V: View + 'view>(mut self, header: V) -> Self {
        self.header = Header::Custom(element(header));
        self
    }

    /// Add any owned or frame-borrowed footer view, commonly [`KeyHints`](super::KeyHints).
    pub fn footer<V: View + 'view>(mut self, footer: V) -> Self {
        self.footer = Some(element(footer));
        self
    }

    /// Draw a theme-aware rule before the header.
    pub fn leading_rule(mut self) -> Self {
        self.leading_rule = true;
        self
    }

    /// Draw a theme-aware rule between the list and footer.
    pub fn trailing_rule(mut self) -> Self {
        self.trailing_rule = true;
        self
    }

    /// Override the styled title while retaining the active heading role underneath.
    pub fn header_style(mut self, style: Style) -> Self {
        self.header_style = Some(style);
        self
    }

    /// Override the selected row style; the active theme supplies the default.
    pub fn selection_style(mut self, style: Style) -> Self {
        self.selection_style = Some(style);
        self
    }

    /// Show the overflow scrollbar (default true).
    pub fn scrollbar(mut self, show: bool) -> Self {
        self.scrollbar = show;
        self
    }

    fn shell(&self) -> AppShell<'_> {
        let body = match self.source_window {
            Some(window) => SelectRows::windowed(self.rows.as_slice(), window, &self.selected),
            None => SelectRows::borrowed(self.rows.as_slice(), &self.selected),
        }
        .scrollbar(self.scrollbar)
        .selection_style(self.selection_style);
        let mut shell = AppShell::new(body);
        if self.leading_rule {
            shell = shell.top_rule();
        }
        shell = shell
            .header(HeaderView {
                header: &self.header,
                style: self.header_style,
            })
            .top_rule();
        if self.trailing_rule {
            shell = shell.bottom_rule();
        }
        if let Some(footer) = &self.footer {
            shell = shell.footer(ViewRef(footer.as_ref()));
        }
        shell
    }
}

impl View for SelectionScreen<'_> {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        self.shell().measure(available, ctx)
    }

    fn measure_request(&self, request: MeasureRequest, ctx: &RenderCtx) -> Size {
        self.shell().measure_request(request, ctx)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        self.shell().render(area, surface, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;
    use crate::components::{KeyHints, Text};
    use crate::event::{Event, InputOutcome, Key, KeyCode};
    use crate::style::{StyleBundle, StyleSheet};
    use crate::testing::{grid, render, render_sizes, render_with_sheet};
    use crate::ui::{Color, Modifier};

    fn rows(count: usize) -> Vec<Line<'static>> {
        (0..count)
            .map(|index| Line::from(format!("option {index}")))
            .collect()
    }

    #[test]
    fn renders_conventional_chrome_and_borrowed_rows() {
        let rows = rows(3);
        let state = SelectState::new();
        let screen = SelectionScreen::borrowed("Choose agent", &rows, &state)
            .leading_rule()
            .trailing_rule()
            .footer(KeyHints::new([("enter", "select")]));

        assert_eq!(
            grid(&render(&screen, 20, 8, &Theme::default())),
            concat!(
                "────────────────────\n",
                "Choose agent        \n",
                "────────────────────\n",
                "› option 0          \n",
                "  option 1          \n",
                "  option 2          \n",
                "────────────────────\n",
                " enter  select      "
            )
        );
    }

    #[test]
    fn short_body_windows_around_selection() {
        let rows = rows(30);
        let mut state = SelectState::new();
        state.select(Some(24));
        let screen = SelectionScreen::borrowed("Resume session", &rows, &state)
            .footer(KeyHints::new([("enter", "resume")]));

        let rendered = render(&screen, 22, 5, &Theme::default());
        let text = grid(&rendered);
        assert!(
            text.contains("option 24"),
            "selected row was hidden:\n{text}"
        );
        assert!(
            !text.contains("option 0 "),
            "list was not windowed:\n{text}"
        );
        assert!((0..5).any(|y| matches!(rendered[(21, y)].symbol(), "█" | "│")));
    }

    #[test]
    fn host_windowed_rows_preserve_absolute_selection_and_total() {
        let supplied = rows(4)
            .into_iter()
            .enumerate()
            .map(|(local, _)| Line::from(format!("option {}", local + 20)))
            .collect::<Vec<_>>();
        let window = VirtualWindow::new(100, 4, 20);
        let mut state = SelectState::new();
        state.select(Some(22));
        let screen = SelectionScreen::windowed("Agent", &supplied, window, &state);
        let rendered = render(&screen, 16, 5, &Theme::default());
        let text = grid(&rendered);

        assert!(
            text.contains("option 22"),
            "absolute selection was lost:\n{text}"
        );
        assert!((0..5).any(|y| matches!(rendered[(15, y)].symbol(), "█" | "│")));
    }

    #[test]
    fn select_state_input_drives_the_screen_without_parallel_state() {
        let rows = rows(3);
        let mut state = SelectState::new();
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Down)), rows.len()),
            InputOutcome::Changed
        );
        let screen = SelectionScreen::borrowed("Action", &rows, &state);
        let rendered = render(&screen, 16, 5, &Theme::default());

        assert_eq!(rendered[(0, 3)].symbol(), "›");
        assert!(grid(&rendered).contains("option 1"));
    }

    #[test]
    fn empty_narrow_and_tiny_areas_are_stable() {
        let rows = Vec::new();
        let state = SelectState::unselected();
        let screen = SelectionScreen::borrowed("Permission", &rows, &state)
            .leading_rule()
            .trailing_rule()
            .footer(KeyHints::new([("y", "allow"), ("n", "deny")]));

        assert_eq!(
            grid(&render(&screen, 4, 3, &Theme::default())),
            "────\n    \n    "
        );
        let sizes = (0..=16).flat_map(|width| (0..=8).map(move |height| (width, height)));
        assert_eq!(render_sizes(&screen, sizes, &Theme::default()).len(), 153);
    }

    #[test]
    fn heading_rule_selection_and_hints_use_semantic_styles() {
        let theme = Theme::default();
        let sheet = StyleSheet {
            heading: StyleBundle::new().fg(Color::Green).bold(),
            rule: StyleBundle::new().fg(Color::Yellow),
            key_hint_key: StyleBundle::new().fg(Color::Black).bg(Color::Blue),
            ..StyleSheet::from_theme(&theme)
        };
        let state = SelectState::new();
        let screen = SelectionScreen::new("Action", rows(1), &state)
            .leading_rule()
            .selection_style(Style::default().fg(Color::Magenta))
            .footer(KeyHints::new([("q", "quit")]));
        let rendered = render_with_sheet(&screen, 16, 5, &theme, sheet);

        assert_eq!(rendered[(0, 0)].fg, Color::Yellow);
        assert_eq!(rendered[(0, 1)].fg, Color::Green);
        assert!(rendered[(0, 1)].modifier.contains(Modifier::BOLD));
        assert_eq!(rendered[(0, 3)].fg, Color::Magenta);
        assert_eq!(rendered[(1, 4)].bg, Color::Blue);
    }

    #[test]
    fn custom_header_and_footer_may_borrow_frame_data() {
        struct Borrowed<'a>(&'a str);

        impl View for Borrowed<'_> {
            fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
                Size::new(self.0.len().min(usize::from(available.width)) as u16, 1)
            }

            fn render(&self, area: Rect, surface: &mut Surface, _ctx: &RenderCtx) {
                surface.set_string(area.x, area.y, self.0, Style::default());
            }
        }

        let header = String::from("agents");
        let footer = String::from("escape to cancel");
        let state = SelectState::new();
        let screen = SelectionScreen::new("unused", rows(1), &state)
            .header(Borrowed(&header))
            .footer(Borrowed(&footer));

        assert_eq!(
            grid(&render(&screen, 16, 4, &Theme::default())),
            "agents          \n────────────────\n› option 0      \nescape to cancel"
        );
    }

    #[test]
    fn explicit_header_style_overlays_heading_role() {
        let state = SelectState::new();
        let screen = SelectionScreen::new("Action", rows(1), &state)
            .header_style(Style::default().fg(Color::Cyan));
        let rendered = render(&screen, 12, 3, &Theme::default());

        assert_eq!(rendered[(0, 0)].fg, Color::Cyan);
        assert!(rendered[(0, 0)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn owned_rows_and_hidden_scrollbar_are_supported() {
        let mut state = SelectState::new();
        state.select(Some(9));
        let screen = SelectionScreen::new("Agent", rows(10), &state).scrollbar(false);
        let rendered = render(&screen, 14, 4, &Theme::default());

        assert!(grid(&rendered).contains("option 9"));
        assert!((0..4).all(|y| !matches!(rendered[(13, y)].symbol(), "█" | "│")));
    }

    #[test]
    fn ordinary_text_view_remains_usable_as_custom_header() {
        let state = SelectState::new();
        let screen = SelectionScreen::new("unused", rows(1), &state).header(Text::raw("custom"));
        assert!(grid(&render(&screen, 10, 3, &Theme::default())).starts_with("custom"));
    }

    #[test]
    fn agf_shaped_example_keeps_exact_caller_loc_comparison() {
        fn lines_between(source: &str, start: &str, end: &str) -> usize {
            source
                .split_once(start)
                .unwrap()
                .1
                .split_once(end)
                .unwrap()
                .0
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count()
        }

        let source = include_str!("../../examples/selection_screen.rs");
        assert_eq!(
            lines_between(source, "// BEFORE CALLER START", "// BEFORE CALLER END"),
            8
        );
        assert_eq!(
            lines_between(source, "// AFTER CALLER START", "// AFTER CALLER END"),
            4
        );
    }
}
