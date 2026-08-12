---
title: Component gallery
description: A visual catalog of every tuika component, grouped into focused pages with API links, examples, and recorded terminal demos.
sidebar:
  group:
    label: Components
  order: 2
---

# Component gallery

A visual catalog of tuika's components — each with a name, a one-line
description, and an animated demo.

## [Motion](components/motion.md)

Progress, activity, loading, and host-driven animation.

[`Spinner`](components/motion.md#spinner) · [`ProgressBar`](components/motion.md#progressbar) · [`ActivityList`](components/motion.md#activitylist) · [`Loader`](components/motion.md#loader) · [`Timeline`](components/motion.md#timeline)

## [Text](components/text.md)

Styled text, wrapped prose, and separators.

[`Text`](components/text.md#text) · [`Rule`](components/text.md#rule)

## [Markdown & code](components/markdown-code.md)

Streaming Markdown, HTML, highlighted code, and diffs.

[`Markdown` + `MarkdownState`](components/markdown-code.md#markdown--markdownstate) · [`Html`](components/markdown-code.md#html) · [`CodeBlock`](components/markdown-code.md#codeblock) · [`Diff`](components/markdown-code.md#diff)

## [Layout](components/layout.md)

Application shells, containers, focus scopes, and viewport structure.

[`AppShell`](components/layout.md#appshell) · [`SelectionScreen`](components/layout.md#selectionscreen) · [`Flex`](components/layout.md#flex) · [`Flow`](components/layout.md#flow) · [`Grid`](components/layout.md#grid) · [`Boxed`](components/layout.md#boxed) · [`FocusScope`](components/layout.md#focusscope) · [`StatusBar`](components/layout.md#statusbar) · [`Scrollbar` + `VirtualWindow`](components/layout.md#scrollbar--virtualwindow)

## [Interactive](components/interactive.md)

Scrolling, forms, dialogs, selection, tables, tabs, and input.

[`Scroll` + `ScrollState`](components/interactive.md#scroll--scrollstate) · [`ItemScroll`](components/interactive.md#itemscroll) · [`Viewport` + `ScrollState`](components/interactive.md#viewport--scrollstate) · [`Form` + `FormField` + `FormState`](components/interactive.md#form--formfield--formstate) · [`Scene` + `Dialog`](components/interactive.md#scene--dialog) · [Dialog presets](components/interactive.md#dialog-presets) · [`DrawView` / `CanvasView`](components/interactive.md#drawview--canvasview) · [`SelectList` + `SelectState`](components/interactive.md#selectlist--selectstate) · [`TreeList` + `TreeState`](components/interactive.md#treelist--treestate) · [`CompletionPalette` + `CompletionState`](components/interactive.md#completionpalette--completionstate) · [`Table` + `SelectState`](components/interactive.md#table--selectstate) · [`KeyedTable` + `KeyedSelectState`](components/interactive.md#keyedtable--keyedselectstate) · [`Tabs` + `TabsState`](components/interactive.md#tabs--tabsstate) · [`TabSelect` + `TabSelectState`](components/interactive.md#tabselect--tabselectstate) · [`Slider` + `SliderState`](components/interactive.md#slider--sliderstate) · [`TextInput` + `TextInputState`](components/interactive.md#textinput--textinputstate)

## [Notifications & console](components/notifications-console.md)

Transient notifications and structured console output.

[`Toasts` + `ToastList`](components/notifications-console.md#toasts--toastlist) · [`Console` + `ConsoleLog`](components/notifications-console.md#console--consolelog)

## [Banners, codes & pixels](components/banners-codes-pixels.md)

ASCII lettering, QR codes, framebuffers, and key hints.

[`AsciiFont`](components/banners-codes-pixels.md#asciifont) · [`QrCode`](components/banners-codes-pixels.md#qrcode) · [`FrameBuffer` + `FrameBufferView`](components/banners-codes-pixels.md#framebuffer--framebufferview) · [`KeyHints`](components/banners-codes-pixels.md#keyhints) · [`KeymapHelp`](components/banners-codes-pixels.md#keymaphelp)

## See also

- [API documentation](https://docs.rs/tuika) — the complete component reference,
  including helpers without a standalone demo (`Spacer`, `Responsive`,
  `Constrained`, `Wrap`).
- [Markdown guide](markdown.md) — streaming, GFM tables, highlighting, links,
  images, and inline HTML, in one place.
- [Runnable examples](../examples/) — enter the alternate screen; quit with `q`/`esc`.
- [README](../README.md) — the model behind the toolkit.
