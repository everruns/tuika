---
title: Text components
description: Styled text, wrapped prose, and separators.
sidebar:
  label: Text
  order: 2
---

# Text components

[All components](../components.md)

### `Text`

A block of pre-styled [`Line`](https://docs.rs/ratatui)s drawn top-down and
clipped. `Paragraph` word-wraps plain prose from a base style and turns bare
`http(s)` URLs into styled OSC 8 hyperlinks by default; `Wrap` word-wraps
pre-styled lines while preserving per-span styles. Use
`Paragraph::link_policy(LinkPolicy::NONE)` when URLs must remain literal, or
`LinkPolicy::WEB.with_mailto()` to also link `mailto:` targets. `Text` and
`Wrap` never infer links from their content.
[Text API](https://docs.rs/tuika/latest/tuika/components/text/struct.Text.html) ·
[Paragraph API](https://docs.rs/tuika/latest/tuika/components/text/struct.Paragraph.html)

Horizontal alignment is honored. `Text` and `Wrap` read each `Line`'s
`alignment` (unset = flush-left), so centered titles, right-aligned totals, and
centered empty-state messages built by an existing formatting layer render as
intended; `Wrap` carries a line's alignment onto every reflowed row.
`Paragraph` takes one alignment for the whole block via `.alignment(..)`.

<img src="../demos/text.png" width="880" alt="Text demo">

```rust
use ratatui::layout::Alignment;
use ratatui::text::Line;
use tuika::term::hyperlink::LinkPolicy;
use tuika::prelude::*;
view! {
    col(gap = 1) {
        // Per-line alignment on pre-styled lines.
        node(Text::new(vec![
            Line::from("flush left"),
            Line::from("centered").centered(),
            Line::from("flush right").right_aligned(),
        ]))
        // Wrapped prose links bare web URLs without a special backend.
        node(Paragraph::new("Read https://docs.rs/tuika", style)
            .alignment(Alignment::Center))
        node(Paragraph::new("literal https://tuika.dev", style)
            .link_policy(LinkPolicy::NONE))
    }
}
```

### `Rule`

A one-row horizontal separator: optional leading title, then a fill glyph out to
the width. [API](https://docs.rs/tuika/latest/tuika/components/struct.Rule.html)

<img src="../demos/rule.png" width="880" alt="Rule demo">

```rust
use tuika::prelude::*;
view! {
    node(Rule::new().title(" Section "))
}
```

---

[All components](../components.md)
