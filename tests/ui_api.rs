use tuika::ui::{Color, Line, Modifier, Rect, Span, Style};

#[test]
fn ui_module_exposes_custom_view_vocabulary() {
    let rect = Rect::new(1, 2, 3, 4);
    let style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let line = Line::from(vec![Span::styled("value", style)]);

    assert_eq!(rect.width, 3);
    assert_eq!(line.width(), 5);
}
