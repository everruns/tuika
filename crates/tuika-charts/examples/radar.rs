//! Interactive radar chart with a profile list.
//! Use Up/Down (or j/k) to change the selected profile; the chart updates immediately.

use std::io;

use tuika::prelude::*;
use tuika_charts::{Axis, Chart, Point, Series};

const AXES: [&str; 6] = ["Strength", "Speed", "Stamina", "Skill", "Focus", "Luck"];
const PROFILES: [(&str, [f64; 6]); 5] = [
    ("Orion", [92.0, 55.0, 80.0, 68.0, 71.0, 38.0]),
    ("Lyra", [48.0, 91.0, 57.0, 86.0, 79.0, 65.0]),
    ("Atlas", [85.0, 40.0, 94.0, 61.0, 70.0, 51.0]),
    ("Nova", [64.0, 82.0, 63.0, 93.0, 88.0, 74.0]),
    ("Vega", [73.0, 69.0, 72.0, 75.0, 66.0, 96.0]),
];

struct RadarApp {
    selected: usize,
}

impl Application for RadarApp {
    fn update(&mut self, signal: Signal) -> UpdateResult {
        let Signal::Event(Event::Key(key)) = signal else {
            return UpdateResult::Clean;
        };
        if !key.plain() {
            return UpdateResult::Clean;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => UpdateResult::Exit,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.checked_sub(1).unwrap_or(PROFILES.len() - 1);
                UpdateResult::Dirty
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1) % PROFILES.len();
                UpdateResult::Dirty
            }
            _ => UpdateResult::Clean,
        }
    }

    fn view(&self, _frame: u64) -> ScopedElement<'_> {
        let theme = Theme::default();
        let (name, values) = PROFILES[self.selected];
        let chart = Chart::new()
            .title(format!("{name} attributes"))
            .x_axis(Axis::new().categories(AXES))
            .y_axis(Axis::new())
            .series(Series::radar(
                name,
                values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| Point::new(index as f64, *value)),
            ));

        let profiles = Text::new(
            PROFILES
                .iter()
                .enumerate()
                .map(|(index, (name, values))| {
                    let average = values.iter().sum::<f64>() / values.len() as f64;
                    Line::styled(
                        format!(
                            " {} {name:<7} {average:>3.0} ",
                            if index == self.selected { "▶" } else { " " }
                        ),
                        if index == self.selected {
                            Style::default()
                                .fg(theme.selection_fg)
                                .bg(theme.selection_bg)
                        } else {
                            Style::default().fg(theme.text)
                        },
                    )
                })
                .collect::<Vec<_>>(),
        );

        view! {
            col(gap = 1) {
                fixed(1) { text("CHARACTER ATTRIBUTES") }
                grow(1) {
                    row(gap = 2) {
                        fixed(20) {
                            boxed(title = " Profiles ") {
                                col(gap = 1) {
                                    fixed(1) { text("   NAME    SCORE") }
                                    grow(1) { node(profiles) }
                                }
                            }
                        }
                        grow(1) { node(chart) }
                    }
                }
                fixed(1) { text("↑/↓ or j/k select  •  q quit") }
            }
        }
    }
}

fn main() -> io::Result<()> {
    let mut app = RadarApp { selected: 0 };
    Runner::new(RunnerConfig::default()).run(&Theme::default(), &mut app)
}
