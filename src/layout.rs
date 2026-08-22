//! Flexbox-style terminal-cell layout.
//!
//! The solver is deliberately small and integer-native. It supports flex lines,
//! wrapping, grow and shrink distribution, per-item alignment, and exact
//! boundary-based rounding without importing a browser layout engine.

use std::ops::Range;

use crate::geometry::Rect;

use super::geometry::{Axis, Padding, Size};

/// How a child establishes its initial size on the main axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dimension {
    /// Use the child's measured main extent.
    #[default]
    Auto,
    /// Use exactly this many cells unless an explicit item style permits shrink.
    Fixed(u16),
    /// Use this percentage of the line's available main extent.
    Percent(u16),
    /// Start at zero and grow by this weight.
    Flex(u16),
}

/// Cross-axis alignment of an item within its flex line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    /// Pack against the cross-axis start.
    Start,
    /// Center on the cross axis.
    Center,
    /// Pack against the cross-axis end.
    End,
    /// Fill the line's cross extent.
    #[default]
    Stretch,
}

/// Main-axis distribution of space left after flex sizing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Justify {
    /// Pack at the main-axis start.
    #[default]
    Start,
    /// Center the line's items.
    Center,
    /// Pack at the main-axis end.
    End,
    /// Put all remaining space between items.
    SpaceBetween,
}

/// Distribution of flex lines on the cross axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlignContent {
    /// Pack lines at the cross-axis start.
    #[default]
    Start,
    /// Center the lines.
    Center,
    /// Pack lines at the cross-axis end.
    End,
    /// Grow line cross extents to fill the container.
    Stretch,
    /// Put all remaining space between lines.
    SpaceBetween,
}

/// Whether items may form multiple flex lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlexWrap {
    /// Keep every item on one line.
    #[default]
    NoWrap,
    /// Wrap overflowing items onto later lines.
    Wrap,
    /// Wrap and reverse line placement on the cross axis.
    WrapReverse,
}

/// Direction a flex container stacks its children.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    /// Stack left-to-right.
    Row,
    /// Stack top-to-bottom.
    #[default]
    Column,
}

impl Direction {
    /// The main axis for this direction.
    pub fn axis(self) -> Axis {
        match self {
            Self::Row => Axis::Horizontal,
            Self::Column => Axis::Vertical,
        }
    }
}

/// Container-owned layout properties.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutStyle {
    /// Which axis items flow along.
    pub direction: Direction,
    /// Inner inset before item placement.
    pub padding: Padding,
    /// Empty cells between rows.
    pub row_gap: u16,
    /// Empty cells between columns.
    pub column_gap: u16,
    /// Whether overflowing items form additional lines.
    pub wrap: FlexWrap,
    /// Default item alignment within each line.
    pub align_items: Align,
    /// Main-axis distribution within each line.
    pub justify: Justify,
    /// Cross-axis distribution of multiple lines.
    pub align_content: AlignContent,
}

impl LayoutStyle {
    /// A row-direction style; all other properties default.
    pub fn row() -> Self {
        Self {
            direction: Direction::Row,
            ..Self::default()
        }
    }

    /// A column-direction style; all other properties default.
    pub fn column() -> Self {
        Self::default()
    }

    /// Set both row and column gaps.
    pub fn gap(mut self, gap: u16) -> Self {
        self.row_gap = gap;
        self.column_gap = gap;
        self
    }

    /// Set the gap between rows.
    pub fn row_gap(mut self, gap: u16) -> Self {
        self.row_gap = gap;
        self
    }

    /// Set the gap between columns.
    pub fn column_gap(mut self, gap: u16) -> Self {
        self.column_gap = gap;
        self
    }

    /// Set the container padding.
    pub fn padding(mut self, padding: Padding) -> Self {
        self.padding = padding;
        self
    }

    /// Set the default cross-axis item alignment.
    pub fn align(mut self, align: Align) -> Self {
        self.align_items = align;
        self
    }

    /// Set main-axis distribution within each line.
    pub fn justify(mut self, justify: Justify) -> Self {
        self.justify = justify;
        self
    }

    /// Set line wrapping.
    pub fn wrap(mut self, wrap: FlexWrap) -> Self {
        self.wrap = wrap;
        self
    }

    /// Set cross-axis distribution of multiple lines.
    pub fn align_content(mut self, align: AlignContent) -> Self {
        self.align_content = align;
        self
    }

    /// Gap between items on the main axis.
    pub const fn main_gap(self) -> u16 {
        match self.direction {
            Direction::Row => self.column_gap,
            Direction::Column => self.row_gap,
        }
    }

    /// Gap between flex lines on the cross axis.
    pub const fn cross_gap(self) -> u16 {
        match self.direction {
            Direction::Row => self.row_gap,
            Direction::Column => self.column_gap,
        }
    }
}

/// Child-owned flex properties, separate from [`LayoutStyle`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlexItemStyle {
    /// Initial main-axis sizing rule.
    pub basis: Dimension,
    /// Weight used to distribute positive free space.
    pub grow: u16,
    /// Weight used to remove negative free space.
    pub shrink: u16,
    /// Minimum main extent.
    pub min_main: u16,
    /// Optional maximum main extent.
    pub max_main: Option<u16>,
    /// Per-item cross alignment override.
    pub align_self: Option<Align>,
}

impl Default for FlexItemStyle {
    fn default() -> Self {
        Self {
            basis: Dimension::Auto,
            grow: 0,
            shrink: 1,
            min_main: 0,
            max_main: None,
            align_self: None,
        }
    }
}

impl FlexItemStyle {
    /// Adapt the original compact [`Dimension`] API to an item style.
    pub fn from_dimension(dimension: Dimension) -> Self {
        match dimension {
            Dimension::Flex(weight) => Self {
                basis: Dimension::Fixed(0),
                grow: weight.max(1),
                ..Self::default()
            },
            Dimension::Fixed(cells) => Self {
                basis: Dimension::Fixed(cells),
                shrink: 0,
                ..Self::default()
            },
            basis => Self {
                basis,
                ..Self::default()
            },
        }
    }

    /// Set the flex basis.
    pub fn basis(mut self, basis: Dimension) -> Self {
        self.basis = basis;
        self
    }

    /// Set the positive free-space weight.
    pub fn grow(mut self, grow: u16) -> Self {
        self.grow = grow;
        self
    }

    /// Set the negative free-space weight.
    pub fn shrink(mut self, shrink: u16) -> Self {
        self.shrink = shrink;
        self
    }

    /// Set the minimum main extent.
    pub fn min_main(mut self, min: u16) -> Self {
        self.min_main = min;
        self
    }

    /// Set the maximum main extent.
    pub fn max_main(mut self, max: u16) -> Self {
        self.max_main = Some(max);
        self
    }

    /// Override the container's item alignment.
    pub fn align_self(mut self, align: Align) -> Self {
        self.align_self = Some(align);
        self
    }
}

/// A measured child and its child-owned flex properties.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    /// Flex properties belonging to this child.
    pub style: FlexItemStyle,
    /// Measured intrinsic content size.
    pub intrinsic: Size,
}

impl Item {
    /// Construct an item through the original compact dimension API.
    pub fn new(dimension: Dimension, intrinsic: Size) -> Self {
        Self::styled(FlexItemStyle::from_dimension(dimension), intrinsic)
    }

    /// Construct an item with independent basis, grow, shrink, and alignment.
    pub const fn styled(style: FlexItemStyle, intrinsic: Size) -> Self {
        Self { style, intrinsic }
    }
}

/// Geometry and item range for one resolved flex line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlexLine {
    /// Contiguous input item range assigned to this line.
    pub items: Range<usize>,
    /// Resolved line bounds inside the container.
    pub rect: Rect,
}

/// Full solver output, including item rectangles and flex-line geometry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutResult {
    /// One rectangle per input item, in input order.
    pub rects: Vec<Rect>,
    /// Resolved flex lines in logical order.
    pub lines: Vec<FlexLine>,
}

fn clamped_basis(item: &Item, axis: Axis, percent_basis: u16) -> u16 {
    let basis = match item.style.basis {
        Dimension::Auto => axis.main(item.intrinsic),
        Dimension::Fixed(cells) => cells,
        Dimension::Percent(percent) => {
            ((u32::from(percent_basis) * u32::from(percent.min(100))) / 100) as u16
        }
        Dimension::Flex(weight) => {
            // `styled` callers may still use Flex as a basis. Treat it like the
            // compact API: zero basis; the weight participates in growth.
            let _ = weight;
            0
        }
    };
    basis
        .max(item.style.min_main)
        .min(item.style.max_main.unwrap_or(u16::MAX))
}

/// Divide integer cells by weighted cumulative boundaries.
///
/// Computing adjacent boundaries instead of rounding each item independently
/// guarantees the shares sum exactly to `total` and keeps error below one cell.
fn weighted_shares(total: u16, weights: &[u32]) -> Vec<u16> {
    let weight_total: u64 = weights.iter().map(|weight| u64::from(*weight)).sum();
    if total == 0 || weight_total == 0 {
        return vec![0; weights.len()];
    }
    let mut cumulative = 0u64;
    let mut previous = 0u64;
    weights
        .iter()
        .map(|weight| {
            cumulative += u64::from(*weight);
            let boundary = u64::from(total) * cumulative / weight_total;
            let share = boundary - previous;
            previous = boundary;
            share as u16
        })
        .collect()
}

fn line_ranges(
    items: &[Item],
    axis: Axis,
    main_avail: u16,
    gap: u16,
    wrap: FlexWrap,
) -> Vec<Range<usize>> {
    if items.is_empty() {
        return Vec::new();
    }
    if wrap == FlexWrap::NoWrap {
        return std::iter::once(0..items.len()).collect();
    }

    let mut lines = Vec::new();
    let mut start = 0;
    let mut used = 0u16;
    for (index, item) in items.iter().enumerate() {
        let item_main = clamped_basis(item, axis, main_avail).min(main_avail);
        let needed = if index == start {
            item_main
        } else {
            gap.saturating_add(item_main)
        };
        if index > start && used.saturating_add(needed) > main_avail {
            lines.push(start..index);
            start = index;
            used = item_main;
        } else {
            used = used.saturating_add(needed);
        }
    }
    lines.push(start..items.len());
    lines
}

fn distribute_growth(sizes: &mut [u16], items: &[Item], free: u16) {
    let mut remaining = free;
    while remaining > 0 {
        let weights: Vec<u32> = sizes
            .iter()
            .zip(items)
            .map(|(size, item)| {
                let capacity = item
                    .style
                    .max_main
                    .unwrap_or(u16::MAX)
                    .saturating_sub(*size);
                if capacity > 0 {
                    u32::from(match (item.style.grow, item.style.basis) {
                        (0, Dimension::Flex(weight)) => weight.max(1),
                        (grow, _) => grow,
                    })
                } else {
                    0
                }
            })
            .collect();
        if weights.iter().all(|weight| *weight == 0) {
            break;
        }
        let shares = weighted_shares(remaining, &weights);
        let mut applied = 0u16;
        for ((size, item), share) in sizes.iter_mut().zip(items).zip(shares) {
            let capacity = item
                .style
                .max_main
                .unwrap_or(u16::MAX)
                .saturating_sub(*size);
            let delta = share.min(capacity);
            *size = size.saturating_add(delta);
            applied = applied.saturating_add(delta);
        }
        if applied == 0 {
            break;
        }
        remaining = remaining.saturating_sub(applied);
    }
}

fn distribute_shrink(sizes: &mut [u16], items: &[Item], overflow: u16) {
    let mut remaining = overflow;
    while remaining > 0 {
        let weights: Vec<u32> = sizes
            .iter()
            .zip(items)
            .map(|(size, item)| {
                if *size > item.style.min_main && item.style.shrink > 0 {
                    u32::from(item.style.shrink) * u32::from((*size).max(1))
                } else {
                    0
                }
            })
            .collect();
        if weights.iter().all(|weight| *weight == 0) {
            break;
        }
        let shares = weighted_shares(remaining, &weights);
        let mut applied = 0u16;
        for ((size, item), share) in sizes.iter_mut().zip(items).zip(shares) {
            let capacity = size.saturating_sub(item.style.min_main);
            let delta = share.min(capacity);
            *size = size.saturating_sub(delta);
            applied = applied.saturating_add(delta);
        }
        if applied == 0 {
            // A small remainder can round to zero for every active item.
            if let Some((size, _item)) = sizes
                .iter_mut()
                .zip(items)
                .find(|(size, item)| **size > item.style.min_main && item.style.shrink > 0)
            {
                *size -= 1;
                applied = 1;
            }
        }
        remaining = remaining.saturating_sub(applied);
    }
}

fn resolve_main_sizes(items: &[Item], axis: Axis, main_avail: u16, gap: u16) -> Vec<u16> {
    let total_gap = gap.saturating_mul(items.len().saturating_sub(1) as u16);
    let available = main_avail.saturating_sub(total_gap);
    let mut sizes: Vec<u16> = items
        .iter()
        .map(|item| clamped_basis(item, axis, available))
        .collect();
    let used = sizes.iter().copied().fold(0u16, u16::saturating_add);
    if used < available {
        distribute_growth(&mut sizes, items, available - used);
    } else if used > available {
        distribute_shrink(&mut sizes, items, used - available);
    }
    sizes
}

fn distributed_positions(
    sizes: &mut [u16],
    available: u16,
    base_gap: u16,
    mode: AlignContent,
) -> (u16, Vec<u16>) {
    let gap_count = sizes.len().saturating_sub(1);
    let base_gaps = base_gap.saturating_mul(gap_count as u16);
    let used = sizes.iter().copied().fold(base_gaps, u16::saturating_add);
    let free = available.saturating_sub(used);
    let mut gaps = vec![base_gap; gap_count];
    let offset = match mode {
        AlignContent::Start | AlignContent::Stretch | AlignContent::SpaceBetween => 0,
        AlignContent::Center => free / 2,
        AlignContent::End => free,
    };
    match mode {
        AlignContent::Stretch if !sizes.is_empty() => {
            let shares = weighted_shares(free, &vec![1; sizes.len()]);
            for (size, extra) in sizes.iter_mut().zip(shares) {
                *size = size.saturating_add(extra);
            }
        }
        AlignContent::SpaceBetween if gap_count > 0 => {
            for (gap, extra) in gaps
                .iter_mut()
                .zip(weighted_shares(free, &vec![1; gap_count]))
            {
                *gap = gap.saturating_add(extra);
            }
        }
        _ => {}
    }
    (offset, gaps)
}

/// Resolve item rectangles and flex-line geometry.
pub fn solve_layout(area: Rect, style: &LayoutStyle, items: &[Item]) -> LayoutResult {
    if items.is_empty() {
        return LayoutResult::default();
    }
    let axis = style.direction.axis();
    let inner = style.padding.inner(area);
    let inner_size = Size::from(inner);
    let main_avail = axis.main(inner_size);
    let cross_avail = axis.cross(inner_size);
    let main_gap = style.main_gap();
    let ranges = line_ranges(items, axis, main_avail, main_gap, style.wrap);

    let mut line_cross: Vec<u16> = ranges
        .iter()
        .map(|range| {
            items[range.clone()]
                .iter()
                .map(|item| axis.cross(item.intrinsic))
                .max()
                .unwrap_or(0)
                .min(cross_avail)
        })
        .collect();
    if ranges.len() == 1 {
        line_cross[0] = cross_avail;
    }
    let (mut cross_cursor, line_gaps) = distributed_positions(
        &mut line_cross,
        cross_avail,
        style.cross_gap(),
        style.align_content,
    );

    let mut rects = vec![Rect::default(); items.len()];
    let mut lines = Vec::with_capacity(ranges.len());
    for (line_index, range) in ranges.into_iter().enumerate() {
        let cross_len = line_cross[line_index];
        let logical_cross = cross_cursor.min(cross_avail);
        let cross_start = if style.wrap == FlexWrap::WrapReverse {
            cross_avail.saturating_sub(logical_cross.saturating_add(cross_len))
        } else {
            logical_cross
        };
        let line_items = &items[range.clone()];
        let main_sizes = resolve_main_sizes(line_items, axis, main_avail, main_gap);
        let used_main = main_sizes.iter().copied().fold(
            main_gap.saturating_mul(line_items.len().saturating_sub(1) as u16),
            u16::saturating_add,
        );
        let free = main_avail.saturating_sub(used_main);
        let mut item_gaps = vec![main_gap; line_items.len().saturating_sub(1)];
        let mut main_cursor = match style.justify {
            Justify::Start | Justify::SpaceBetween => 0,
            Justify::Center => free / 2,
            Justify::End => free,
        };
        if style.justify == Justify::SpaceBetween && !item_gaps.is_empty() {
            let gap_count = item_gaps.len();
            for (gap, extra) in item_gaps
                .iter_mut()
                .zip(weighted_shares(free, &vec![1; gap_count]))
            {
                *gap = gap.saturating_add(extra);
            }
        }

        for (line_item_index, item) in line_items.iter().enumerate() {
            let index = range.start + line_item_index;
            let align = item.style.align_self.unwrap_or(style.align_items);
            let intrinsic_cross = axis.cross(item.intrinsic).min(cross_len);
            let item_cross = if align == Align::Stretch {
                cross_len
            } else {
                intrinsic_cross
            };
            let cross_offset = match align {
                Align::Start | Align::Stretch => 0,
                Align::Center => cross_len.saturating_sub(item_cross) / 2,
                Align::End => cross_len.saturating_sub(item_cross),
            };
            let main_start = main_cursor.min(main_avail);
            let main_len = main_sizes[line_item_index].min(main_avail.saturating_sub(main_start));
            rects[index] = axis.place(
                inner,
                main_start,
                cross_start.saturating_add(cross_offset),
                main_len,
                item_cross.min(cross_avail.saturating_sub(cross_start)),
            );
            main_cursor = main_cursor.saturating_add(main_sizes[line_item_index]);
            if let Some(gap) = item_gaps.get(line_item_index) {
                main_cursor = main_cursor.saturating_add(*gap);
            }
        }

        lines.push(FlexLine {
            items: range,
            rect: axis.place(
                inner,
                0,
                cross_start,
                main_avail,
                cross_len.min(cross_avail.saturating_sub(cross_start)),
            ),
        });
        cross_cursor = cross_cursor.saturating_add(cross_len);
        if let Some(gap) = line_gaps.get(line_index) {
            cross_cursor = cross_cursor.saturating_add(*gap);
        }
    }
    LayoutResult { rects, lines }
}

/// Resolve one rectangle per item, preserving the original compact API.
pub fn solve(area: Rect, style: &LayoutStyle, items: &[Item]) -> Vec<Rect> {
    solve_layout(area, style, items).rects
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(dimension: Dimension, width: u16, height: u16) -> Item {
        Item::new(dimension, Size::new(width, height))
    }

    #[test]
    fn grow_uses_exact_boundary_rounding() {
        let items = [
            item(Dimension::Flex(1), 0, 1),
            item(Dimension::Flex(2), 0, 1),
        ];
        let rects = solve(Rect::new(0, 0, 10, 1), &LayoutStyle::row(), &items);
        assert_eq!((rects[0].width, rects[1].width), (3, 7));
        assert_eq!(rects[1].right(), 10);
    }

    #[test]
    fn percent_preserves_gap_aware_basis() {
        let items = [
            item(Dimension::Percent(50), 0, 1),
            item(Dimension::Auto, 4, 1),
        ];
        let rects = solve(Rect::new(0, 0, 20, 1), &LayoutStyle::row().gap(2), &items);
        assert_eq!(rects[0].width, 9);
        assert_eq!(rects[1].x, 11);
    }

    #[test]
    fn shrink_distributes_negative_space_and_respects_minimums() {
        let shrinkable = FlexItemStyle::default().basis(Dimension::Fixed(8));
        let protected = shrinkable.min_main(6);
        let items = [
            Item::styled(protected, Size::new(8, 1)),
            Item::styled(shrinkable, Size::new(8, 1)),
        ];
        let rects = solve(Rect::new(0, 0, 10, 1), &LayoutStyle::row(), &items);
        assert_eq!((rects[0].width, rects[1].width), (6, 4));
    }

    #[test]
    fn wrapping_forms_lines_and_aligns_them() {
        let items = [
            item(Dimension::Fixed(4), 4, 1),
            item(Dimension::Fixed(4), 4, 1),
            item(Dimension::Fixed(4), 4, 1),
        ];
        let result = solve_layout(
            Rect::new(0, 0, 9, 5),
            &LayoutStyle::row()
                .column_gap(1)
                .row_gap(1)
                .wrap(FlexWrap::Wrap)
                .align(Align::Start)
                .align_content(AlignContent::End),
            &items,
        );
        assert_eq!(result.lines.len(), 2);
        assert_eq!(result.lines[0].items, 0..2);
        assert_eq!(result.lines[1].items, 2..3);
        assert_eq!(result.rects[0].y, 2);
        assert_eq!(result.rects[2].y, 4);
    }

    #[test]
    fn cross_line_space_between_uses_the_full_cross_extent() {
        let items = [
            item(Dimension::Fixed(4), 4, 1),
            item(Dimension::Fixed(4), 4, 1),
            item(Dimension::Fixed(4), 4, 1),
        ];
        let result = solve_layout(
            Rect::new(0, 0, 4, 7),
            &LayoutStyle::row()
                .wrap(FlexWrap::Wrap)
                .align(Align::Start)
                .align_content(AlignContent::SpaceBetween),
            &items,
        );
        assert_eq!(
            result.rects.iter().map(|rect| rect.y).collect::<Vec<_>>(),
            [0, 3, 6]
        );
        assert_eq!(result.lines.last().unwrap().rect.bottom(), 7);
    }

    #[test]
    fn align_self_overrides_container_alignment() {
        let items = [Item::styled(
            FlexItemStyle::from_dimension(Dimension::Fixed(2)).align_self(Align::End),
            Size::new(2, 1),
        )];
        let rects = solve(Rect::new(0, 0, 5, 3), &LayoutStyle::row(), &items);
        assert_eq!(rects[0], Rect::new(0, 2, 2, 1));
    }

    #[test]
    fn padding_and_degenerate_areas_stay_bounded() {
        let items = [
            item(Dimension::Flex(1), 0, 0),
            item(Dimension::Fixed(5), 5, 1),
            item(Dimension::Percent(50), 0, 0),
        ];
        for (width, height) in [(0, 0), (1, 1), (2, 2), (3, 10), (60, 3)] {
            let area = Rect::new(0, 0, width, height);
            let style = LayoutStyle::row().gap(10).padding(Padding::all(1));
            for rect in solve(area, &style, &items) {
                assert!(rect.right() <= area.right());
                assert!(rect.bottom() <= area.bottom());
            }
        }
    }
}
