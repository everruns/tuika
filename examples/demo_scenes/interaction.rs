use crate::*;

/// Bare `http(s)` URLs the host wraps in OSC 8, and a markdown link whose label
/// carries the target. Rendered with the theme's link color + underline — the
/// look a supporting terminal makes clickable; others show the text unchanged.
/// The normal paint path draws styled cells; real OSC 8 emission is the job of
/// `HyperlinkBackend` / `write_line`, so this scene shows the *appearance*.
pub(crate) fn scene_hyperlink(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let link = Style::default()
        .fg(theme.code.link)
        .add_modifier(Modifier::UNDERLINED);
    let body = Text::new(vec![
        Line::from(Span::styled(
            "A bare URL is wrapped in place — clickable, text unchanged:",
            theme.muted_style(),
        )),
        Line::from(vec![
            Span::styled("  see ", theme.text_style()),
            Span::styled("https://docs.rs/tuika", link),
            Span::styled(" for the API.", theme.text_style()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "A markdown link shows its label, hiding the target:",
            theme.muted_style(),
        )),
        Line::from(vec![
            Span::styled("  the ", theme.text_style()),
            Span::styled("tuika component gallery", link),
            Span::styled(" demos every widget.", theme.text_style()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Only http(s) links are emitted; anything else stays plain text.",
            theme.muted_style(),
        )),
    ]);
    element(body)
}

/// A left-drag selection growing over a phrase (real, copyable text), plus a
/// row of clickable regions a `HitMap` would resolve to actions.
pub(crate) fn scene_mouse(frame: u64, theme: &Theme) -> Element {
    let phrase = "the quick brown fox jumps over the lazy dog";
    let count = phrase.chars().count();
    let reach = tuika::anim::ping_pong(frame, 200);
    let selected = (reach * count as f32).round() as usize;
    let sel: String = phrase.chars().take(selected).collect();
    let rest: String = phrase.chars().skip(selected).collect();

    let button = |label: &str, active: bool| -> Span<'static> {
        if active {
            Span::styled(
                format!(" {label} "),
                Style::default()
                    .fg(theme.background)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(format!(" {label} "), theme.muted_style())
        }
    };
    let hot = (frame / 24) % 3;

    let body = Text::new(vec![
        Line::from(Span::styled(
            "Left-drag selects real text — copy it over SSH via OSC 52:",
            theme.muted_style(),
        )),
        Line::from(vec![
            Span::styled("  ", theme.text_style()),
            Span::styled(sel, theme.selection_style()),
            Span::styled(rest, theme.text_style()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "A HitMap maps screen regions to values — clicks become actions:",
            theme.muted_style(),
        )),
        Line::from(vec![
            Span::styled("  ", theme.text_style()),
            button("Run", hot == 0),
            Span::styled("  ", theme.text_style()),
            button("Diff", hot == 1),
            Span::styled("  ", theme.text_style()),
            button("Cancel", hot == 2),
        ]),
    ]);
    element(body)
}

pub(crate) fn scene_overlay(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    use tuika::overlay::Extent;
    use tuika::probe::RectProbe;

    let target = RectProbe::new();
    let base = |s: &str| {
        Text::new(vec![Line::from(Span::styled(
            s.to_string(),
            theme.muted_style(),
        ))])
    };
    let trigger = target.wrap(Text::new(vec![Line::from(Span::styled(
        "[ Open actions ▾ ]",
        theme.accent_style(),
    ))]));
    let root: Element = view! {
        col(gap = 0) {
            fixed(1) { node(base("base layer stays independently laid out")) }
            fixed(1) { node(base("the popover follows its trigger after layout")) }
            grow(1) { spacer() }
            fixed(1) {
                row {
                    grow(1) { spacer() }
                    fixed(20) { node(trigger) }
                    fixed(2) { spacer() }
                }
            }
            fixed(1) { node(base("preferred below · flipped above at the edge")) }
        }
    };
    let popover = view! {
        boxed(
            title = Line::from(Span::styled(" actions ", theme.accent_style())),
            border = BorderStyle::Rounded,
            padding = Padding::all(1)
        ) {
            col(gap = 1) {
                node(Text::new(vec![Line::from(Span::styled(
                    "Run command",
                    theme.text_style(),
                ))]))
                node(Text::new(vec![Line::from(Span::styled(
                    "Inspect logs",
                    theme.muted_style(),
                ))]))
            }
        }
    };
    let spec = OverlaySpec {
        width: Extent::Cells(28),
        height: Extent::Cells(7),
        ..OverlaySpec::centered(0, 0).margin(1)
    };
    element(
        Scene::new(root).overlay(SceneOverlay::new(popover, spec).target(
            &target,
            TargetPlacement::below().align(TargetAlign::End).gap(1),
        )),
    )
}
