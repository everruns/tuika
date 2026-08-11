use crate::*;

pub(crate) fn scene_select(frame: u64, theme: &Theme) -> Element {
    let items: Vec<Line<'static>> = ["Open file…", "Save", "Save As…", "Toggle theme", "Quit"]
        .iter()
        .map(|s| Line::from(Span::styled((*s).to_string(), theme.text_style())))
        .collect();
    let mut state = SelectState::new();
    let target = (frame / 10) % items.len() as u64;
    let down = Event::Key(Key::new(KeyCode::Down));
    for _ in 0..target {
        let _ = state.handle(&down, items.len());
    }
    view! {
        col {
            grow(1) { node(SelectList::new(items, &state)) }
        }
    }
}

struct KeyedDemoRow {
    id: u64,
    name: &'static str,
    state: &'static str,
    age: u16,
}

static KEYED_ROWS: [KeyedDemoRow; 6] = [
    KeyedDemoRow {
        id: 101,
        name: "index workspace",
        state: "running",
        age: 4,
    },
    KeyedDemoRow {
        id: 102,
        name: "review patch",
        state: "queued",
        age: 12,
    },
    KeyedDemoRow {
        id: 103,
        name: "run checks",
        state: "running",
        age: 19,
    },
    KeyedDemoRow {
        id: 104,
        name: "refresh graph",
        state: "queued",
        age: 27,
    },
    KeyedDemoRow {
        id: 105,
        name: "stream logs",
        state: "running",
        age: 34,
    },
    KeyedDemoRow {
        id: 106,
        name: "publish report",
        state: "queued",
        age: 41,
    },
];

static KEYED_REORDERED: [usize; 6] = [4, 0, 3, 2, 5, 1];
static KEYED_ORIGINAL: [usize; 6] = [0, 1, 2, 3, 4, 5];
static KEYED_FILTERED: [usize; 2] = [0, 4];
static KEYED_SELECTION: KeyedSelectState<u64> = KeyedSelectState::with_selected(103);

struct KeyedDemoRows {
    visible: &'static [usize],
}

impl KeyedRowSource<u64> for KeyedDemoRows {
    type Row = KeyedDemoRow;

    fn len(&self) -> usize {
        self.visible.len()
    }

    fn row(&self, index: usize) -> Option<&Self::Row> {
        self.visible
            .get(index)
            .and_then(|&index| KEYED_ROWS.get(index))
    }

    fn key_eq(&self, _index: usize, row: &Self::Row, key: &u64) -> bool {
        row.id == *key
    }
}

fn keyed_demo_name(row: &KeyedDemoRow) -> Line<'_> {
    Line::from(row.name)
}

fn keyed_demo_state(row: &KeyedDemoRow) -> Line<'_> {
    Line::from(row.state)
}

fn keyed_demo_age(row: &KeyedDemoRow) -> Line<'_> {
    Line::from(format!("{}s", row.age))
}

pub(crate) fn scene_keyed_table(frame: u64, _theme: &Theme) -> Element {
    let phase = (frame / 18) % 3;
    let visible: &'static [usize] = match phase {
        0 => &KEYED_REORDERED,
        1 => &KEYED_FILTERED,
        _ => &KEYED_ORIGINAL,
    };
    let label = match phase {
        0 => "reordered · key 103 stays selected",
        1 => "filtered out · key 103 remains in host state",
        _ => "stream refreshed · key 103 restored",
    };
    view! {
        col {
            fixed(1) { node(Text::raw(label)) }
            grow(1) {
                node(KeyedTable::source(
                    vec![
                        KeyedColumn::fixed("id", 5, |row: &KeyedDemoRow| Line::from(row.id.to_string())).right(),
                        KeyedColumn::flex("task", 2, keyed_demo_name),
                        KeyedColumn::fixed("state", 8, keyed_demo_state).hide_below(34),
                        KeyedColumn::fixed("age", 5, keyed_demo_age).right().optional(),
                    ],
                    KeyedDemoRows { visible },
                    &KEYED_SELECTION,
                ))
            }
        }
    }
}

pub(crate) fn scene_completion_palette(frame: u64, theme: &Theme) -> Element {
    let _ = theme;
    let items = vec![
        CompletionItem::new("status")
            .detail("Show session status")
            .replacement("/status"),
        CompletionItem::new("model")
            .detail("Choose a model")
            .replacement("/model")
            .keyword("engine"),
        CompletionItem::new("approvals")
            .detail("Configure command approvals")
            .replacement("/approvals"),
        CompletionItem::new("review")
            .detail("Review current changes")
            .replacement("/review"),
        CompletionItem::new("init")
            .detail("Create project instructions")
            .replacement("/init"),
    ];
    let queries = ["", "m", "mo", "mod"];
    let query = queries[((frame / 24) as usize) % queries.len()];
    let mut state = CompletionState::new();
    state.sync(query, &items);
    element(
        CompletionPalette::new(&items, &state)
            .title("Commands")
            .viewport(5)
            .show_query(true),
    )
}

pub(crate) fn scene_tabs(frame: u64, theme: &Theme) -> Element {
    let labels: Vec<Line<'static>> = ["Chat", "Diff", "Logs", "Files"]
        .iter()
        .map(|s| Line::from(Span::styled((*s).to_string(), theme.text_style())))
        .collect();
    let mut state = TabsState::default();
    let target = (frame / 16) % labels.len() as u64;
    let right = Event::Key(Key::new(KeyCode::Right));
    for _ in 0..target {
        let _ = state.handle(&right, labels.len());
    }
    view! {
        col(gap = 1) {
            fixed(1) { node(Tabs::new(labels, &state)) }
            fixed(1) { node(Text::new(vec![Line::from(Span::styled("←/→ or Tab to switch", theme.muted_style()))])) }
        }
    }
}

fn demo_keymap() -> Keymap<&'static str> {
    Keymap::new()
        .layer(
            Layer::new("global")
                .bind_labeled("?", "open help", "help")
                .bind_labeled("q", "quit", "quit"),
        )
        .layer(
            Layer::new("search")
                .priority(10)
                .bind_labeled("enter", "open result", "open")
                .bind_labeled("ctrl+n", "next result", "next")
                .bind_labeled("ctrl+p", "previous result", "previous"),
        )
}

pub(crate) fn scene_key_hints(_frame: u64, theme: &Theme) -> Element {
    view! {
        col(gap = 1) {
            node(Text::new(vec![Line::from(Span::styled("Full width", theme.accent_style()))]))
            node(KeyHints::from_keymap(&demo_keymap()))
            row(gap = 2) {
                fixed(12) { node(Text::new(vec![Line::from(Span::styled("28 columns", theme.muted_style()))])) }
                fixed(28) { node(KeyHints::from_keymap(&demo_keymap())) }
            }
        }
    }
}

pub(crate) fn scene_keymap_help(_frame: u64, theme: &Theme) -> Element {
    view! {
        col(gap = 1) {
            node(Text::new(vec![Line::from(Span::styled("Active bindings", theme.accent_style()))]))
            node(KeymapHelp::from_keymap(&demo_keymap()))
        }
    }
}

pub(crate) fn scene_status_bar(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let bar = StatusBar::new()
        .left(vec![
            Span::styled(" NORMAL ", theme.selection_style()),
            Span::styled("  main.rs", theme.text_style()),
        ])
        .right(vec![
            Span::styled("utf-8  ", theme.muted_style()),
            Span::styled("Ln 42, Col 7 ", theme.text_style()),
        ])
        .background(Style::default().bg(theme.surface));
    view! {
        col {
            fixed(1) { node(bar) }
        }
    }
}

/// Typing into a composer: the placeholder while it is empty, and a `@` token
/// colored as it is typed. The trigger table is the host's — tuika finds the
/// token and paints the range it is handed.
pub(crate) fn scene_textinput(frame: u64, theme: &Theme) -> Element {
    let full = "fix @src/parser.rs: trailing commas";
    let n = ((frame / 3) as usize % (full.chars().count() + 12)).min(full.chars().count());
    let typed: String = full.chars().take(n).collect();
    let state = TextInputState::from_text(&typed);
    let mention = Style::default()
        .fg(theme.code.link)
        .add_modifier(Modifier::BOLD);
    let highlights: Vec<TextSpan> = state
        .tokens(&[Trigger::new('@')])
        .iter()
        .map(|t| t.span(mention))
        .collect();
    view! {
        col {
            fixed(3) {
                boxed(title = Line::from(Span::styled(" commit message ", theme.accent_style()))) {
                    node(
                        TextInput::new(&state)
                            .placeholder("describe the change", Style::default().fg(theme.dim))
                            .highlights(highlights)
                    )
                }
            }
        }
    }
}
