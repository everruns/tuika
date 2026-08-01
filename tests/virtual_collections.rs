use tuika::components::{Column, Scrollbar, SelectList, SelectState, Table, VirtualWindow};
use tuika::prelude::{Line, Style, Theme};
use tuika::testing::{grid, render};

#[test]
fn virtual_window_clamps_and_centers_without_overflowing() {
    let top = VirtualWindow::new(100, 10, usize::MAX);
    assert_eq!(top.range(), 90..100);
    assert_eq!(top.max_start(), 90);
    assert_eq!(VirtualWindow::max_start_for(100, 10), 90);
    assert!(top.overflows());

    let centered = VirtualWindow::around(100, 9, Some(50));
    assert_eq!(centered.range(), 46..55);
    assert!(centered.contains(50));

    let short = VirtualWindow::around(3, 10, Some(usize::MAX));
    assert_eq!(short.range(), 0..3);
    assert!(!short.overflows());
}

#[test]
fn one_scrollbar_component_handles_both_axes_and_large_collections() {
    let window = VirtualWindow::new(usize::MAX, 10, usize::MAX);
    let vertical = Scrollbar::vertical(window);
    let vertical_grid = grid(&render(&vertical, 1, 10, &Theme::default()));
    assert!(vertical_grid.lines().last().is_some_and(|line| line == "█"));

    let horizontal = Scrollbar::horizontal(VirtualWindow::new(100, 5, 50))
        .track_glyph('.')
        .thumb_glyph('#')
        .track_style(Style::default())
        .thumb_style(Style::default());
    assert_eq!(
        grid(&render(&horizontal, 10, 1, &Theme::default())),
        "....#....."
    );
}

#[test]
fn built_in_collections_accept_only_the_visible_data_window() {
    let mut state = SelectState::new();
    state.select(Some(502));
    let window = VirtualWindow::around(1_000, 5, state.selected());
    let items = window
        .range()
        .map(|index| Line::from(format!("item {index}")))
        .collect();
    let list = SelectList::windowed(items, window, &state);
    let list_grid = grid(&render(&list, 14, 5, &Theme::default()));
    assert!(list_grid.contains("item 502"));
    assert!(list_grid.lines().any(|line| line.ends_with(['█', '│'])));

    let rows = window
        .range()
        .map(|index| vec![Line::from(index.to_string())])
        .collect();
    let table = Table::windowed(vec![Column::flex("id", 1)], rows, window, &state);
    let table_grid = grid(&render(&table, 10, 6, &Theme::default()));
    assert!(table_grid.contains("502"));
    assert!(
        table_grid
            .lines()
            .skip(1)
            .any(|line| line.ends_with(['█', '│']))
    );
}
