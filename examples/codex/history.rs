//! Transcript cells — the history the Codex TUI prints above its composer.
//!
//! Most of the history is rows of text — one `•` bullet per event, details
//! indented under a `└` — but not all of it: the session banner and `/status`
//! are **bordered panels**, laid out rather than drawn as glyphs in strings.
//!
//! So a cell renders to an [`Element`], not to lines, and the transcript is an
//! [`ItemScroll`](tuika::ItemScroll) over those elements. Line-shaped cells wrap
//! their rows in a `Text`; panel-shaped ones return a real `Boxed`. A streamed
//! answer still keeps a [`MarkdownState`] so only its in-flight tail re-parses.

use tuika::ui::{Line, Span};
use tuika::ui::{Modifier, Style};

use tuika::components::text::wrap_lines;
use tuika::prelude::*;

/// Rows of command output kept inline before the middle is elided. Codex shows a
/// short window and a `… +N lines` marker rather than flooding the transcript.
const MAX_OUTPUT_ROWS: usize = 6;

/// How a shell command ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecStatus {
    Running,
    Ok,
    Failed(i32),
    Rejected,
}

/// Severity of a local notice (slash-command output, interruptions, errors).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Info,
    Warn,
    Error,
}

/// One item in the transcript.
pub enum Cell {
    /// The session header Codex prints on startup.
    Banner {
        version: String,
        rows: Vec<(String, String)>,
        tips: Vec<(String, String)>,
    },
    /// A labeled key/value block — `/status` output.
    Config {
        title: String,
        rows: Vec<(String, String)>,
    },
    /// The user's own message, echoed into the transcript.
    User(String),
    /// A reasoning summary, streamed in as the model "thinks".
    Reasoning { body: String },
    /// Read-only exploration: reads, searches, listings.
    Explore { items: Vec<String> },
    /// A shell command and a window of its output.
    Exec {
        command: String,
        output: Vec<String>,
        status: ExecStatus,
    },
    /// A patch: a file edit, or the working-tree diff from `/diff`.
    Patch { header: String, hunk: Vec<String> },
    /// The assistant's answer, streamed as markdown.
    ///
    /// Boxed because `MarkdownState` carries the parse/flatten caches and would
    /// otherwise dominate the size of every other variant.
    Answer(Box<MarkdownState>),
    /// A local notice: interruptions, rejections, slash-command acknowledgements.
    Notice {
        tone: Tone,
        title: String,
        body: Vec<String>,
    },
}

impl Cell {
    /// Render this cell as a transcript item, laid out to `width`.
    pub fn view(&mut self, width: u16, theme: &Theme, sheet: &StyleSheet) -> Element {
        match self {
            Cell::Banner {
                version,
                rows,
                tips,
            } => banner(version, rows, tips, theme),
            Cell::Config { title, rows } => config(title, rows, theme),
            other => element(Text::new(other.lines(width, theme, sheet))),
        }
    }

    /// Render a line-shaped cell to transcript rows, wrapped to `width`.
    fn lines(&mut self, width: u16, theme: &Theme, sheet: &StyleSheet) -> Vec<Line<'static>> {
        match self {
            Cell::User(text) => user(text, width, theme),
            Cell::Reasoning { body } => reasoning(body, width, theme),
            Cell::Explore { items } => explore(items, theme),
            Cell::Exec {
                command,
                output,
                status,
            } => exec(command, output, *status, theme),
            Cell::Patch { header, hunk } => patch(header, hunk, theme),
            Cell::Answer(state) => answer(state, width, theme, sheet),
            Cell::Notice { tone, title, body } => notice(*tone, title, body, width, theme),
            // Panel-shaped cells are handled by `view` and never reach here.
            Cell::Banner { .. } | Cell::Config { .. } => Vec::new(),
        }
    }
}

/// Wrap `child` so it takes only the width it measures, instead of stretching
/// to the transcript's — how Codex's own panels sit against the left margin.
fn shrink_to_fit(child: Element) -> Element {
    element(Flex::column().align(Align::Start).auto(child))
}

/// `• ` — the marker Codex puts in front of every event it reports.
fn bullet(theme: &Theme) -> Span<'static> {
    Span::styled("• ", Style::default().fg(theme.accent))
}

/// A cell header: the bullet plus a bold title and optional trailing detail.
fn header(title: &str, detail: Option<Span<'static>>, theme: &Theme) -> Line<'static> {
    let mut spans = vec![
        bullet(theme),
        Span::styled(
            title.to_string(),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
    ];
    spans.extend(detail);
    Line::from(spans)
}

/// Indent detail rows under a header: `  └ ` on the first, `    ` on the rest.
fn detail(rows: Vec<Line<'static>>, theme: &Theme) -> Vec<Line<'static>> {
    rows.into_iter()
        .enumerate()
        .map(|(i, line)| {
            let lead = if i == 0 { "  └ " } else { "    " };
            let mut spans = vec![Span::styled(lead, Style::default().fg(theme.dim))];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}

/// Prefix every row with `pad` columns of blank.
fn indent(rows: Vec<Line<'static>>, pad: usize) -> Vec<Line<'static>> {
    rows.into_iter()
        .map(|line| {
            let mut spans = vec![Span::raw(" ".repeat(pad))];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}

/// `label:` / `value` rows, aligned on the widest label.
fn kv_rows(rows: &[(String, String)], theme: &Theme) -> Vec<Line<'static>> {
    let pad = rows
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(0);
    rows.iter()
        .map(|(k, v)| {
            Line::from(vec![
                Span::styled(
                    format!("{k}:{:width$} ", "", width = pad - k.chars().count()),
                    theme.muted_style(),
                ),
                Span::styled(v.clone(), Style::default().fg(theme.text)),
            ])
        })
        .collect()
}

fn banner(
    version: &str,
    rows: &[(String, String)],
    tips: &[(String, String)],
    theme: &Theme,
) -> Element {
    let mut panel = vec![Line::from(vec![
        Span::styled(
            ">_ ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("OpenAI Codex (v{version})"),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
    ])];
    panel.push(Line::default());
    panel.extend(kv_rows(rows, theme));

    let pad = tips
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(0);
    let mut tip_rows = vec![
        Line::default(),
        Line::from(Span::styled(
            "To get started, describe a task or try one of these commands:",
            theme.muted_style(),
        )),
        Line::default(),
    ];
    tip_rows.extend(tips.iter().map(|(cmd, blurb)| {
        Line::from(vec![
            Span::styled(
                format!("{cmd}{:width$}  ", "", width = pad - cmd.chars().count()),
                Style::default().fg(theme.accent_alt),
            ),
            Span::styled(blurb.clone(), theme.muted_style()),
        ])
    }));

    element(
        Flex::column()
            .auto(shrink_to_fit(element(
                Boxed::new(element(Text::new(panel)))
                    .border(BorderStyle::Rounded)
                    .border_color(theme.border)
                    .padding(Padding::symmetric(1, 0)),
            )))
            .auto(element(Text::new(tip_rows))),
    )
}

fn config(title: &str, rows: &[(String, String)], theme: &Theme) -> Element {
    let panel = Boxed::new(element(Text::new(kv_rows(rows, theme))))
        .title(Line::from(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )))
        .border(BorderStyle::Rounded)
        .border_color(theme.border)
        .padding(Padding::symmetric(1, 0));
    shrink_to_fit(element(panel))
}

fn user(text: &str, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let body: Vec<Line<'static>> = text
        .lines()
        .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(theme.text))))
        .collect();
    wrap_lines(&body, width.saturating_sub(2))
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let lead = if i == 0 { "› " } else { "  " };
            let mut spans = vec![Span::styled(lead, Style::default().fg(theme.accent_alt))];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}

fn reasoning(body: &str, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let style = theme.muted_style().add_modifier(Modifier::ITALIC);
    let rows: Vec<Line<'static>> = body
        .split('\n')
        .map(|l| Line::from(Span::styled(l.to_string(), style)))
        .collect();
    let mut out = vec![header("Thinking", None, theme)];
    out.extend(indent(wrap_lines(&rows, width.saturating_sub(2)), 2));
    out
}

fn explore(items: &[String], theme: &Theme) -> Vec<Line<'static>> {
    let rows: Vec<Line<'static>> = items
        .iter()
        .map(|item| {
            // `Read src/markdown.rs` — the verb reads as the action, the rest as
            // its target, so they are styled apart.
            let (verb, rest) = item.split_once(' ').unwrap_or((item.as_str(), ""));
            Line::from(vec![
                Span::styled(format!("{verb} "), Style::default().fg(theme.text)),
                Span::styled(rest.to_string(), theme.muted_style()),
            ])
        })
        .collect();
    let mut out = vec![header("Explored", None, theme)];
    out.extend(detail(rows, theme));
    out
}

fn exec(command: &str, output: &[String], status: ExecStatus, theme: &Theme) -> Vec<Line<'static>> {
    let verb = match status {
        ExecStatus::Running => "Running",
        _ => "Ran",
    };
    let trailing = match status {
        ExecStatus::Failed(code) => Some(Span::styled(
            format!("  (exit {code})"),
            Style::default().fg(theme.accent),
        )),
        ExecStatus::Rejected => Some(Span::styled(
            "  (rejected)".to_string(),
            Style::default().fg(theme.accent),
        )),
        _ => None,
    };
    let mut out = vec![Line::from({
        let mut spans = vec![
            bullet(theme),
            Span::styled(
                format!("{verb} "),
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
            Span::styled(command.to_string(), Style::default().fg(theme.accent_alt)),
        ];
        spans.extend(trailing);
        spans
    })];

    // Long output is elided in the middle, like Codex's own output window.
    let mut rows: Vec<Line<'static>> = Vec::new();
    if output.len() > MAX_OUTPUT_ROWS {
        let head = MAX_OUTPUT_ROWS / 2;
        let tail = output.len() - (MAX_OUTPUT_ROWS - head);
        for line in &output[..head] {
            rows.push(Line::from(Span::styled(line.clone(), theme.muted_style())));
        }
        rows.push(Line::from(Span::styled(
            format!("… +{} lines", tail - head),
            Style::default().fg(theme.dim),
        )));
        for line in &output[tail..] {
            rows.push(Line::from(Span::styled(line.clone(), theme.muted_style())));
        }
    } else {
        rows.extend(
            output
                .iter()
                .map(|line| Line::from(Span::styled(line.clone(), theme.muted_style()))),
        );
    }
    out.extend(detail(rows, theme));
    out
}

fn patch(header_text: &str, hunk: &[String], theme: &Theme) -> Vec<Line<'static>> {
    let rows: Vec<Line<'static>> = hunk
        .iter()
        .map(|line| {
            let style = match line.chars().next() {
                Some('+') => Style::default().fg(theme.code.string),
                Some('-') => Style::default().fg(theme.accent),
                Some('@') => Style::default().fg(theme.code.link),
                _ => theme.muted_style(),
            };
            Line::from(Span::styled(line.clone(), style))
        })
        .collect();
    let mut out = vec![header(header_text, None, theme)];
    out.extend(detail(rows, theme));
    out
}

fn answer(
    state: &mut MarkdownState,
    width: u16,
    theme: &Theme,
    sheet: &StyleSheet,
) -> Vec<Line<'static>> {
    // The markdown is wrapped two columns narrow so the bullet gutter fits.
    let body = state
        .lines(
            width.saturating_sub(2),
            theme,
            sheet,
            CodeHighlighter::Plain,
        )
        .to_vec();
    indent(body, 2)
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            if i > 0 {
                return line;
            }
            // Replace the first row's indent with the bullet.
            let mut spans = vec![bullet(theme)];
            spans.extend(line.spans.into_iter().skip(1));
            Line::from(spans)
        })
        .collect()
}

fn notice(
    tone: Tone,
    title: &str,
    body: &[String],
    width: u16,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let (glyph, color) = match tone {
        Tone::Info => ("• ", theme.accent),
        Tone::Warn => ("⚠ ", theme.accent_alt),
        Tone::Error => ("✗ ", theme.accent),
    };
    let mut out = vec![Line::from(vec![
        Span::styled(glyph, Style::default().fg(color)),
        Span::styled(
            title.to_string(),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
    ])];
    let rows: Vec<Line<'static>> = body
        .iter()
        .map(|l| Line::from(Span::styled(l.clone(), theme.muted_style())))
        .collect();
    out.extend(indent(wrap_lines(&rows, width.saturating_sub(2)), 2));
    out
}
