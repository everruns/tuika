use std::sync::LazyLock;

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

static TREE_DEMO_ROWS: LazyLock<Vec<TreeRow<'static, u8>>> = LazyLock::new(|| {
    vec![
        TreeRow::root(1, "workspace", true),
        TreeRow::new(2, Some(1), 1, "src", true),
        TreeRow::new(3, Some(2), 2, "components", true),
        TreeRow::new(4, Some(3), 3, "tree_list.rs", false),
        TreeRow::new(5, Some(2), 2, "lib.rs", false),
        TreeRow::new(6, Some(1), 1, "tests", true),
        TreeRow::new(7, Some(6), 2, "integration.rs", false),
        TreeRow::root(8, "target", true),
    ]
});

static TREE_DEMO_STATES: LazyLock<Vec<TreeState<u8>>> = LazyLock::new(|| {
    let collapsed = TreeState::with_selected(1);
    let mut expanded = TreeState::with_selected(2);
    expanded.expand(1);
    let mut deep = TreeState::with_selected(4);
    deep.expand(1);
    deep.expand(2);
    deep.expand(3);
    deep.set_offset(1);
    vec![collapsed, expanded, deep]
});

pub(crate) fn scene_tree_list(frame: u64, _theme: &Theme) -> Element {
    let state = &TREE_DEMO_STATES[((frame / 24) as usize) % TREE_DEMO_STATES.len()];
    element(TreeList::new(&TREE_DEMO_ROWS, state).viewport(7))
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

/// A toy highlighter for the `TextArea` scene: keywords, strings, and macros.
/// A real host plugs in `tuika-codeformatters` through the same trait.
struct SceneRustHighlighter;

impl Highlighter for SceneRustHighlighter {
    fn highlight(
        &self,
        _lang: &str,
        lines: &[&str],
        theme: &Theme,
    ) -> Option<Vec<Vec<Span<'static>>>> {
        let code = theme.code;
        Some(
            lines
                .iter()
                .map(|line| {
                    let mut spans = Vec::new();
                    let mut in_string = false;
                    for word in line.split_inclusive(|c: char| {
                        !c.is_alphanumeric() && c != '_' && c != '!' && c != '"'
                    }) {
                        let (body, sep) = match word.char_indices().last() {
                            Some((i, c))
                                if !c.is_alphanumeric() && c != '_' && c != '!' && c != '"' =>
                            {
                                (&word[..i], &word[i..])
                            }
                            _ => (word, ""),
                        };
                        let mut style = Style::default().fg(code.text);
                        if body.starts_with('"') || in_string {
                            style = Style::default().fg(code.string);
                            in_string = !(body.ends_with('"') && body.len() > 1
                                || in_string && body.ends_with('"'));
                        } else if matches!(body, "fn" | "let" | "pub" | "mut") {
                            style = Style::default().fg(code.keyword);
                        } else if body.ends_with('!') {
                            style = Style::default().fg(code.function);
                        } else if body == "str" {
                            style = Style::default().fg(code.type_name);
                        }
                        spans.push(Span::styled(body.to_string(), style));
                        let sep_style = if in_string {
                            Style::default().fg(code.string)
                        } else {
                            Style::default().fg(code.punctuation)
                        };
                        spans.push(Span::styled(sep.to_string(), sep_style));
                    }
                    spans
                })
                .collect(),
        )
    }
}

/// The scripted session the `TextArea` scene replays: each step is an event and
/// how many frames to hold after it.
fn textarea_script() -> Vec<(Event, u64)> {
    let key = |code| Event::Key(Key::new(code));
    let with = |code, ctrl, shift| {
        Event::Key(Key {
            code,
            ctrl,
            alt: false,
            shift,
        })
    };
    let mut script = Vec::new();
    let type_line = |script: &mut Vec<(Event, u64)>, text: &str| {
        for ch in text.chars() {
            script.push((key(KeyCode::Char(ch)), 1));
        }
    };
    type_line(&mut script, "fn greet(name: &str) {");
    script.push((with(KeyCode::Enter, false, true), 2));
    type_line(&mut script, "    println!(\"hi {name}\");");
    script.push((with(KeyCode::Enter, false, true), 2));
    type_line(&mut script, "}");
    script.push((key(KeyCode::Up), 6));
    script.push((key(KeyCode::Home), 4));
    // Ctrl+Right hops `println` and `!("`; Shift+Ctrl+Right then selects
    // `hi {name}` a word at a time.
    script.push((with(KeyCode::Right, true, false), 5));
    script.push((with(KeyCode::Right, true, false), 5));
    for _ in 0..3 {
        script.push((with(KeyCode::Right, true, true), 5));
    }
    script.push((with(KeyCode::Right, false, true), 16));
    type_line(&mut script, "hello, {name}!");
    script.push((key(KeyCode::End), 14));
    // Undo steps back a word at a time, and the last step brings the selected
    // text back; redo replays it.
    for hold in [10, 24] {
        script.push((with(KeyCode::Char('z'), true, false), hold));
    }
    for hold in [8, 30] {
        script.push((with(KeyCode::Char('y'), true, false), hold));
    }
    script
}

pub(crate) fn scene_textarea(frame: u64, theme: &Theme) -> Element {
    let script = textarea_script();
    let total: u64 = script.iter().map(|(_, hold)| hold).sum::<u64>() + 12;
    let mut budget = frame % total;
    let mut state = TextAreaState::new();
    let mut last = "type";
    for (event, hold) in &script {
        if budget == 0 {
            break;
        }
        let _ = state.handle(event);
        if let Event::Key(k) = event {
            last = match (k.ctrl, k.shift, k.code) {
                (true, _, KeyCode::Char('z')) => "⌃Z undo",
                (true, _, KeyCode::Char('y')) => "⌃Y redo",
                (_, true, KeyCode::Left | KeyCode::Right) => "⇧ select",
                (_, _, KeyCode::Char(_)) if state.can_undo() => "type",
                _ => "move",
            };
        }
        budget = budget.saturating_sub(*hold);
    }
    let (row, col) = state.cursor();
    let selected = state
        .selected_text()
        .map(|text| format!(" · {} selected", text.chars().count()))
        .unwrap_or_default();
    let status = format!(" Ln {}, Col {}{selected} · {last}", row + 1, col + 1);
    let cursor_style = Style::default().add_modifier(Modifier::REVERSED);
    let editor = DrawView::new(
        move |area, surface: &mut Surface<'_>, ctx: &RenderCtx<'_>| {
            TextArea::new(&state)
                .line_numbers(true)
                .highlighter(&SceneRustHighlighter, "rust")
                .placeholder("Write some code…", Style::default().fg(ctx.theme.dim))
                .render(area, surface, ctx);
            // The recording has no terminal cursor, so draw one.
            if let Some((x, y)) = state.cursor_screen() {
                let symbol = surface.buffer_mut()[(x, y)].symbol().to_string();
                surface.set_string(x, y, &symbol, cursor_style);
            }
        },
    );
    view! {
        col {
            fixed(5) {
                boxed(title = Line::from(Span::styled(" greet.rs ", theme.accent_style()))) {
                    node(editor)
                }
            }
            fixed(1) { node(Text::new(vec![Line::from(Span::styled(status, Style::default().fg(theme.muted)))])) }
        }
    }
}
