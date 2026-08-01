//! Property-based tests for the layout solver, overlay resolver, and the split
//! footer.
//!
//! Example-based tests pin down specific cases; these assert *invariants* that
//! must hold for every input, which is how the flexbox solver's arithmetic
//! edge cases get shaken out (the out-of-bounds-placement bug fixed alongside
//! the resize suite is exactly the kind proptest finds automatically). The
//! footer properties are the same idea one layer up: whatever the screen size,
//! footer height, and publishing history, the footer must end up on the last
//! rows with its own content intact.

use proptest::prelude::*;
use ratatui_core::backend::{Backend, TestBackend};
use ratatui_core::layout::{Position, Rect};
use ratatui_core::terminal::{Terminal, TerminalOptions};
use ratatui_core::text::Line;

use crate::components::Text;
use crate::geometry::{Axis, Padding, Size};
use crate::layout::{Align, Dimension, Direction, Item, Justify, LayoutStyle, solve};
use crate::overlay::{Anchor, Extent, OverlaySpec, TargetAlign, TargetPlacement, TargetSide};
use crate::screen::{ScreenMode, Scrollback, close_footer, pin_footer};
use crate::style::Theme;
use crate::tests::support::row;
use crate::view::element;

/// An inline terminal anchored at `cursor_row`, as a host gets on startup.
fn footer_terminal(width: u16, height: u16, footer: u16, cursor_row: u16) -> Terminal<TestBackend> {
    let mut backend = TestBackend::new(width, height);
    backend
        .set_cursor_position(Position::new(0, cursor_row.min(height.saturating_sub(1))))
        .expect("place cursor");
    Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: ScreenMode::split_footer(footer).viewport(),
        },
    )
    .expect("inline terminal")
}

/// Paint `FOOTER` on every row the footer owns.
fn draw_footer(terminal: &mut Terminal<TestBackend>) {
    let theme = Theme::default();
    terminal
        .draw(|frame| {
            let area = frame.area();
            let lines = vec![Line::from("FOOTER"); area.height as usize];
            let view = element(Text::new(lines));
            crate::paint(frame.buffer_mut(), area, &theme, view.as_ref(), &[]);
        })
        .expect("draw footer");
}

fn dimension_strategy() -> impl Strategy<Value = Dimension> {
    prop_oneof![
        Just(Dimension::Auto),
        (0u16..60).prop_map(Dimension::Fixed),
        (0u16..=100).prop_map(Dimension::Percent),
        (1u16..6).prop_map(Dimension::Flex),
    ]
}

fn item_strategy() -> impl Strategy<Value = Item> {
    (dimension_strategy(), 0u16..60, 0u16..30).prop_map(|(d, w, h)| Item::new(d, Size::new(w, h)))
}

fn align_strategy() -> impl Strategy<Value = Align> {
    prop_oneof![
        Just(Align::Start),
        Just(Align::Center),
        Just(Align::End),
        Just(Align::Stretch),
    ]
}

fn justify_strategy() -> impl Strategy<Value = Justify> {
    prop_oneof![
        Just(Justify::Start),
        Just(Justify::Center),
        Just(Justify::End),
        Just(Justify::SpaceBetween),
    ]
}

fn direction_strategy() -> impl Strategy<Value = Direction> {
    prop_oneof![Just(Direction::Row), Just(Direction::Column)]
}

proptest! {
    /// For ANY area, item set, gap, padding, alignment and justification: every
    /// child rect stays inside the area, the rect count matches, and children
    /// are placed in non-decreasing main-axis order (never overlapping backward).
    #[test]
    fn solved_children_stay_within_area(
        w in 0u16..300,
        h in 0u16..120,
        items in proptest::collection::vec(item_strategy(), 1..8),
        gap in 0u16..30,
        pad in 0u16..6,
        dir in direction_strategy(),
        align in align_strategy(),
        justify in justify_strategy(),
    ) {
        let area = Rect::new(0, 0, w, h);
        let style = LayoutStyle {
            direction: dir,
            padding: Padding::all(pad),
            gap,
            align_items: align,
            justify,
        };
        let rects = solve(area, &style, &items);
        prop_assert_eq!(rects.len(), items.len());

        let axis = dir.axis();
        let mut last_main = 0u16;
        for r in &rects {
            prop_assert!(
                r.x >= area.x && r.right() <= area.right(),
                "x out of bounds: {:?} in {:?}",
                r,
                area
            );
            prop_assert!(
                r.y >= area.y && r.bottom() <= area.bottom(),
                "y out of bounds: {:?} in {:?}",
                r,
                area
            );
            let main = match axis {
                Axis::Horizontal => r.x,
                Axis::Vertical => r.y,
            };
            prop_assert!(main >= last_main, "children placed out of order: {:?}", rects);
            last_main = main;
        }
    }

    /// N equal-weight flex children with no gap or padding fill the main axis
    /// exactly — no cell lost to rounding, none double-counted.
    #[test]
    fn flex_children_fill_main_axis_exactly(w in 0u16..500, n in 1usize..8) {
        let items: Vec<Item> = (0..n)
            .map(|_| Item::new(Dimension::Flex(1), Size::ZERO))
            .collect();
        let rects = solve(Rect::new(0, 0, w, 1), &LayoutStyle::row(), &items);
        let total: u16 = rects.iter().map(|r| r.width).fold(0, u16::saturating_add);
        prop_assert_eq!(total, w, "flex children must fill width exactly");
    }

    /// An overlay always lands inside its screen and never exceeds its max
    /// clamp — for any anchor, size request, and margin (including a margin
    /// larger than the screen).
    #[test]
    fn overlay_stays_within_screen(
        w in 0u16..300,
        h in 0u16..120,
        pct_w in 0u16..=100,
        pct_h in 0u16..=100,
        margin in 0u16..12,
        anchor_i in 0usize..9,
        max_w in 1u16..300,
        max_h in 1u16..120,
    ) {
        let anchors = [
            Anchor::Center,
            Anchor::Top,
            Anchor::Bottom,
            Anchor::Left,
            Anchor::Right,
            Anchor::TopLeft,
            Anchor::TopRight,
            Anchor::BottomLeft,
            Anchor::BottomRight,
        ];
        let screen = Rect::new(0, 0, w, h);
        let spec = OverlaySpec {
            anchor: anchors[anchor_i],
            width: Extent::Percent(pct_w),
            height: Extent::Percent(pct_h),
            min_width: 0,
            min_height: 0,
            max_width: max_w,
            max_height: max_h,
            margin,
        };
        let r = spec.resolve(screen);
        prop_assert!(
            r.x >= screen.x && r.right() <= screen.right(),
            "overlay x out of bounds: {:?} in {:?}",
            r,
            screen
        );
        prop_assert!(
            r.y >= screen.y && r.bottom() <= screen.bottom(),
            "overlay y out of bounds: {:?} in {:?}",
            r,
            screen
        );
        prop_assert!(r.width <= max_w, "width exceeds max");
        prop_assert!(r.height <= max_h, "height exceeds max");
    }

    /// A target-relative overlay remains inside the screen for every side,
    /// alignment, target rectangle, margin, and degenerate screen size.
    #[test]
    fn target_overlay_stays_within_screen(
        x in 0u16..200,
        y in 0u16..100,
        w in 0u16..300,
        h in 0u16..120,
        target_x in 0u16..500,
        target_y in 0u16..220,
        target_w in 0u16..100,
        target_h in 0u16..80,
        overlay_w in 0u16..350,
        overlay_h in 0u16..160,
        margin in 0u16..20,
        side_i in 0usize..4,
        align_i in 0usize..3,
        gap in 0u16..20,
        flip in any::<bool>(),
    ) {
        let sides = [TargetSide::Above, TargetSide::Below, TargetSide::Left, TargetSide::Right];
        let aligns = [TargetAlign::Start, TargetAlign::Center, TargetAlign::End];
        let screen = Rect::new(x, y, w.min(u16::MAX - x), h.min(u16::MAX - y));
        let target = Rect::new(
            target_x,
            target_y,
            target_w.min(u16::MAX - target_x),
            target_h.min(u16::MAX - target_y),
        );
        let spec = OverlaySpec {
            width: Extent::Cells(overlay_w),
            height: Extent::Cells(overlay_h),
            margin,
            ..OverlaySpec::centered(0, 0)
        };
        let placement = TargetPlacement::new(sides[side_i])
            .align(aligns[align_i])
            .gap(gap)
            .flip(flip);

        let r = spec.resolve_target(screen, target, placement);

        prop_assert!(r.x >= screen.x && r.right() <= screen.right(), "overlay x out of bounds: {:?} in {:?}", r, screen);
        prop_assert!(r.y >= screen.y && r.bottom() <= screen.bottom(), "overlay y out of bounds: {:?} in {:?}", r, screen);
    }
}

proptest! {
    /// For ANY screen, footer height, and starting cursor row: pinning lands the
    /// footer on the last rows of the screen, never taller than the screen and
    /// never past its edges — and pinning again changes nothing.
    #[test]
    fn a_pinned_footer_always_owns_the_last_rows(
        width in 1u16..80,
        height in 1u16..40,
        footer in 1u16..24,
        cursor_row in 0u16..40,
    ) {
        let mut terminal = footer_terminal(width, height, footer, cursor_row);
        pin_footer(&mut terminal).expect("pin");

        let area = terminal.get_frame().area();
        prop_assert_eq!(area.height, footer.min(height), "footer clamps to the screen");
        prop_assert_eq!(area.bottom(), height, "footer sits on the last row");
        prop_assert_eq!(area.x, 0);
        prop_assert_eq!(area.width, width);

        pin_footer(&mut terminal).expect("re-pin");
        prop_assert_eq!(terminal.get_frame().area(), area, "pinning is idempotent");
    }

    /// For ANY sequence of published blocks — including blocks taller than the
    /// screen — the footer keeps its own rows: publishing scrolls the terminal
    /// above it and a repaint restores it exactly.
    #[test]
    fn publishing_never_costs_the_footer_its_rows(
        width in 4u16..40,
        height in 2u16..24,
        footer in 1u16..8,
        blocks in proptest::collection::vec(1u16..30, 0..6),
    ) {
        let theme = Theme::default();
        let mut terminal = footer_terminal(width, height, footer, 0);
        pin_footer(&mut terminal).expect("pin");
        draw_footer(&mut terminal);

        let scrollback = Scrollback::new();
        for (i, &rows) in blocks.iter().enumerate() {
            scrollback.write(move |_width| {
                element(Text::new(
                    (0..rows).map(|r| Line::from(format!("b{i}.{r}"))).collect::<Vec<_>>(),
                ))
            });
        }
        scrollback.flush(&mut terminal, &theme).expect("flush");
        prop_assert!(scrollback.is_empty(), "every block was consumed");

        // The footer is repainted after publishing, exactly as a host loop does.
        pin_footer(&mut terminal).expect("re-pin");
        draw_footer(&mut terminal);

        let area = terminal.get_frame().area();
        prop_assert_eq!(area.bottom(), height, "the footer is still pinned");
        let buffer = terminal.backend().buffer();
        for y in area.top()..area.bottom() {
            let painted = row(buffer, y);
            prop_assert!(
                painted.starts_with(&"FOOTER"[..(width as usize).min(6)]),
                "footer row {} was overwritten by published output: {:?}",
                y,
                painted
            );
        }
    }

    /// For ANY screen and footer, closing hands back exactly the footer's rows:
    /// they end up blank, the cursor is parked at the top of them, and nothing
    /// above is touched.
    #[test]
    fn closing_returns_the_footer_rows_and_nothing_else(
        width in 6u16..40,
        height in 2u16..24,
        footer in 1u16..8,
    ) {
        let theme = Theme::default();
        let mut terminal = footer_terminal(width, height, footer, 0);
        pin_footer(&mut terminal).expect("pin");

        let scrollback = Scrollback::new();
        scrollback.write(|_width| element(Text::raw("kept")));
        scrollback.flush(&mut terminal, &theme).expect("flush");
        pin_footer(&mut terminal).expect("re-pin");
        draw_footer(&mut terminal);

        let area = terminal.get_frame().area();
        close_footer(&mut terminal).expect("close");

        prop_assert_eq!(
            terminal.backend().cursor_position(),
            Position::new(area.x, area.y),
            "the prompt resumes at the top of the released rows"
        );
        let buffer = terminal.backend().buffer();
        for y in area.top()..area.bottom() {
            prop_assert_eq!(row(buffer, y), "", "row {} was not released", y);
        }
        // The published block survives whenever it was still on screen: the
        // footer's own rows are the only ones `close_footer` may touch.
        if area.top() > 0 {
            let above: Vec<String> = (0..area.top()).map(|y| row(buffer, y)).collect();
            prop_assert!(
                above.iter().any(|line| line == "kept"),
                "published scrollback was cleared: {:?}",
                above
            );
        }
    }
}
