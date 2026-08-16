//! Interactive local-model picker with a live capability radar.
//! Use Up/Down (or j/k) to change the selected model.

use std::io;

use tuika::prelude::*;
use tuika_charts::{Axis, Chart, Point, Series};

const AXES: [&str; 5] = ["INTELLIGENCE", "SPEED", "SPECULATION", "MEMORY", "ACCURACY"];

struct Profile {
    name: &'static str,
    size: &'static str,
    context: &'static str,
    installed: bool,
    description: &'static str,
    why: [&'static str; 2],
    values: [f64; 5],
}

const PROFILES: [Profile; 6] = [
    Profile {
        name: "Mistral Small 3.1",
        size: "24 GB",
        context: "128K",
        installed: false,
        description: "Fast multimodal generalist with generous context.",
        why: [
            "Balanced for everyday coding and document work.",
            "Strong context handling without a huge footprint.",
        ],
        values: [82.0, 76.0, 68.0, 71.0, 84.0],
    },
    Profile {
        name: "Qwen3 30B A3B",
        size: "20 GB",
        context: "128K",
        installed: false,
        description: "Mixture-of-experts model for deep tool-using tasks.",
        why: [
            "High reasoning quality with few active weights.",
            "Best when tool use matters more than raw speed.",
        ],
        values: [90.0, 69.0, 88.0, 66.0, 91.0],
    },
    Profile {
        name: "Gemma 3 12B",
        size: "8 GB",
        context: "128K",
        installed: true,
        description: "Compact vision-language model for local tasks.",
        why: [
            "Reliable long-context work on modest hardware.",
            "Strong when quality and response both matter.",
        ],
        values: [84.0, 85.0, 63.0, 78.0, 87.0],
    },
    Profile {
        name: "Llama 3.2 3B",
        size: "2 GB",
        context: "128K",
        installed: true,
        description: "Tiny generalist for low-latency local use.",
        why: [
            "Starts quickly on memory-limited systems.",
            "Ideal for rewriting, extraction, and classification.",
        ],
        values: [64.0, 97.0, 42.0, 94.0, 72.0],
    },
    Profile {
        name: "Phi-4 Mini",
        size: "3 GB",
        context: "16K",
        installed: true,
        description: "Small reasoning model with excellent speed.",
        why: [
            "Punches above its size on structured reasoning.",
            "Practical offline assistant for moderate context.",
        ],
        values: [78.0, 94.0, 58.0, 91.0, 81.0],
    },
    Profile {
        name: "DeepSeek R1 14B",
        size: "9 GB",
        context: "64K",
        installed: true,
        description: "Reasoning-first model for technical analysis.",
        why: [
            "Excels when problems need longer deliberation.",
            "Trades speed for planning and mathematical accuracy.",
        ],
        values: [93.0, 57.0, 92.0, 73.0, 89.0],
    },
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
        let profile = &PROFILES[self.selected];
        let labels = AXES
            .iter()
            .zip(profile.values)
            .map(|(axis, value)| format!("{axis} {value:.0}%"));
        let chart = Chart::new()
            .x_axis(Axis::new().categories(labels))
            .y_axis(Axis::hidden())
            .series(Series::radar(
                profile.name,
                profile
                    .values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| Point::new(index as f64, *value)),
            ))
            .legend(false);

        let mut model_lines = vec![
            Line::styled(" AVAILABLE TO DOWNLOAD", theme.muted_style()),
            Line::styled(
                "   MODEL                   SIZE     CTX",
                theme.muted_style(),
            ),
        ];
        for (index, item) in PROFILES.iter().enumerate() {
            if index == 2 {
                model_lines.push(Line::styled("", Style::default()));
                model_lines.push(Line::styled(" ON THIS COMPUTER", theme.muted_style()));
            }
            let style = if index == self.selected {
                Style::default()
                    .fg(theme.selection_fg)
                    .bg(theme.selection_bg)
            } else {
                Style::default().fg(theme.text)
            };
            model_lines.push(Line::styled(
                format!(
                    " {} {:<21} {:>5} {:>7} ",
                    if index == self.selected { "▸" } else { " " },
                    item.name,
                    item.size,
                    item.context
                ),
                style,
            ));
        }
        let models = Text::new(model_lines);

        let summary = Text::new(vec![
            Line::styled(profile.name, theme.accent_style()),
            Line::styled(
                format!(
                    "{}  •  {} context  •  {}",
                    profile.size,
                    profile.context,
                    if profile.installed {
                        "ready locally"
                    } else {
                        "download required"
                    }
                ),
                theme.muted_style(),
            ),
            Line::styled(profile.description, Style::default().fg(theme.text)),
        ]);
        let rationale = Text::new(vec![
            Line::styled("WHY THIS MODEL", theme.muted_style()),
            Line::styled(profile.why[0], Style::default().fg(theme.text)),
            Line::styled(profile.why[1], Style::default().fg(theme.text)),
            Line::styled(
                format!(
                    "STATUS  {}",
                    if profile.installed {
                        "Installed • Ready"
                    } else {
                        "Available to download"
                    }
                ),
                theme.accent_style(),
            ),
        ]);

        view! {
            col(gap = 1) {
                fixed(2) {
                    node(Text::new(vec![
                        Line::styled("◈  LOCAL MODEL ROUTER", theme.accent_style()),
                        Line::styled("   Pick a runtime profile for the work ahead.", theme.muted_style()),
                    ]))
                }
                fixed(24) {
                    boxed(title = " Choose a local model ") {
                        col {
                            grow(1) {
                                row(gap = 2) {
                                    fixed(43) {
                                        boxed(title = " Models ") {
                                            node(models)
                                        }
                                    }
                                    grow(1) {
                                        col(gap = 1) {
                                            fixed(3) { node(summary) }
                                            fixed(10) { node(chart) }
                                            grow(1) { node(rationale) }
                                        }
                                    }
                                }
                            }
                            fixed(1) { text("↑/↓ or j/k choose  •  q quit") }
                        }
                    }
                }
                grow(1) { spacer() }
            }
        }
    }
}

fn main() -> io::Result<()> {
    let mut app = RadarApp { selected: 0 };
    Runner::new(RunnerConfig::default()).run(&Theme::default(), &mut app)
}
