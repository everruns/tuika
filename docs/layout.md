# Layout

Tuika uses integer-native flex layout for application structure. `Flex` is the
general container, `Flow` is the concise choice for intrinsic items that wrap,
and `Grid` is a deliberately small equal-column grid. All three measure ordinary
third-party `View` implementations through the same `MeasureRequest` contract.

## Flex containers and items

Container style controls direction, wrapping, padding, gaps, and alignment:

```rust
use tuika::prelude::*;

let tags = Flex::row()
    .wrap(FlexWrap::Wrap)
    .column_gap(1)
    .row_gap(1)
    .align_content(AlignContent::SpaceBetween)
    .auto(element(Text::raw("rust")))
    .auto(element(Text::raw("terminal-ui")));
# let _ = tags;
```

`align` positions items within one line. `align_content` distributes the lines
themselves when wrapping creates more than one. `justify` distributes space on
each line. Use `row_gap` and `column_gap` when the two axes need different
spacing; `gap` sets both.

Child sizing belongs to `FlexItemStyle`, separately from the container:

```rust
# use tuika::prelude::*;
let item = FlexItemStyle::default()
    .basis(Dimension::Fixed(12))
    .grow(1)
    .shrink(1)
    .min_main(6)
    .max_main(20)
    .align_self(Align::Center);
let row = Flex::row().styled(item, element(Text::raw("resizable")));
# let _ = row;
```

The basis is the starting main-axis size. Positive free space is shared by
`grow`; negative free space is removed according to `shrink`, without crossing
the item's min/max constraints. Integer cells are assigned by rounding track
boundaries, so the final child reaches the exact container boundary without
losing or double-counting a cell.

## Wrapping and line alignment

`FlexWrap::Wrap` starts a new line when the next item's outer main size no
longer fits. Each line resolves grow, shrink, justification, and item alignment
independently. `AlignContent::{Start, Center, End, Stretch, SpaceBetween}` then
places completed lines on the cross axis. `NoWrap` remains the default.

`Flow` packages the common intrinsic wrapping case:

```rust
# use tuika::prelude::*;
let flow = Flow::new()
    .gap(1)
    .item(element(Text::raw("one")))
    .item(element(Text::raw("two")))
    .item(element(Text::raw("a-longer-item")));
# let _ = flow;
```

Choose `Flex` when children need grow/shrink, fixed or percentage bases, or
per-item alignment. Choose `Flow` when every child is intrinsic and wrapping is
the main behavior.

## Flow versus Grid

`Flow` packs variable-width items and wraps wherever the next item stops
fitting. `Grid::new(columns)` creates a fixed number of equal-width columns,
fills them row-major, and derives each row's height from its tallest cell.

```rust
# use tuika::prelude::*;
let grid = Grid::new(3)
    .column_gap(1)
    .row_gap(1)
    .cell(element(Text::raw("a")))
    .cell(element(Text::raw("b")))
    .cell(element(Text::raw("c")));
# let _ = grid;
```

Grid intentionally omits CSS Grid's named lines, implicit tracks, spanning,
dense packing, and independent track definitions. Those features add a large
constraint language while terminal applications usually need a stable column
count and predictable clipping. Compose nested Flex/Grid containers when a
screen needs a more irregular structure.

## Measurement requests

`View::measure_request` receives optional known axes and an `AvailableSpace`
mode for unresolved axes:

- `Definite(n)` means at most `n` cells are available.
- `MinContent` asks for the smallest useful intrinsic contribution.
- `MaxContent` asks for the preferred unconstrained contribution.
- `known_width` and `known_height` mean the parent has already fixed that axis.

Flex and Grid preserve those modes when measuring third-party views and add a
known axis once a fixed basis or grid track has been resolved. Existing views
that only implement `measure` remain compatible through the default adapter.
Because `MeasureRequest` and `AvailableSpace` are non-exhaustive, downstream
implementations should match with a fallback arm so future request metadata can
be added compatibly.

## Migrating existing layouts

- `LayoutStyle::gap` became independent `row_gap` and `column_gap` fields. Use
  `Flex::gap(n)` when both should remain equal.
- `Item::dimension` moved into `Item::style.basis`. Prefer
  `Item::styled(FlexItemStyle, intrinsic)` for direct solver use.
- `Flex::{auto,grow,fixed}` remain concise compatibility builders. Use
  `Flex::styled` when shrink, min/max, or per-child alignment matters.
- Existing no-wrap layouts keep their behavior unless `FlexWrap::Wrap` is set.

The public [`solve_layout`](https://docs.rs/tuika/latest/tuika/fn.solve_layout.html)
result includes child rectangles and flex-line metadata for hosts that need hit
testing, scrolling, or layout inspection without painting.
