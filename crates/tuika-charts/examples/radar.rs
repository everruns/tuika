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
    fit: &'static str,
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
        fit: "Balanced",
        installed: false,
        description: "Fast multimodal generalist with long context.",
        why: [
            "Balanced for coding and document work.",
            "Long context without a huge footprint.",
        ],
        values: [82.0, 76.0, 68.0, 71.0, 84.0],
    },
    Profile {
        name: "Qwen3 30B A3B",
        size: "20 GB",
        context: "128K",
        fit: "Smartest",
        installed: false,
        description: "MoE model for deep tool-using tasks.",
        why: [
            "Strong reasoning with few active weights.",
            "Best when tool use matters more than speed.",
        ],
        values: [90.0, 69.0, 88.0, 66.0, 91.0],
    },
    Profile {
        name: "Gemma 3 12B",
        size: "8 GB",
        context: "128K",
        fit: "Long ctx",
        installed: true,
        description: "Compact vision-language model for local work.",
        why: [
            "Long-context quality on modest hardware.",
            "Balances answer quality and response speed.",
        ],
        values: [84.0, 85.0, 63.0, 78.0, 87.0],
    },
    Profile {
        name: "Llama 3.2 3B",
        size: "2 GB",
        context: "128K",
        fit: "Fastest",
        installed: true,
        description: "Tiny generalist for low-latency local work.",
        why: [
            "Starts quickly on memory-limited systems.",
            "Great for rewriting and extraction.",
        ],
        values: [64.0, 97.0, 42.0, 94.0, 72.0],
    },
    Profile {
        name: "Phi-4 Mini",
        size: "3 GB",
        context: "16K",
        fit: "Light",
        installed: true,
        description: "Small reasoning model with excellent speed.",
        why: [
            "Strong structured reasoning for its size.",
            "Practical offline assistant for daily work.",
        ],
        values: [78.0, 94.0, 58.0, 91.0, 81.0],
    },
    Profile {
        name: "DeepSeek R1 14B",
        size: "9 GB",
        context: "64K",
        fit: "Reasoning",
        installed: true,
        description: "Reasoning-first model for technical analysis.",
        why: [
            "Excels at problems needing deliberation.",
            "Trades speed for planning and accuracy.",
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
        let theme = demo_theme();
        let profile = &PROFILES[self.selected];
        let labels = AXES
            .iter()
            .zip(profile.values)
            .map(|(axis, value)| format!("{axis}\n{value:.0}%"));
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
            Line::styled("AVAILABLE TO DOWNLOAD", theme.accent_style()),
            Line::styled(
                "   MODEL              SIZE   CTX   FIT",
                theme.muted_style(),
            ),
        ];
        for (index, item) in PROFILES.iter().enumerate() {
            if index == 2 {
                model_lines.push(Line::styled("", Style::default()));
                model_lines.push(Line::styled("ON THIS COMPUTER", theme.accent_style()));
            }
            let selected = index == self.selected;
            let name_style = if selected {
                theme.selection_style()
            } else {
                Style::default().fg(theme.text)
            };
            let meta_style = if selected {
                theme.selection_style()
            } else {
                theme.muted_style()
            };
            let fit_style = if selected {
                theme.selection_style()
            } else {
                Style::default().fg(theme.accent_alt)
            };
            model_lines.push(Line::from(vec![
                Span::styled(
                    format!(" {} ", if selected { "▸" } else { " " }),
                    name_style,
                ),
                Span::styled(format!("{:<17}", item.name), name_style),
                Span::styled(
                    format!(" {:>5} {:>5}  ", item.size, item.context),
                    meta_style,
                ),
                Span::styled(item.fit, fit_style),
            ]));
        }
        let models = Text::new(model_lines);

        let summary = Text::new(vec![
            Line::styled(profile.name, theme.accent_style()),
            Line::styled(
                format!(
                    "{} • {} • {} • {}",
                    profile.fit,
                    profile.size,
                    profile.context,
                    if profile.installed {
                        "Ready"
                    } else {
                        "Download"
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
                fixed(3) {
                    node(Text::new(vec![
                        Line::styled("◈  LOCAL MODEL ROUTER", theme.accent_style()),
                        Line::styled("   Private inference, matched to the work ahead.", theme.muted_style()),
                        Line::styled("   Current directory  ~/workspace", theme.muted_style()),
                    ]))
                }
                fixed(36) {
                    boxed(title = " Choose a local model ") {
                        col {
                            grow(1) {
                                row(gap = 4) {
                                    fixed(48) { node(models) }
                                    grow(1) {
                                        col(gap = 1) {
                                            fixed(4) { node(summary) }
                                            fixed(15) { node(chart) }
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

fn demo_theme() -> Theme {
    Theme {
        background: Color::Rgb(7, 17, 27),
        surface: Color::Rgb(10, 27, 40),
        text: Color::Rgb(190, 211, 221),
        muted: Color::Rgb(104, 137, 153),
        dim: Color::Rgb(31, 70, 88),
        accent: Color::Rgb(46, 202, 232),
        accent_alt: Color::Rgb(100, 157, 236),
        border: Color::Rgb(34, 74, 94),
        border_focused: Color::Rgb(46, 202, 232),
        selection_bg: Color::Rgb(18, 56, 76),
        selection_fg: Color::Rgb(224, 244, 250),
        ..Theme::default()
    }
}

fn main() -> io::Result<()> {
    let mut app = RadarApp { selected: 0 };
    Runner::new(RunnerConfig::default()).run(&demo_theme(), &mut app)
}
