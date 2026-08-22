//! A polished application shell composed from tuika views. Run with
//! `cargo run --example workbench_demo`.
//!
//! The scene is deterministic and offline. `Flex` owns the layout, every
//! visible panel is a bordered `Boxed`, and the content uses tuika's table,
//! code, chart, tab, rule, and status components. Use `←`/`→` to move between
//! tabs, `↑`/`↓` to move through the file tree, click a file to select it, and
//! `q`/`Esc` to quit.

use std::time::Duration;

use tuika::prelude::*;
use tuika::probe::RectProbe;
use tuika::ui::Alignment;
use tuika_charts::{Axis as ChartAxis, Chart, Domain, Point, Series};
use tuika_codeformatters::TreeSitterHighlighter;

#[path = "../support/mod.rs"]
mod support;

const COLS: u16 = 96;
const ROWS: u16 = 27;
const TAB_COUNT: usize = 4;
const FILES: [(&str, &str, bool); 7] = [
    ("▾", "src", false),
    ("◈", "app.rs", true),
    ("◈", "ui.rs", true),
    ("◈", "components.rs", true),
    ("▸", "examples", false),
    ("▱", "README.md", false),
    ("▱", "Cargo.toml", false),
];
const FILE_COUNT: usize = FILES.len();

const BG: Color = Color::Rgb(22, 18, 21);
const SURFACE: Color = Color::Rgb(31, 24, 27);
const TEXT: Color = Color::Rgb(222, 204, 199);
const MUTED: Color = Color::Rgb(151, 119, 115);
const DIM: Color = Color::Rgb(92, 65, 65);
const COPPER: Color = Color::Rgb(175, 91, 59);
const COPPER_SOFT: Color = Color::Rgb(119, 59, 48);
const PEACH: Color = Color::Rgb(221, 139, 101);
const PLUM: Color = Color::Rgb(189, 143, 179);
const PLUM_DARK: Color = Color::Rgb(78, 47, 73);
const GREEN: Color = Color::Rgb(164, 178, 132);
const BLUE: Color = Color::Rgb(128, 155, 177);

const CODE: &str = r#"use tuika::prelude::*;

#[tuika::main]
async fn main() -> anyhow::Result<()> {
    App::default()
        .title("workbench demo")
        .layout(Layout! {
            rows: [
                header(3),
                body: {
                    columns: [sidebar(24), content(), panel(30)]
                },
                footer(1),
            ]
        })
}"#;

const REQUESTS: &[(f64, f64)] = &[
    (0.0, 240.0),
    (1.0, 360.0),
    (2.0, 290.0),
    (3.0, 430.0),
    (4.0, 520.0),
    (5.0, 470.0),
    (6.0, 650.0),
    (7.0, 390.0),
    (8.0, 310.0),
    (9.0, 530.0),
    (10.0, 610.0),
    (11.0, 520.0),
    (12.0, 640.0),
    (13.0, 590.0),
    (14.0, 430.0),
    (15.0, 520.0),
    (16.0, 760.0),
    (17.0, 810.0),
    (18.0, 610.0),
    (19.0, 550.0),
    (20.0, 850.0),
    (21.0, 620.0),
    (22.0, 570.0),
    (23.0, 780.0),
    (24.0, 610.0),
];

static HIGHLIGHTER: TreeSitterHighlighter = TreeSitterHighlighter;

struct App {
    tabs: TabSelectState,
    files: SelectState,
}

impl Default for App {
    fn default() -> Self {
        let mut files = SelectState::new();
        files.select(Some(1));
        Self {
            tabs: TabSelectState::new(),
            files,
        }
    }
}

#[derive(Clone, Copy)]
struct DashboardState {
    tab: usize,
    file: usize,
}

impl From<&App> for DashboardState {
    fn from(app: &App) -> Self {
        Self {
            tab: app.tabs.selected(),
            file: app.files.selected().unwrap_or(0),
        }
    }
}

fn main() -> std::io::Result<()> {
    let cli = support::Cli::parse()?;
    let theme = if cli.theme_name.is_some() {
        cli.theme
    } else {
        theme()
    };
    let args = cli.args;

    if args.first().map(String::as_str) == Some("--dump") {
        let root = scene(
            DashboardState::from(&App::default()),
            &RectProbe::new(),
            theme,
        );
        let buffer = tuika::testing::render(root.as_ref(), COLS, ROWS, &theme);
        println!("{}", tuika::testing::grid(&buffer));
        return Ok(());
    }

    let runner = Runner::new(runner_config());
    let files = RectProbe::new();
    let render_files = files.clone();
    runner.run(
        &theme,
        from_fn(
            &mut App::default(),
            move |app, _frame| scene(DashboardState::from(app), &render_files, theme),
            move |app, signal| match signal {
                Signal::Event(event) => update(app, &event, files.rect()),
                _ => UpdateResult::Clean,
            },
        ),
    )
}

fn runner_config() -> RunnerConfig {
    RunnerConfig {
        tick_rate: Duration::from_millis(100),
        screen_mode: ScreenMode::Alternate.with_mouse_capture(),
    }
}

fn update(app: &mut App, event: &Event, files: Rect) -> UpdateResult {
    if matches!(
        event,
        Event::Key(key)
            if key.plain() && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
    ) {
        return UpdateResult::Exit;
    }

    if let Event::Key(key) = event
        && key.plain()
        && let KeyCode::Char(number @ '1'..='4') = key.code
    {
        app.tabs.select(number as usize - '1' as usize, TAB_COUNT);
        return UpdateResult::Dirty;
    }

    let before_tab = app.tabs.selected();
    let tabs = app.tabs.handle(event, TAB_COUNT);
    if tabs.consumed() {
        return if app.tabs.selected() != before_tab {
            UpdateResult::Dirty
        } else {
            UpdateResult::Consumed
        };
    }

    let before_file = app.files.selected();
    let file_input = if matches!(event, Event::Mouse(_)) {
        app.files.handle_mouse(event, FILE_COUNT, files, 0)
    } else {
        app.files.handle(event, FILE_COUNT)
    };
    if file_input.consumed() {
        if app.files.selected() != before_file {
            UpdateResult::Dirty
        } else {
            UpdateResult::Consumed
        }
    } else {
        UpdateResult::Clean
    }
}

fn theme() -> Theme {
    Theme {
        background: BG,
        surface: SURFACE,
        text: TEXT,
        muted: MUTED,
        dim: DIM,
        accent: COPPER,
        accent_alt: PLUM,
        border: COPPER_SOFT,
        border_focused: COPPER,
        selection_bg: PLUM_DARK,
        selection_fg: Color::Rgb(240, 217, 207),
        code: CodeTheme {
            heading: TEXT,
            link: BLUE,
            background: BG,
            text: TEXT,
            label: MUTED,
            keyword: PEACH,
            function: BLUE,
            type_name: Color::Rgb(190, 156, 116),
            constant: PLUM,
            string: GREEN,
            comment: MUTED,
            punctuation: Color::Rgb(192, 151, 137),
        },
    }
}

fn scene(state: DashboardState, files: &RectProbe, theme: Theme) -> Element {
    let dashboard = dashboard(state, files, theme);
    element(view_fn(
        |available, _ctx| available,
        move |area, surface, ctx| {
            if area.width < 76 || area.height < 20 {
                let message = Boxed::new(
                    Paragraph::new(
                        "This showcase needs at least 76 × 20 cells.\nResize the terminal · q to quit",
                        ctx.theme.muted_style(),
                    )
                    .alignment(Alignment::Center),
                )
                .title(" workbench-demo ")
                .border_color(theme.accent)
                .padding(Padding::all(1));
                message.render(area, surface, ctx);
            } else {
                dashboard.render(area, surface, ctx);
            }
        },
    ))
}

fn dashboard(state: DashboardState, files: &RectProbe, theme: Theme) -> Element {
    let body = Flex::column()
        .fixed(2, header(state.tab, theme))
        .grow(1, main_panels(state.file, files, theme))
        .fixed(2, footer(theme));

    element(
        Boxed::new(element(body))
            .title(Line::from(vec![
                Span::styled(" ● ", Style::default().fg(theme.accent)),
                Span::styled(
                    "workbench-demo ",
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                ),
            ]))
            .border_color(theme.accent)
            .padding(Padding::ZERO)
            .background(Style::default().bg(theme.background)),
    )
}

fn header(selected: usize, theme: Theme) -> Element {
    let state = TabSelectState::with_selected(selected);
    let tabs = TabSelect::new(
        [" Overview ", " Requests ", " Metrics ", " Logs "]
            .into_iter()
            .map(Line::from)
            .collect(),
        &state,
    );
    let tabs_row = Flex::row()
        .grow(1, element(Spacer))
        .auto(element(tabs))
        .fixed(
            3,
            element(Text::new(vec![
                Line::from(Span::styled("≡", Style::default().fg(theme.accent_alt)))
                    .right_aligned(),
            ])),
        );
    element(Flex::column().fixed(1, element(tabs_row)).fixed(
        1,
        element(Rule::new().style(Style::default().fg(theme.dim))),
    ))
}

fn main_panels(selected: usize, files: &RectProbe, theme: Theme) -> Element {
    element(
        Flex::row()
            .gap(1)
            .fixed(17, sidebar(selected, files, theme))
            .grow(1, code_panel(theme))
            .fixed(29, metrics(theme)),
    )
}

fn sidebar(selected: usize, files: &RectProbe, theme: Theme) -> Element {
    element(
        Flex::column()
            .grow(1, files_panel(selected, files, theme))
            .fixed(8, keymaps_panel(theme)),
    )
}

fn panel(title: &str, child: Element, theme: Theme) -> Element {
    element(
        Boxed::new(child)
            .title(panel_title(title, theme))
            .border_color(theme.border)
            .padding(Padding::ZERO)
            .background(Style::default().bg(theme.surface)),
    )
}

fn panel_title(title: &str, theme: Theme) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {title} "),
        Style::default()
            .fg(theme.muted)
            .add_modifier(Modifier::BOLD),
    ))
}

fn files_panel(selected: usize, probe: &RectProbe, theme: Theme) -> Element {
    let mut state = SelectState::new();
    state.select(Some(selected));
    let rows = FILES
        .iter()
        .map(|(icon, name, child)| {
            vec![Line::from(vec![
                Span::styled(format!("{icon} "), Style::default().fg(theme.accent_alt)),
                Span::styled(
                    (*name).to_string(),
                    Style::default().fg(if *child { theme.text } else { theme.muted }),
                ),
            ])]
        })
        .collect();
    let table = Table::new(vec![Column::flex("", 1)], rows, &state)
        .header(false)
        .gutter(false)
        .scrollbar(false)
        .gap(0)
        .preserve_selection_fg(true);
    let content = probe.wrap(table);
    element(
        Boxed::new(content)
            .title(panel_title("FILES", theme))
            .border_color(theme.border)
            .padding(Padding::ZERO)
            .background(Style::default().bg(theme.surface)),
    )
}

fn keymaps_panel(theme: Theme) -> Element {
    let rows = [
        ("q", "Quit"),
        ("↑/↓", "Files"),
        ("←/→", "Tabs"),
        ("click", "Select"),
    ]
    .into_iter()
    .map(|(key, label)| {
        vec![
            Line::from(Span::styled(key, Style::default().fg(theme.accent_alt))),
            Line::from(Span::styled(label, Style::default().fg(theme.muted))),
        ]
    })
    .collect();
    let table = Table::new(
        vec![Column::fixed("", 5), Column::flex("", 1)],
        rows,
        &SelectState::unselected(),
    )
    .header(false)
    .gutter(false)
    .scrollbar(false)
    .gap(1);
    panel(
        "KEYMAPS",
        element(
            Boxed::new(element(table))
                .border(BorderStyle::None)
                .padding(Padding::symmetric(1, 0)),
        ),
        theme,
    )
}

fn code_panel(theme: Theme) -> Element {
    element(
        Boxed::new(element(
            CodeBlock::new("rust", CODE)
                .highlighter(&HIGHLIGHTER)
                .label(false)
                .line_numbers(true),
        ))
        .border_color(theme.border)
        .padding(Padding::symmetric(1, 0))
        .background(Style::default().bg(theme.background)),
    )
}

fn metrics(theme: Theme) -> Element {
    element(
        Flex::column()
            .fixed(12, requests_panel(theme))
            .grow(1, status_panel(theme)),
    )
}

fn requests_panel(theme: Theme) -> Element {
    let points = REQUESTS
        .iter()
        .map(|&(x, y)| Point::new(x, y))
        .collect::<Vec<_>>();
    let chart = Chart::new()
        .legend(false)
        .x_domain(Domain::new(0.0, 24.0).expect("fixed request domain"))
        .y_domain(Domain::new(0.0, 1200.0).expect("fixed request domain"))
        .x_axis(
            ChartAxis::new()
                .ticks(4)
                .format(|value| format!("{value:02.0}:00")),
        )
        .y_axis(ChartAxis::new().ticks(3).format(|value| {
            if value >= 1000.0 {
                format!("{:.1}k", value / 1000.0)
            } else {
                format!("{value:.0}")
            }
        }))
        .series(Series::area("requests", points).color(theme.accent_alt));
    panel("REQUESTS (24H)", element(chart), theme)
}

fn status_panel(theme: Theme) -> Element {
    let rows = [
        ("Uptime", "12h 48m", theme.text),
        ("Requests", "18,426", theme.text),
        ("Errors", "23", theme.accent),
        ("P95 Latency", "142ms", theme.accent_alt),
    ]
    .into_iter()
    .map(|(label, value, color)| {
        vec![
            Line::from(Span::styled(label, Style::default().fg(color))),
            Line::from(Span::styled(value, Style::default().fg(color))),
        ]
    })
    .collect();
    let table = Table::new(
        vec![Column::flex("", 1), Column::fixed("", 8)],
        rows,
        &SelectState::unselected(),
    )
    .header(false)
    .gutter(false)
    .scrollbar(false)
    .gap(1);
    panel(
        "STATUS",
        element(
            Boxed::new(element(table))
                .border(BorderStyle::None)
                .padding(Padding::symmetric(1, 0)),
        ),
        theme,
    )
}

fn footer(theme: Theme) -> Element {
    let status = StatusBar::new()
        .left(vec![
            Span::styled(
                " NORMAL ",
                Style::default()
                    .fg(theme.selection_fg)
                    .bg(theme.selection_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "   Connected to localhost:8080",
                Style::default().fg(theme.muted),
            ),
        ])
        .right(vec![Span::styled(
            "rustc 1.94.0  •  ↑  •  13:37:42 ",
            Style::default().fg(theme.muted),
        )])
        .background(Style::default().bg(theme.background));
    element(
        Flex::column()
            .fixed(
                1,
                element(Rule::new().style(Style::default().fg(theme.dim))),
            )
            .fixed(1, element(status)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuika::event::{Mouse, MouseButton, MouseKind};

    #[test]
    fn recorded_frame_contains_the_dashboard_landmarks() {
        let root = scene(
            DashboardState::from(&App::default()),
            &RectProbe::new(),
            theme(),
        );
        let buffer = tuika::testing::render(root.as_ref(), COLS, ROWS, &theme());
        let grid = tuika::testing::grid(&buffer);
        for landmark in [
            "workbench-demo",
            "FILES",
            "REQUESTS (24H)",
            "P95 Latency",
            "NORMAL",
        ] {
            assert!(grid.contains(landmark), "missing {landmark:?}\n{grid}");
        }
    }

    #[test]
    fn compact_frame_explains_its_minimum_size() {
        let root = scene(
            DashboardState::from(&App::default()),
            &RectProbe::new(),
            theme(),
        );
        let buffer = tuika::testing::render(root.as_ref(), 50, 12, &theme());
        assert!(tuika::testing::grid(&buffer).contains("at least 76 × 20"));
    }

    #[test]
    fn clicking_a_file_row_selects_it() {
        let mut app = App::default();
        let files = RectProbe::new();
        let root = scene(DashboardState::from(&app), &files, theme());
        let _ = tuika::testing::render(root.as_ref(), COLS, ROWS, &theme());
        let bounds = files.rect();
        let click = Event::Mouse(Mouse::at(
            MouseKind::Down(MouseButton::Left),
            bounds.x + 2,
            bounds.y + 3,
        ));

        assert_eq!(update(&mut app, &click, bounds), UpdateResult::Dirty);
        assert_eq!(app.files.selected(), Some(3));
        assert!(runner_config().screen_mode.captures_mouse());
    }
}
