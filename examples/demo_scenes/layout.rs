use crate::*;

pub(crate) fn scene_flex(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let cell = |label: &str, color: Color| -> Element {
        let text = Text::new(vec![Line::from(Span::styled(
            label.to_string(),
            Style::default()
                .fg(theme.background)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        ))]);
        element(
            tuika::components::Boxed::new(element(text))
                .border(BorderStyle::Plain)
                .background(Style::default().bg(color)),
        )
    };
    view! {
        col(gap = 1) {
            fixed(3) {
                row(gap = 1) {
                    grow(1) { node(cell("grow 1", theme.accent)) }
                    grow(2) { node(cell("grow 2", theme.accent_alt)) }
                    fixed(12) { node(cell("fixed 12", theme.muted)) }
                }
            }
            fixed(1) { node(Text::new(vec![Line::from(Span::styled("row · gap 1 · grow shares leftover width", theme.muted_style()))])) }
        }
    }
}

pub(crate) fn scene_app_shell(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let query = "view";
    let header_text = theme.text_style();
    let header_accent = theme.accent_style();
    let header = view_fn(
        |available, _ctx| Size::new(available.width, available.height.min(2)),
        move |area, surface, _ctx| {
            surface.set_string(
                area.x.saturating_add(2),
                area.bottom().saturating_sub(1),
                "Search: ",
                header_text,
            );
            surface.set_string(
                area.x.saturating_add(10),
                area.bottom().saturating_sub(1),
                query,
                header_text,
            );
            surface.set_string(
                area.right().saturating_sub(10),
                area.bottom().saturating_sub(1),
                " 3 matches ",
                header_accent,
            );
        },
    );
    let content = Boxed::new(element(Text::new(vec![
        Line::from(Span::styled("app_shell.rs", theme.text_style())),
        Line::from(Span::styled("flex.rs", theme.text_style())),
        Line::from(Span::styled("responsive.rs", theme.text_style())),
    ])))
    .title(" Files ");
    let status_ready = theme.selection_style();
    let status_muted = theme.muted_style();
    let status = view_fn(
        |available, _ctx| Size::new(available.width, available.height.min(1)),
        move |area, surface, _ctx| {
            surface.set_string(area.x, area.y, " READY ", status_ready);
            surface.set_string(
                area.right().saturating_sub(15),
                area.y,
                "borrowed state ",
                status_muted,
            );
        },
    );

    element(
        AppShell::new(content)
            .header(header)
            .top_rule()
            .status(status)
            .bottom_rule()
            .footer(KeyHints::new([
                ("↑/↓", "move"),
                ("enter", "open"),
                ("q", "quit"),
            ])),
    )
}

pub(crate) fn scene_selection_screen(frame: u64, theme: &Theme) -> Element {
    let _ = (frame, theme);
    let rows = vec![
        Line::from("Run the requested command"),
        Line::from("Delegate to a specialist agent"),
        Line::from("Allow access for this command"),
        Line::from("Allow access for this session"),
        Line::from("Resume the interrupted task"),
        Line::from("Start a fresh task"),
    ];
    let mut state = SelectState::new();
    state.select(Some(3));

    element(
        SelectionScreen::new("Choose how to continue", rows, &state)
            .leading_rule()
            .trailing_rule()
            .footer(KeyHints::new([
                ("↑/↓", "move"),
                ("enter", "choose"),
                ("esc", "cancel"),
            ])),
    )
}

pub(crate) fn scene_flow(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let labels = [
        "build",
        "test",
        "docs",
        "release-ready",
        "terminal UI",
        "Rust",
    ];
    let mut flow = tuika::components::Flow::new().gap(1);
    for label in labels {
        flow = flow.item(element(
            tuika::components::Boxed::new(element(Text::raw(label)))
                .border_color(theme.accent)
                .padding(Padding::symmetric(1, 0)),
        ));
    }
    element(flow)
}

pub(crate) fn scene_grid(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let mut grid = tuika::components::Grid::new(3).gap(1);
    for (label, value) in [
        ("jobs", "12"),
        ("passed", "12"),
        ("failed", "0"),
        ("time", "4.2s"),
        ("target", "all"),
        ("status", "ready"),
    ] {
        grid = grid.cell(element(
            tuika::components::Boxed::new(element(Text::new(vec![
                Line::from(Span::styled(label, theme.muted_style())),
                Line::from(Span::styled(value, theme.accent_style())),
            ])))
            .padding(Padding::symmetric(1, 0)),
        ));
    }
    element(grid)
}

pub(crate) fn scene_scroll(frame: u64, theme: &Theme) -> Element {
    let lines: Vec<Line<'static>> = (1..=24)
        .map(|i| {
            Line::from(Span::styled(
                format!("  line {i:>2} — content that overflows the viewport"),
                theme.text_style(),
            ))
        })
        .collect();
    let viewport_h = 8usize;
    let content_h = lines.len();
    let mut state = ScrollState::new();
    let reach = tuika::anim::ping_pong(frame, 200);
    let steps = (reach * (content_h.saturating_sub(viewport_h) as f32 / 3.0 + 1.0)) as u32;
    let down = Event::Mouse(Mouse::at(MouseKind::ScrollDown, 0, 0));
    for _ in 0..steps {
        let _ = state.handle(&down, content_h, viewport_h);
    }
    view! {
        col {
            fixed(8) { node(Scroll::new(lines, &state)) }
        }
    }
}

/// A transcript of *components* — bordered panels beside plain rows — scrolling
/// by row, so a panel taller than the remaining space clips at the edge instead
/// of snapping. This is what `Scroll` cannot do: its content is pre-wrapped
/// lines, so anything laid out has to be flattened by hand first.
pub(crate) fn scene_item_scroll(frame: u64, theme: &Theme) -> Element {
    let items: Vec<Element> = (1..=6)
        .map(|i| {
            if i % 2 == 0 {
                element(
                    Boxed::new(element(Text::new(vec![
                        Line::from(Span::styled(format!("panel {i}"), theme.text_style())),
                        Line::from(Span::styled("laid out, not drawn", theme.muted_style())),
                    ])))
                    .title(Line::from(Span::styled(" note ", theme.accent_style()))),
                )
            } else {
                element(Text::new(vec![Line::from(Span::styled(
                    format!("  row {i} — an ordinary line of text"),
                    theme.text_style(),
                ))]))
            }
        })
        .collect();
    let viewport_h = 8usize;
    let content_h = ItemScroll::measure_height(&items, 60, 1, true, &RenderCtx::new(theme));
    let mut state = ScrollState::new();
    let reach = tuika::anim::ping_pong(frame, 200);
    let steps = (reach * (content_h.saturating_sub(viewport_h) as f32 / 3.0 + 1.0)) as u32;
    let down = Event::Mouse(Mouse::at(MouseKind::ScrollDown, 0, 0));
    for _ in 0..steps {
        let _ = state.handle(&down, content_h, viewport_h);
    }
    view! {
        col {
            fixed(8) { node(ItemScroll::new(items, &state).gap(1)) }
        }
    }
}

pub(crate) fn scene_scrollbar(_frame: u64, theme: &Theme) -> Element {
    let window = VirtualWindow::new(100, 24, 38);
    let labels = Text::new(vec![
        Line::from(Span::styled("item 38", theme.text_style())),
        Line::from(Span::styled("   ⋮", theme.muted_style())),
        Line::from(Span::styled("item 61", theme.text_style())),
        Line::from(Span::styled("24 visible of 100", theme.muted_style())),
    ]);
    view! {
        col(gap = 1) {
            fixed(4) {
                row(gap = 2) {
                    fixed(1) { node(Scrollbar::vertical(window)) }
                    grow(1) { node(labels) }
                }
            }
            fixed(1) { node(Scrollbar::horizontal(window)) }
        }
    }
}

pub(crate) fn scene_primitives(frame: u64, theme: &Theme) -> Element {
    let mut scroll = ScrollState::default();
    let pan = (tuika::anim::ping_pong(frame, 160) * 14.0).round() as usize;
    scroll.set_x_offset(pan);
    let mut form_state = FormState::default();
    form_state.focus(((frame / 28) % 2) as usize);

    let canvas = DrawView::new(
        |area: Rect, surface: &mut tuika::Surface<'_>, ctx: &tuika::RenderCtx<'_>| {
            for y in area.y..area.bottom() {
                surface.set_string(
                    area.x,
                    y,
                    "界  custom grid  0123456789  →",
                    ctx.theme.info_style(),
                );
            }
        },
    )
    .intrinsic_size(Size::new(31, 2));
    let preview = Viewport::new(element(canvas), Size::new(31, 2), &scroll)
        .vertical_scrollbar(false)
        .horizontal_scrollbar(true);
    let form = Form::new(
        vec![
            FormField::new("Name", element(Text::raw("Ada"))).help("host-owned value"),
            FormField::new("Preview", element(preview)).error("semantic validation row"),
        ],
        &form_state,
    )
    .stack_below(36);
    let dialog = Dialog::new("Reusable primitives", element(form))
        .size(56, 10)
        .key_hints([("enter", "submit"), ("esc", "cancel")])
        .dim_backdrop(true)
        .focus_owner("primitives");
    element(
        Scene::new(element(Text::new(vec![Line::from(Span::styled(
            "base screen · overlay placement resolves at render time",
            theme.muted_style(),
        ))])))
        .dialog(dialog),
    )
}

pub(crate) fn scene_dialog_presets(frame: u64, theme: &Theme) -> Element {
    let phase = (frame / 48) % 4;
    let dialog: Dialog = match phase {
        0 => ConfirmDialog::new(
            "Apply changes?",
            "This will update three files in the workspace.",
            &ConfirmDialogState::new(),
        )
        .confirm_label("Apply")
        .into(),
        1 => {
            let mut state = ChoiceDialogState::new();
            let down = Event::Key(Key::new(KeyCode::Down));
            let _ = state.handle(&down, 3);
            ChoiceDialog::new(
                "Choose model",
                "Select the model for the next turn.",
                vec!["Fast".into(), "Balanced".into(), "Deep".into()],
                &state,
            )
            .into()
        }
        2 => {
            let mut state = MultiChoiceDialogState::new();
            let space = Event::Key(Key::new(KeyCode::Char(' ')));
            let _ = state.handle(&space, 3);
            MultiChoiceDialog::new(
                "Include context",
                "Choose the sources sent with the request.",
                vec![
                    "Current file".into(),
                    "Diagnostics".into(),
                    "Git diff".into(),
                ],
                &state,
            )
            .into()
        }
        _ => InputDialog::new(
            "Rename task",
            "Give this activity a concise name.",
            &InputDialogState::from_text("Review parser changes"),
        )
        .placeholder("Task name")
        .into(),
    };
    element(
        Scene::new(element(Text::new(vec![Line::from(Span::styled(
            "Agent workspace · host content remains independent",
            theme.muted_style(),
        ))])))
        .dialog(dialog),
    )
}
