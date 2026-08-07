//! AGF-shaped sessions projected from authoritative storage into a keyed table.
//!
//! Run with `cargo run --example keyed_table`. Use arrows, Page Up/Down,
//! Home/End, the mouse wheel, or click a row. Space/Enter checks a row; `r`
//! reorders, `f` filters, `a` inserts, `d` deletes, and `q`/Escape quits.

use std::io;
use std::time::Duration;

use crossterm::event;
use ratatui_core::terminal::Terminal;
use ratatui_crossterm::CrosstermBackend;
use tuika::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Agent {
    Claude,
    Codex,
}

impl Agent {
    fn label(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SessionKey {
    agent: Agent,
    session_id: String,
}

struct Session {
    agent: Agent,
    session_id: String,
    summary: String,
    branch: Option<String>,
    pinned: bool,
}

struct SessionRows<'a> {
    sessions: &'a [Session],
    visible: &'a [usize],
}

impl KeyedRowSource<SessionKey> for SessionRows<'_> {
    type Row = Session;

    fn len(&self) -> usize {
        self.visible.len()
    }

    fn row(&self, index: usize) -> Option<&Self::Row> {
        self.visible
            .get(index)
            .and_then(|&source_index| self.sessions.get(source_index))
    }

    fn key_eq(&self, _index: usize, row: &Self::Row, key: &SessionKey) -> bool {
        row.agent == key.agent && row.session_id == key.session_id
    }
}

impl NavigableKeyedRowSource<SessionKey> for SessionRows<'_> {
    fn key(&self, _index: usize, row: &Self::Row) -> SessionKey {
        SessionKey {
            agent: row.agent,
            session_id: row.session_id.clone(),
        }
    }
}

struct App {
    sessions: Vec<Session>,
    visible: Vec<usize>,
    fuzzy: Vec<Vec<usize>>,
    selection: KeyedMultiSelectState<SessionKey>,
    filter_codex: bool,
    next_id: u64,
}

impl App {
    fn new() -> Self {
        let sessions = (1..=30)
            .map(|id| Session {
                agent: if id % 2 == 0 {
                    Agent::Codex
                } else {
                    Agent::Claude
                },
                // Both agent namespaces deliberately contain the same ids.
                session_id: format!("session-{:02}", (id + 1) / 2),
                summary: format!("searchable session {id:02}"),
                branch: (id % 3 == 0).then(|| format!("feature/{id}")),
                pinned: id % 7 == 0,
            })
            .collect();
        let mut app = Self {
            sessions,
            visible: Vec::new(),
            fuzzy: Vec::new(),
            selection: KeyedMultiSelectState::new(),
            filter_codex: false,
            next_id: 31,
        };
        app.refresh_projection();
        let first = app.source().row(0).map(|row| SessionKey {
            agent: row.agent,
            session_id: row.session_id.clone(),
        });
        app.selection.cursor_mut().select(first);
        app.selection.cursor_mut().set_scroll_margin(2);
        app
    }

    fn source(&self) -> SessionRows<'_> {
        SessionRows {
            sessions: &self.sessions,
            visible: &self.visible,
        }
    }

    fn refresh_projection(&mut self) {
        self.visible = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| !self.filter_codex || session.agent == Agent::Codex)
            .map(|(index, _)| index)
            .collect();
        self.fuzzy = self
            .visible
            .iter()
            .map(|&index| {
                self.sessions[index]
                    .summary
                    .chars()
                    .enumerate()
                    .filter_map(|(position, ch)| (ch == 's').then_some(position))
                    .collect()
            })
            .collect();
    }

    fn reorder(&mut self) {
        let distance = 7.min(self.sessions.len());
        self.sessions.rotate_left(distance);
        self.refresh_projection();
    }

    fn insert(&mut self) {
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.insert(
            0,
            Session {
                agent: Agent::Codex,
                session_id: format!("stream-{id:02}"),
                summary: format!("streamed session {id:02}"),
                branch: None,
                pinned: false,
            },
        );
        self.refresh_projection();
    }

    fn delete_selected(&mut self) {
        let selected = self.selection.cursor().selected().cloned();
        if let Some(key) = selected {
            self.sessions.retain(|session| {
                session.agent != key.agent || session.session_id != key.session_id
            });
            self.refresh_projection();
            let source = SessionRows {
                sessions: &self.sessions,
                visible: &self.visible,
            };
            self.selection.retain_present_source(&source);
        }
    }
}

fn summary_line<'a>(summary: &'a str, positions: &[usize]) -> Line<'a> {
    let mut spans = Vec::new();
    let mut run_start = 0;
    let mut highlighted = false;
    for (character, (byte, _)) in summary.char_indices().enumerate() {
        let next_highlighted = positions.contains(&character);
        if next_highlighted != highlighted {
            if byte > run_start {
                let text = &summary[run_start..byte];
                spans.push(if highlighted {
                    Span::styled(text, Style::default().fg(Color::Yellow).bold())
                } else {
                    Span::raw(text)
                });
            }
            run_start = byte;
            highlighted = next_highlighted;
        }
    }
    if run_start < summary.len() {
        let text = &summary[run_start..];
        spans.push(if highlighted {
            Span::styled(text, Style::default().fg(Color::Yellow).bold())
        } else {
            Span::raw(text)
        });
    }
    Line::from(spans)
}

fn main() -> io::Result<()> {
    let theme = Theme::default();
    let _session = TerminalSession::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut app = App::new();

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let table_area = Rect::new(
                area.x,
                area.y.saturating_add(2),
                area.width,
                area.height.saturating_sub(2),
            );
            let title = if app.filter_codex {
                "Projected sessions · codex only"
            } else {
                "Projected sessions · all agents"
            };
            let ctx = RenderCtx::new(&theme);
            let mut surface = Surface::new(frame.buffer_mut(), area);
            Text::raw(title).render(Rect::new(area.x, area.y, area.width, 1), &mut surface, &ctx);
            Text::raw("r reorder · f filter · a insert · d delete · space check · q quit").render(
                Rect::new(area.x, area.y.saturating_add(1), area.width, 1),
                &mut surface,
                &ctx,
            );
            let source = app.source();
            KeyedTable::multi_source(
                vec![
                    KeyedColumn::fixed("pin", 3, |row: &Session| {
                        Line::from(if row.pinned { "pin" } else { "" })
                    }),
                    KeyedColumn::fixed("agent", 7, |row: &Session| Line::from(row.agent.label())),
                    KeyedColumn::flex_indexed("summary", 2, |index, row: &Session| {
                        summary_line(&row.summary, &app.fuzzy[index])
                    }),
                    KeyedColumn::fixed("branch", 12, |row: &Session| {
                        Line::from(row.branch.as_deref().unwrap_or(""))
                    })
                    .hide_below(52),
                    KeyedColumn::fixed("id", 10, |row: &Session| {
                        Line::from(row.session_id.as_str())
                    })
                    .optional(),
                ],
                &source,
                &app.selection,
            )
            .preserve_selection_fg(true)
            .render(table_area, &mut surface, &ctx);
        })?;

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let Some(input) = translate_event(event::read()?) else {
            continue;
        };
        if let Event::Key(key) = &input
            && key.plain()
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char('r') => app.reorder(),
                KeyCode::Char('f') => {
                    app.filter_codex = !app.filter_codex;
                    app.refresh_projection();
                }
                KeyCode::Char('a') => app.insert(),
                KeyCode::Char('d') => app.delete_selected(),
                _ => {
                    let source = SessionRows {
                        sessions: &app.sessions,
                        visible: &app.visible,
                    };
                    let selection = &mut app.selection;
                    let _ = selection.handle_source(
                        &input,
                        &source,
                        terminal.size()?.height.saturating_sub(3) as usize,
                        SelectNavigation::common(),
                    );
                }
            }
        } else {
            let source = SessionRows {
                sessions: &app.sessions,
                visible: &app.visible,
            };
            let size = terminal.size()?;
            let table = Rect::new(0, 3, size.width, size.height.saturating_sub(3));
            let window = app
                .selection
                .cursor()
                .window_source(&source, usize::from(table.height));
            let selection = &mut app.selection;
            let _ = match input {
                Event::Mouse(ref mouse) if mouse.kind == MouseKind::Down(MouseButton::Left) => {
                    selection.handle_mouse_source(&input, &source, table, window)
                }
                _ => selection.handle_source(
                    &input,
                    &source,
                    usize::from(table.height),
                    SelectNavigation::common(),
                ),
            };
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_preserves_composite_identity() {
        let mut app = App::new();
        app.selection.cursor_mut().select(Some(SessionKey {
            agent: Agent::Codex,
            session_id: "session-05".into(),
        }));
        app.reorder();
        assert_eq!(
            app.selection.cursor().selected().unwrap().agent,
            Agent::Codex
        );
        app.filter_codex = true;
        app.refresh_projection();
        assert!(app.source().row(0).is_some());
        app.delete_selected();
        assert!(app.selection.cursor().selected().is_none());
    }
}
