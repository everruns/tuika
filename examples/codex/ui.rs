//! The view tree: transcript, working indicator, popup, composer, footer.
//!
//! The whole screen is one column. The transcript grows, everything below it is
//! measured first and pinned — which is what makes a chat TUI feel right: the
//! composer never moves, and new output pushes the history up instead of the
//! input down. The bottom stack is built with the builder API because its
//! children are conditional; the pieces themselves use the `view!` DSL.

use std::io;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use tuika::prelude::*;
use tuika::probe::RectProbe;

use crate::app::{App, Popup};

/// Columns of blank Codex keeps down each side of its UI.
pub const GUTTER: u16 = 1;
/// Visual rows the composer grows to before it scrolls internally.
const MAX_COMPOSER_ROWS: u16 = 6;
/// Rows the completion popup shows before it windows around the selection.
const MAX_POPUP_ROWS: usize = 8;
/// Blank rows between two transcript items.
const TRANSCRIPT_GAP: u16 = 1;

/// Build the frame. Takes `&mut App` because the transcript's streaming markdown
/// re-renders through its own cache, and because the scroll offset is reconciled
/// against this frame's geometry.
pub fn build(
    app: &mut App,
    area: Rect,
    theme: &Theme,
    sheet: &StyleSheet,
    probe: &RectProbe,
) -> Element {
    let width = area.width.saturating_sub(GUTTER * 2);

    // Measure the pinned bottom stack first; the transcript takes what is left.
    let popup_items = app.popup_items();
    let popup_h = if popup_items.is_empty() {
        0
    } else {
        popup_items.len().min(MAX_POPUP_ROWS) as u16 + 1
    };
    let working_h = if app.agent.is_running() { 2 } else { 0 };
    let body_h = if app.approval.is_some() {
        7
    } else {
        let rows = app
            .composer
            .visual_height(width.saturating_sub(4))
            .clamp(1, MAX_COMPOSER_ROWS);
        rows + 2
    };
    let bottom_h = working_h + popup_h + body_h + 1;
    let transcript_h = area.height.saturating_sub(bottom_h).max(1);

    // Reconcile the scroll offset with this frame's dimensions, then stash them
    // so `PageUp`/`PageDown` have geometry to work against before the next one.
    // Item heights are measured the way the viewport will measure them — same
    // width, same scrollbar setting — so the offset can't drift from the paint.
    let items = app.transcript(width, theme, sheet);
    let ctx = RenderCtx::new(theme).with_sheet(*sheet);
    app.content_h = ItemScroll::measure_height(&items, width, TRANSCRIPT_GAP, true, &ctx);
    app.viewport_h = transcript_h as usize;
    app.scroll.clamp(app.content_h, app.viewport_h);

    let mut root = Flex::column()
        .padding(Padding::symmetric(GUTTER, 0))
        .background(Style::default().bg(theme.background))
        .grow(
            1,
            element(ItemScroll::new(items, &app.scroll).gap(TRANSCRIPT_GAP)),
        );
    if working_h > 0 {
        root = root.fixed(working_h, working(app, theme));
    }
    if popup_h > 0 {
        root = root.fixed(popup_h, popup(app, &popup_items, theme));
    }
    root = root.fixed(
        body_h,
        match &app.approval {
            Some(_) => approval(app, theme),
            None => composer(app, theme, probe),
        },
    );
    root = root.fixed(1, footer(app, theme));
    element(root)
}

/// `⠹ Working (12s • Esc to interrupt)` — the row Codex shows under the
/// transcript while a turn is in flight.
fn working(app: &App, theme: &Theme) -> Element {
    let label = Line::from(vec![
        Span::styled(
            "Working",
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" ({}s • Esc to interrupt)", app.elapsed_secs()),
            theme.muted_style(),
        ),
    ]);
    let row = view! {
        row(gap = 1) {
            fixed(1) { node(Spinner::new(app.frame).color(theme.accent)) }
            grow(1) { node(Text::new(vec![label])) }
        }
    };
    element(Flex::column().fixed(1, element(Spacer)).fixed(1, row))
}

/// The command picker: slash completion, `/model`, `/approvals`.
fn popup(app: &App, items: &[(String, String)], theme: &Theme) -> Element {
    let pad = items
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0);
    let rows: Vec<Line<'static>> = items
        .iter()
        .map(|(label, blurb)| {
            Line::from(vec![
                Span::styled(
                    format!(
                        "{label}{:width$}  ",
                        "",
                        width = pad - label.chars().count()
                    ),
                    Style::default().fg(theme.accent_alt),
                ),
                Span::styled(blurb.clone(), theme.muted_style()),
            ])
        })
        .collect();
    let state = match &app.popup {
        Some(Popup::Token { state, .. } | Popup::Model(state) | Popup::Approvals(state)) => *state,
        None => return element(Spacer),
    };
    let list = SelectList::new(rows, &state).viewport(MAX_POPUP_ROWS as u16);
    element(
        Flex::column()
            .fixed(1, element(Spacer))
            .grow(1, element(list)),
    )
}

/// The rounded input box, with the caret placed by the host through `probe`.
fn composer(app: &App, theme: &Theme, probe: &RectProbe) -> Element {
    // The input paints its own placeholder and its own token highlights: the
    // ranges come from the host's trigger table, the painting from tuika.
    let input = element(
        TextInput::new(&app.composer)
            .style(Style::default().fg(theme.text))
            .placeholder("Ask Codex to do something", Style::default().fg(theme.dim))
            .highlights(app.composer_highlights(theme)),
    );
    let prompt = Text::new(vec![Line::from(Span::styled(
        "› ",
        Style::default().fg(theme.accent_alt),
    ))]);
    view! {
        boxed(border = BorderStyle::Rounded, border_color = theme.border,
              padding = Padding::symmetric(1, 0)) {
            row {
                fixed(2) { node(prompt) }
                grow(1) { node(probe.wrap(input)) }
            }
        }
    }
}

/// The escalation prompt: a command the sandbox will not run unattended.
fn approval(app: &App, theme: &Theme) -> Element {
    let Some(pending) = &app.approval else {
        return element(Spacer);
    };
    let program = pending
        .command
        .split_whitespace()
        .next()
        .unwrap_or(&pending.command)
        .to_string();
    let command = Text::new(vec![
        Line::from(vec![
            Span::styled("$ ", theme.muted_style()),
            Span::styled(
                pending.command.clone(),
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::default(),
    ]);
    let options = vec![
        Line::from("1. Yes, run it".to_string()),
        Line::from(format!(
            "2. Yes, and don't ask again for `{program}` this session"
        )),
        Line::from("3. No, and tell Codex what to do instead".to_string()),
    ];
    let list = SelectList::new(options, &pending.state).scrollbar(false);
    let title = Line::from(Span::styled(
        " Codex wants to run a command ",
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    ));
    view! {
        boxed(title = title, border = BorderStyle::Rounded, border_color = theme.accent,
              padding = Padding::symmetric(1, 0)) {
            col {
                fixed(2) { node(command) }
                grow(1) { node(list) }
            }
        }
    }
}

/// The key hints and the context meter Codex prints under its composer.
fn footer(app: &App, theme: &Theme) -> Element {
    let hints = if app.approval.is_some() {
        "  1-3 choose   ↑↓ move   ⏎ confirm   esc reject"
    } else if app.popup.is_some() {
        "  ↑↓ move   ⇥ complete   ⏎ run   esc dismiss"
    } else {
        // `PgUp` spelled out rather than `⇞⇟`: those two glyphs are missing from
        // most terminal fonts and land as replacement boxes.
        "  ⏎ send   ⇧⏎ newline   PgUp scroll   ⌃C quit"
    };
    let bar = StatusBar::new()
        .left(vec![Span::styled(hints, theme.muted_style())])
        .right(vec![Span::styled(
            format!("{}% context left  ", app.session.context_left()),
            Style::default().fg(theme.dim),
        )]);
    element(bar)
}

/// Render canned frames as text — no terminal, no recorder. Handy for a PR
/// before/after, and it keeps the example runnable in CI-shaped environments.
pub fn dump(scene: Option<&str>) -> io::Result<()> {
    const SCENES: [&str; 6] = ["welcome", "turn", "approval", "slash", "mention", "status"];
    let theme = crate::app::codex_theme();
    let sheet = StyleSheet::from_theme(&theme);
    let (w, h) = (96u16, 34u16);

    for name in SCENES {
        if scene.is_some_and(|s| s != name) {
            continue;
        }
        let mut app = App::new();
        match name {
            "turn" => {
                app.submit("why does the snapshot test fail?");
                for _ in 0..160 {
                    app.tick();
                }
            }
            "approval" => {
                app.submit("clean up the build artifacts");
                for _ in 0..120 {
                    app.tick();
                }
            }
            "slash" => {
                app.composer.set_text("/");
                app.sync_popup();
            }
            "mention" => {
                // The same popup machinery, opened by a different trigger.
                app.composer.set_text("explain @src/");
                app.sync_popup();
            }
            "status" => {
                app.submit("/status");
            }
            _ => {}
        }
        let probe = RectProbe::new();
        let root = build(&mut app, Rect::new(0, 0, w, h), &theme, &sheet, &probe);
        let buffer = tuika::testing::render(root.as_ref(), w, h, &theme);
        println!(
            "── {name} {}",
            "─".repeat((w as usize).saturating_sub(4 + name.len()))
        );
        println!("{}", tuika::testing::grid(&buffer));
        println!();
    }
    Ok(())
}
