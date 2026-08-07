//! Dynamic borrowed rows with stable keyed selection and viewport rendering.
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

struct Job {
    id: u64,
    name: String,
    status: &'static str,
    age: u16,
}

struct App {
    jobs: Vec<Job>,
    selection: KeyedMultiSelectState<u64>,
    filter_running: bool,
    next_id: u64,
}

impl App {
    fn new() -> Self {
        let jobs = (1..=30)
            .map(|id| Job {
                id,
                name: format!("worker-{id:02}"),
                status: if id % 3 == 0 { "queued" } else { "running" },
                age: id as u16 * 3,
            })
            .collect();
        let mut selection = KeyedMultiSelectState::new();
        selection.cursor_mut().select(Some(1));
        selection.cursor_mut().set_scroll_margin(2);
        Self {
            jobs,
            selection,
            filter_running: false,
            next_id: 31,
        }
    }

    fn visible(&self) -> Vec<&Job> {
        visible_jobs(&self.jobs, self.filter_running)
    }

    fn reorder(&mut self) {
        let distance = 7.min(self.jobs.len());
        self.jobs.rotate_left(distance);
    }

    fn insert(&mut self) {
        let id = self.next_id;
        self.next_id += 1;
        self.jobs.insert(
            0,
            Job {
                id,
                name: format!("stream-{id:02}"),
                status: "running",
                age: 0,
            },
        );
    }

    fn delete_selected(&mut self) {
        let selected = self.selection.cursor().selected().copied();
        if let Some(id) = selected {
            self.jobs.retain(|job| job.id != id);
            self.selection.retain_present(&self.jobs, |job| &job.id);
        }
    }
}

fn visible_jobs(jobs: &[Job], filter_running: bool) -> Vec<&Job> {
    jobs.iter()
        .filter(|job| !filter_running || job.status == "running")
        .collect()
}

fn visible_key<'row>(row: &'row &Job) -> &'row u64 {
    &row.id
}

fn name<'row>(row: &'row &Job) -> Line<'row> {
    Line::from(row.name.as_str())
}

fn status<'row>(row: &'row &Job) -> Line<'row> {
    let color = if row.status == "running" {
        Color::Green
    } else {
        Color::Yellow
    };
    Line::styled(row.status, Style::default().fg(color))
}

fn age<'row>(row: &'row &Job) -> Line<'row> {
    Line::from(format!("{}s", row.age))
}

fn main() -> io::Result<()> {
    let theme = Theme::default();
    let _session = TerminalSession::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut app = App::new();

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let visible = app.visible();
            let table_area = Rect::new(
                area.x,
                area.y.saturating_add(2),
                area.width,
                area.height.saturating_sub(2),
            );
            let title = if app.filter_running {
                "Keyed jobs · running only"
            } else {
                "Keyed jobs · all rows"
            };
            let ctx = RenderCtx::new(&theme);
            let mut surface = Surface::new(frame.buffer_mut(), area);
            Text::raw(title).render(Rect::new(area.x, area.y, area.width, 1), &mut surface, &ctx);
            Text::raw("r reorder · f filter · a insert · d delete · space check · q quit").render(
                Rect::new(area.x, area.y.saturating_add(1), area.width, 1),
                &mut surface,
                &ctx,
            );
            KeyedTable::multi(
                vec![
                    KeyedColumn::fixed("id", 4, |row: &&Job| Line::from(row.id.to_string()))
                        .right(),
                    KeyedColumn::flex("worker", 2, name),
                    KeyedColumn::fixed("status", 8, status).hide_below(34),
                    KeyedColumn::fixed("age", 6, age).right().optional(),
                ],
                &visible,
                visible_key,
                &app.selection,
            )
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
                KeyCode::Char('f') => app.filter_running = !app.filter_running,
                KeyCode::Char('a') => app.insert(),
                KeyCode::Char('d') => app.delete_selected(),
                _ => {
                    let visible = visible_jobs(&app.jobs, app.filter_running);
                    let selection = &mut app.selection;
                    let _ = selection.handle(
                        &input,
                        &visible,
                        terminal.size()?.height.saturating_sub(3) as usize,
                        visible_key,
                        SelectNavigation::common(),
                    );
                }
            }
        } else {
            let visible = visible_jobs(&app.jobs, app.filter_running);
            let size = terminal.size()?;
            let table = Rect::new(0, 3, size.width, size.height.saturating_sub(3));
            let window =
                app.selection
                    .cursor()
                    .window(&visible, usize::from(table.height), visible_key);
            let selection = &mut app.selection;
            let _ = match input {
                Event::Mouse(ref mouse) if mouse.kind == MouseKind::Down(MouseButton::Left) => {
                    selection.handle_mouse(&input, &visible, table, window, visible_key)
                }
                _ => selection.handle(
                    &input,
                    &visible,
                    usize::from(table.height),
                    visible_key,
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
    fn dynamic_updates_keep_key_identity() {
        let mut app = App::new();
        app.selection.cursor_mut().select(Some(9));
        app.reorder();
        assert_eq!(app.selection.cursor().selected(), Some(&9));
        app.filter_running = true;
        assert!(!app.visible().iter().any(|job| job.id == 9));
        assert_eq!(app.selection.cursor().selected(), Some(&9));
        app.filter_running = false;
        assert!(app.visible().iter().any(|job| job.id == 9));
        app.delete_selected();
        assert_eq!(app.selection.cursor().selected(), None);
    }
}
