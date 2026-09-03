//! A scroll viewport whose content is **views**, not lines.
//!
//! [`Scroll`](crate::components::Scroll) windows a `Vec<Line>`: one content row per line,
//! which makes its arithmetic trivial and its render O(viewport). That is the
//! right primitive for logs and prose, but it forces any host whose history
//! contains *laid-out* things — a bordered panel, a table, a diff, an image, a
//! nested flex row — to flatten them into styled lines by hand, or to draw box
//! glyphs into strings.
//!
//! [`ItemScroll`] is the same viewport over `Vec<Element>`. Each item is
//! measured at the render width, items stack with an optional gap, and the
//! viewport scrolls by **row**, not by item: an item taller than the remaining
//! space is clipped at the viewport edge and scrolls through it smoothly. That
//! makes it the primitive for a chat/agent transcript, a feed, or any list whose
//! entries are components rather than text.
//!
//! Ownership mirrors [`Scroll`]: [`new`](ItemScroll::new) takes the whole
//! content and measures it every frame (simplest, and right up to a few hundred
//! items); [`windowed`](ItemScroll::windowed) takes only the items that overlap
//! the viewport plus the true content height, so a host with its own height
//! cache stays O(viewport) however long the history grows.

use std::cell::RefCell;

use crate::buffer::Buffer;
use crate::geometry::Rect;

use crate::geometry::Size;
use crate::style::{StyleSheet, Theme};
use crate::surface::Surface;
use crate::view::{Element, RenderCtx, ScopedElement, View};

use super::{ScrollState, Scrollbar, VirtualWindow};

/// A vertical viewport over a list of [`Element`]s.
///
/// ![item_scroll demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/item_scroll.gif)
///
/// ```
/// use tuika::prelude::*;
/// use tuika::testing::{grid, render};
///
/// let items: Vec<Element> = vec![
///     element(Text::raw("first")),
///     element(Text::raw("second")),
/// ];
/// // Total rows the items occupy at this width — what the host clamps against.
/// let theme = Theme::default();
/// let ctx = RenderCtx::new(&theme);
/// assert_eq!(ItemScroll::measure_height(&items, 10, 0, false, &ctx), 2);
///
/// let state = ScrollState::new();
/// let view = ItemScroll::new(items, &state).scrollbar(false);
/// let buffer = render(&view, 6, 2, &Theme::default());
/// assert_eq!(grid(&buffer), "first \nsecond");
/// ```
pub struct ItemScroll<V: View = Element> {
    /// The items this view holds: all of them in [`new`](Self::new), or only the
    /// visible window in [`windowed`](Self::windowed).
    items: Vec<V>,
    /// Blank rows between two consecutive items.
    gap: u16,
    /// Content row of `items[0]`'s top edge. Zero for `new`.
    window_start_row: usize,
    /// Total content height in rows. `None` in owned mode, where it is measured
    /// at the render width; `Some` when windowed, where the host supplies it.
    content_height: Option<usize>,
    /// Top visible content row.
    offset: usize,
    scrollbar: bool,
    /// Item heights memoized for the width they were measured at, so `measure`
    /// and `render` in the same frame don't measure the tree twice.
    heights: RefCell<Option<(MeasureKey, Vec<u16>)>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MeasureKey {
    width: u16,
    theme: Theme,
    sheet: StyleSheet,
    style_resolver: Option<(usize, u64)>,
    focused: bool,
}

impl MeasureKey {
    fn new(width: u16, ctx: &RenderCtx) -> Self {
        Self {
            width,
            theme: *ctx.theme,
            sheet: ctx.sheet,
            style_resolver: ctx.style_resolver_key(),
            focused: ctx.focused,
        }
    }
}

impl<V: View> ItemScroll<V> {
    fn build(items: Vec<V>, state: &ScrollState) -> Self {
        Self {
            items,
            gap: 0,
            window_start_row: 0,
            content_height: None,
            offset: state.offset(),
            scrollbar: true,
            heights: RefCell::new(None),
        }
    }

    /// A viewport that already holds only the items overlapping the visible
    /// rows. `window_start_row` is the content row of `window[0]`'s top edge —
    /// which may sit *above* `state`'s offset when the first item is partially
    /// scrolled — and `content_height` is the row count of the whole list.
    ///
    /// This is the O(viewport) path: the host keeps its own per-item height
    /// cache (heights only change when an item or the width changes) and hands
    /// over the slice, instead of tuika measuring every item every frame.
    fn build_windowed(
        window: Vec<V>,
        window_start_row: usize,
        content_height: usize,
        state: &ScrollState,
    ) -> Self {
        Self {
            items: window,
            gap: 0,
            window_start_row,
            content_height: Some(content_height),
            offset: state.offset(),
            scrollbar: true,
            heights: RefCell::new(None),
        }
    }

    /// Insert `rows` blank rows between consecutive items (default 0).
    pub fn gap(mut self, rows: u16) -> Self {
        self.gap = rows;
        self
    }

    /// Show the overflow scrollbar (default true; only drawn when content
    /// overflows the viewport).
    pub fn scrollbar(mut self, show: bool) -> Self {
        self.scrollbar = show;
        self
    }

    /// Item heights at `width`, measuring (and memoizing) on first use.
    fn with_heights<R>(&self, width: u16, ctx: &RenderCtx, f: impl FnOnce(&[u16]) -> R) -> R {
        let mut cache = self.heights.borrow_mut();
        let key = MeasureKey::new(width, ctx);
        let fresh = !matches!(cache.as_ref(), Some((cached, _)) if *cached == key);
        if fresh {
            *cache = Some((key, measure_items(&self.items, width, ctx)));
        }
        let (_, heights) = cache.as_ref().expect("heights measured above");
        f(heights)
    }

    /// Content height in rows: the host's number when windowed, else measured.
    fn content_height(&self, width: u16, ctx: &RenderCtx) -> usize {
        match self.content_height {
            Some(h) => h,
            None => self.with_heights(width, ctx, |h| total_height(h, self.gap)),
        }
    }
}

impl ItemScroll<Element> {
    /// A viewport over all owned `items`, painting rows visible at `state`'s offset.
    pub fn new(items: Vec<Element>, state: &ScrollState) -> Self {
        Self::build(items, state)
    }

    /// A viewport over borrowed or owned items for the current frame.
    pub fn scoped<'view>(
        items: Vec<ScopedElement<'view>>,
        state: &ScrollState,
    ) -> ItemScroll<ScopedElement<'view>> {
        ItemScroll::build(items, state)
    }

    /// An owned viewport that already holds only the visible item window.
    pub fn windowed(
        window: Vec<Element>,
        window_start_row: usize,
        content_height: usize,
        state: &ScrollState,
    ) -> Self {
        Self::build_windowed(window, window_start_row, content_height, state)
    }

    /// A frame-borrowed viewport that already holds only the visible item window.
    pub fn scoped_windowed<'view>(
        window: Vec<ScopedElement<'view>>,
        window_start_row: usize,
        content_height: usize,
        state: &ScrollState,
    ) -> ItemScroll<ScopedElement<'view>> {
        ItemScroll::build_windowed(window, window_start_row, content_height, state)
    }

    /// Total rows `items` occupy at `width`, including `gap` rows.
    pub fn measure_height(
        items: &[Element],
        width: u16,
        gap: u16,
        scrollbar: bool,
        ctx: &RenderCtx,
    ) -> usize {
        Self::measure_views(items, width, gap, scrollbar, ctx)
    }

    /// Total rows any owned or borrowed view slice occupies at `width`.
    pub fn measure_views<V: View>(
        items: &[V],
        width: u16,
        gap: u16,
        scrollbar: bool,
        ctx: &RenderCtx,
    ) -> usize {
        let heights = measure_items(items, content_width(width, scrollbar), ctx);
        total_height(&heights, gap)
    }
}

/// Columns left for content once the scrollbar has its column.
///
/// The column is reserved whenever the bar is enabled, not only while the
/// content overflows: items re-wrap when the width changes, so a bar appearing
/// as the transcript grows would otherwise re-flow — and re-measure — everything
/// above it. The bar itself is still only *drawn* on overflow.
fn content_width(width: u16, scrollbar: bool) -> u16 {
    if scrollbar {
        width.saturating_sub(1)
    } else {
        width
    }
}

fn measure_items<V: View>(items: &[V], width: u16, ctx: &RenderCtx) -> Vec<u16> {
    items
        .iter()
        .map(|item| item.measure(Size::new(width, u16::MAX), ctx).height)
        .collect()
}

fn total_height(heights: &[u16], gap: u16) -> usize {
    let rows: usize = heights.iter().map(|h| *h as usize).sum();
    let gaps = heights.len().saturating_sub(1) * gap as usize;
    rows + gaps
}

impl<V: View> View for ItemScroll<V> {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        // `Size` is a cell extent (`u16`); a transcript can be taller than that,
        // so saturate exactly as `Scroll` does.
        let height = self
            .content_height(content_width(available.width, self.scrollbar), ctx)
            .min(u16::MAX as usize) as u16;
        Size::new(available.width, height)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let text_width = content_width(area.width, self.scrollbar);
        if text_width == 0 {
            return;
        }
        let content_h = self.content_height(text_width, ctx);
        let overflow = content_h > area.height as usize;

        let viewport_top = self.offset;
        let viewport_bottom = self.offset + area.height as usize;
        self.with_heights(text_width, ctx, |heights| {
            let mut top = self.window_start_row;
            for (item, height) in self.items.iter().zip(heights) {
                let height = *height;
                let bottom = top + height as usize;
                if bottom > viewport_top && top < viewport_bottom {
                    // Rows of this item hidden above the viewport's top edge.
                    let hidden = viewport_top.saturating_sub(top) as u16;
                    let y = area.y + (top.saturating_sub(viewport_top)) as u16;
                    let rect = Rect::new(area.x, y, text_width, height);
                    if hidden == 0 {
                        // Fully below the top edge: paint in place and let the
                        // surface clip whatever runs past the bottom.
                        item.render(rect, surface, ctx);
                    } else {
                        let visible = height.saturating_sub(hidden).min(area.height);
                        render_scrolled(
                            item, area.x, area.y, text_width, height, hidden, visible, surface, ctx,
                        );
                    }
                }
                top = bottom + self.gap as usize;
                if top >= viewport_bottom {
                    break;
                }
            }
        });

        if overflow && self.scrollbar {
            Scrollbar::vertical(VirtualWindow::new(
                content_h,
                usize::from(area.height),
                self.offset,
            ))
            .render(
                Rect::new(area.right() - 1, area.y, 1, area.height),
                surface,
                ctx,
            );
        }
    }
}

/// Paint an item whose first `hidden` rows are scrolled off the top.
///
/// A `Rect` cannot start above the viewport (coordinates are unsigned), so the
/// item is painted into a detached buffer and the visible slice is blitted down.
/// This runs for at most one item per frame — the one straddling the top edge —
/// and goes through [`render_ratatui`](Surface::render_ratatui), so the
/// destination stays clipped to the viewport and cells the item never touches
/// keep what was underneath.
///
/// The scratch buffer covers only the rows about to be blitted, positioned at
/// the item's own `hidden` offset rather than at zero: the item is handed its
/// full-height rect, and everything above or below the window is clipped away by
/// the buffer's own area. That bounds the allocation to the *viewport*, not to
/// the item — a child that measures itself as `u16::MAX` tall (any greedy flex
/// child does) would otherwise allocate a buffer of tens of megabytes to paint a
/// handful of visible rows.
#[allow(clippy::too_many_arguments)]
fn render_scrolled<V: View>(
    item: &V,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    hidden: u16,
    visible: u16,
    surface: &mut Surface,
    ctx: &RenderCtx,
) {
    if visible == 0 || width == 0 || height == 0 {
        return;
    }
    let destination = Rect::new(x, y, width, visible);
    surface.render_scratch(destination, |dst, buffer| {
        let window = Rect::new(0, hidden, width, dst.height);
        let mut scratch = Buffer::empty(window);
        // Seed the window with what is already on screen, so an item that paints
        // no background composites over it rather than punching a hole.
        for row in 0..dst.height {
            for col in 0..dst.width {
                if let Some(cell) = buffer.cell((dst.x + col, dst.y + row)) {
                    scratch[(col, hidden + row)] = cell.clone();
                }
            }
        }
        let full = Rect::new(0, 0, width, height);
        item.render(full, &mut Surface::new(&mut scratch, full), ctx);
        for row in 0..dst.height {
            for col in 0..dst.width {
                if let Some(cell) = scratch.cell((col, hidden + row)) {
                    buffer[(dst.x + col, dst.y + row)] = cell.clone();
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Boxed, Text};
    use crate::style::Theme;
    use crate::tests::support::{buffer, row};
    use crate::text::Line;
    use crate::view::element;

    /// `n` items, each a `rows`-tall block of numbered lines.
    fn blocks(n: usize, rows: usize) -> Vec<Element> {
        (0..n)
            .map(|i| {
                let lines: Vec<Line<'static>> = (0..rows)
                    .map(|r| Line::from(format!("item{i}-{r}")))
                    .collect();
                element(Text::new(lines))
            })
            .collect()
    }

    fn paint(view: &ItemScroll, w: u16, h: u16) -> Vec<String> {
        let theme = Theme::default();
        let mut buf = buffer(w, h);
        let area = Rect::new(0, 0, w, h);
        let ctx = RenderCtx::new(&theme);
        view.render(area, &mut Surface::new(&mut buf, area), &ctx);
        (0..h)
            .map(|y| row(&buf, y).trim_end().to_string())
            .collect()
    }

    #[test]
    fn measure_height_sums_items_and_gaps() {
        let items = blocks(3, 2);
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        assert_eq!(ItemScroll::measure_height(&items, 20, 0, false, &ctx), 6);
        assert_eq!(ItemScroll::measure_height(&items, 20, 1, false, &ctx), 8);
    }

    #[test]
    fn cached_heights_are_invalidated_when_the_measurement_context_changes() {
        struct StyledHeight;
        impl View for StyledHeight {
            fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
                let vertical = ctx
                    .sheet
                    .panel
                    .padding
                    .map_or(0, |padding| padding.vertical());
                Size::new(1, 1u16.saturating_add(vertical)).clamp_to(available)
            }

            fn render(&self, _area: Rect, _surface: &mut Surface, _ctx: &RenderCtx) {}
        }

        let state = ScrollState::new();
        let view = ItemScroll::new(vec![element(StyledHeight)], &state).scrollbar(false);
        let theme = Theme::default();
        let base = RenderCtx::new(&theme);
        assert_eq!(view.measure(Size::new(10, 10), &base).height, 1);

        let sheet = StyleSheet {
            panel: crate::style::StyleBundle::new().padding(crate::Padding::all(2)),
            ..StyleSheet::from_theme(&theme)
        };
        let styled = RenderCtx::new(&theme).with_sheet(sheet);
        assert_eq!(view.measure(Size::new(10, 10), &styled).height, 5);
    }

    #[test]
    fn resolver_revision_invalidates_cached_heights() {
        use std::cell::Cell;

        const ITEM: crate::StyleRole = crate::StyleRole::new("test.item");

        struct Resolver(Cell<u64>);
        impl crate::StyleResolver for Resolver {
            fn resolve(&self, role: crate::StyleRole) -> Option<crate::style::StyleBundle> {
                (role == ITEM).then(|| {
                    crate::style::StyleBundle::new()
                        .padding(crate::Padding::symmetric(0, self.0.get() as u16))
                })
            }

            fn revision(&self) -> u64 {
                self.0.get()
            }
        }

        struct StyledHeight;
        impl View for StyledHeight {
            fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
                let vertical = ctx
                    .style(ITEM)
                    .padding
                    .map_or(0, |padding| padding.vertical());
                Size::new(1, 1u16.saturating_add(vertical)).clamp_to(available)
            }

            fn render(&self, _area: Rect, _surface: &mut Surface, _ctx: &RenderCtx) {}
        }

        let resolver = Resolver(Cell::new(1));
        let state = ScrollState::new();
        let view = ItemScroll::new(vec![element(StyledHeight)], &state).scrollbar(false);
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme).with_style_resolver(&resolver);
        assert_eq!(view.measure(Size::new(10, 10), &ctx).height, 3);

        resolver.0.set(2);
        assert_eq!(view.measure(Size::new(10, 10), &ctx).height, 5);
    }

    #[test]
    fn renders_items_stacked_with_a_gap() {
        let state = ScrollState::new();
        let view = ItemScroll::new(blocks(2, 1), &state).gap(1);
        assert_eq!(paint(&view, 20, 3), ["item0-0", "", "item1-0"]);
    }

    #[test]
    fn scrolls_by_row_through_a_tall_item() {
        // Three 3-row items; offset 4 lands mid-way through the second one.
        let mut state = ScrollState::new();
        state.set_offset(4);
        let view = ItemScroll::new(blocks(3, 3), &state).scrollbar(false);
        assert_eq!(paint(&view, 20, 3), ["item1-1", "item1-2", "item2-0"]);
    }

    #[test]
    fn a_partially_scrolled_item_keeps_its_own_layout() {
        // A bordered panel scrolled one row up must show its body and bottom
        // edge — not a re-flowed or snapped-to-boundary version of itself.
        let mut state = ScrollState::new();
        state.set_offset(1);
        let items: Vec<Element> = vec![element(
            Boxed::new(element(Text::raw("body"))).title(Line::from("t")),
        )];
        let view = ItemScroll::new(items, &state).scrollbar(false);
        assert_eq!(paint(&view, 8, 2), ["│ body │", "╰──────╯"]);
    }

    #[test]
    fn windowed_matches_owned_for_the_same_offset() {
        let mut state = ScrollState::new();
        state.set_offset(3);
        let owned = ItemScroll::new(blocks(4, 2), &state).scrollbar(false);
        // The window starts at the item covering row 3: items[1] (rows 2..4).
        let windowed =
            ItemScroll::windowed(blocks(4, 2).into_iter().skip(1).collect(), 2, 8, &state)
                .scrollbar(false);
        assert_eq!(paint(&owned, 20, 3), paint(&windowed, 20, 3));
    }

    #[test]
    fn scrollbar_appears_only_on_overflow() {
        let state = ScrollState::new();
        let overflowing = ItemScroll::new(blocks(4, 2), &state);
        let rows = paint(&overflowing, 10, 3);
        assert!(rows.iter().any(|r| r.contains('█') || r.contains('│')));

        let fits = ItemScroll::new(blocks(1, 1), &state);
        assert_eq!(paint(&fits, 10, 3)[0], "item0-0");
    }

    /// A greedy child reports `u16::MAX` for the height it is offered, so a
    /// partially scrolled one must not allocate a scratch buffer of that many
    /// rows to paint the handful the viewport shows.
    #[test]
    fn a_greedy_item_scrolls_without_allocating_its_measured_height() {
        struct Greedy;
        impl View for Greedy {
            fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
                // What `Spacer` and any `grow` child do: take everything.
                available
            }
            fn render(&self, area: Rect, surface: &mut Surface, _ctx: &RenderCtx) {
                for y in area.y..area.bottom().min(area.y.saturating_add(4)) {
                    surface.set_string(area.x, y, "greedy", crate::style::Style::default());
                }
            }
        }

        let mut state = ScrollState::new();
        state.set_offset(2);
        let items: Vec<Element> = vec![element(Greedy)];
        let view = ItemScroll::new(items, &state).scrollbar(false);
        // Completes promptly and paints the right slice: rows 0..2 are hidden,
        // so the third and fourth "greedy" rows are what remain on screen.
        assert_eq!(paint(&view, 8, 2), ["greedy", "greedy"]);
    }

    #[test]
    fn degenerate_sizes_do_not_panic() {
        let state = ScrollState::new();
        for w in 0..4u16 {
            for h in 0..4u16 {
                let view = ItemScroll::new(blocks(3, 2), &state);
                let theme = Theme::default();
                let mut buf = buffer(w.max(1), h.max(1));
                let area = Rect::new(0, 0, w, h);
                let ctx = RenderCtx::new(&theme);
                view.render(area, &mut Surface::new(&mut buf, area), &ctx);
            }
        }
    }
}
