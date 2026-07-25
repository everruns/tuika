//! The demo application.
//!
//! Deliberately *ordinary* tuika: nothing here knows it is running in a
//! browser. It is the same shape as `examples/gallery.rs` or a host's own
//! renderer — build an [`Element`] tree from state each frame, hand it to
//! [`paint`], route [`Event`]s into the component states — which is the point
//! of the prototype. The browser-specific work lives entirely in `lib.rs` (the
//! wasm ABI) and `tuika.js` (canvas + input), on the far side of the same seam
//! a terminal host sits behind.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Modifier, Style};
use ratatui_core::text::{Line, Span};

use tuika::{
    Anchor, CodeHighlighter, Element, Event, Key, KeyCode, KeyHints, MarkdownState, Mouse, Overlay,
    OverlaySpec, Padding, ProgressBar, RectProbe, Scroll, ScrollState, SelectList, SelectOutcome,
    SelectState, Spinner, SpinnerStyle, StatusBar, StyleSheet, Tabs, TabsState, Text, Theme,
    ToastLevel, ToastList, Toasts, element, paint, view,
};

/// Prose for the markdown tab, revealed a few characters per frame so the
/// streaming renderer is doing real work rather than showing a static document.
const DOC: &str = "\
# tuika in the browser

This is the **streaming markdown** renderer, running as `wasm32-unknown-unknown`
and painting onto a `<canvas>`. No terminal is involved — and no tuika code
knows that.

- Layout, overlays, focus, and the keymap are pure computation.
- The cell buffer is [ratatui](https://ratatui.rs)'s, unchanged.
- Only the *host* differs: canvas instead of a PTY.

```rust
let mut buffer = Buffer::empty(area);
paint(&mut buffer, area, &theme, root.as_ref(), &overlays);
```

Resize the window and the prose re-wraps, because the host feeds the live
viewport width in cells — exactly as a terminal host does on `SIGWINCH`.
";

/// One log line per entry, for the third tab's scrollable viewport.
const LOG: [&str; 22] = [
    "INFO  wasm module instantiated",
    "INFO  canvas host attached (monospace, 2× device pixels)",
    "DEBUG measured cell 8.4 × 18 px",
    "INFO  grid resolved to 96 × 34 cells",
    "DEBUG theme: tuika default",
    "INFO  first frame painted in 0.9ms",
    "DEBUG buffer diff skipped — full repaint each frame",
    "INFO  keydown → tuika::Key { code: Down }",
    "DEBUG select state advanced to index 1",
    "INFO  keydown → tuika::Key { code: Tab }",
    "DEBUG tab state advanced to index 1",
    "WARN  no clipboard bridge in this prototype",
    "INFO  wheel → tuika::MouseKind::ScrollDown",
    "DEBUG scroll offset 0 → 3",
    "INFO  pointer moved, no hit region",
    "DEBUG toast pushed: Info",
    "INFO  resize observer fired: 1180 × 620 px",
    "DEBUG grid resolved to 140 × 34 cells",
    "INFO  markdown re-wrapped at width 96",
    "DEBUG 12 cached line groups invalidated",
    "INFO  steady state, 60fps",
    "DEBUG idle",
];

/// The three body tabs.
const TABS: [&str; 3] = ["Overview", "Markdown", "Log"];

/// Everything that survives a frame. Exactly the state a terminal host would
/// hold: tuika keeps none of it.
pub struct App {
    theme: Theme,
    frame: u64,
    tabs: TabsState,
    select: SelectState,
    scroll: ScrollState,
    markdown: MarkdownState,
    toasts: Toasts,
    help_open: bool,
    /// Where the log viewport actually landed last frame. Scroll clamping needs
    /// a row count, and only the solver knows it — so the demo reads it back
    /// with tuika's probe seam instead of hard-coding a guess that resizing
    /// would invalidate.
    log_rect: RectProbe,
    /// Same, for the pane the markdown wraps into: prose has to re-wrap to the
    /// real interior width, not the screen width minus a guessed border.
    prose_rect: RectProbe,
    /// Set whenever state changes, so the host can skip rebuilding a frame
    /// nothing would change. Animated scenes set it every tick anyway.
    dirty: bool,
}

impl Default for App {
    fn default() -> Self {
        let mut toasts = Toasts::new(3);
        toasts.push(ToastLevel::Info, "Press ? for the key map");
        Self {
            theme: Theme::default(),
            frame: 0,
            tabs: TabsState::default(),
            select: SelectState::new(),
            scroll: ScrollState::new(),
            markdown: MarkdownState::new(),
            toasts,
            help_open: false,
            log_rect: RectProbe::new(),
            prose_rect: RectProbe::new(),
            dirty: true,
        }
    }
}

impl App {
    /// The active theme, so the host can resolve `Color::Reset` and paint the
    /// canvas margin in the same background.
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Whether a redraw is warranted. Always true here while the spinner and
    /// progress bar are animating; a static screen would go quiet.
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// Advance animation and composite one frame into `buffer`.
    pub fn render(&mut self, buffer: &mut Buffer, area: Rect) {
        self.frame = self.frame.wrapping_add(1);
        self.toasts.tick();
        self.dirty = false;

        let root = self.body(area);
        // Both overlay views must outlive the `paint` borrow below. `Element` is
        // `Box<dyn View>` — implicitly `'static` — but an `Overlay` borrows its
        // view for the call only, which is what lets the toast stack (a view
        // *over* host-owned state) composite without leaking or cloning.
        let help = self.help_open.then(|| self.help_dialog());
        let toasts = ToastList::new(&self.toasts);
        let mut overlays: Vec<Overlay> = Vec::new();
        if !self.toasts.is_empty() {
            overlays.push(Overlay {
                area: OverlaySpec::centered(40, 20)
                    .anchor(Anchor::TopRight)
                    .max_size(44, 4)
                    .margin(2)
                    .resolve(area),
                view: &toasts,
                clear: false,
            });
        }
        if let Some(dialog) = &help {
            overlays.push(Overlay {
                area: OverlaySpec::centered(60, 50)
                    .min_size(38, 11)
                    .max_size(58, 11)
                    .resolve(area),
                view: dialog.as_ref(),
                clear: true,
            });
        }
        paint(buffer, area, &self.theme, root.as_ref(), &overlays);
    }

    /// Route one translated event. Returns `true` when a redraw is warranted —
    /// the same contract a terminal host's event loop uses.
    pub fn handle(&mut self, event: &Event) -> bool {
        let handled = match event {
            // While the dialog is open it owns input, so the same keys mean
            // something different. This is the whole of "modality" in tuika.
            Event::Key(key) if self.help_open => {
                matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?'))
                    .then(|| self.help_open = false)
                    .is_some()
            }
            Event::Key(key) => self.key(key),
            Event::Mouse(mouse) => self.mouse(mouse),
            Event::Resize { .. } => true,
            _ => false,
        };
        self.dirty |= handled;
        handled
    }

    fn key(&mut self, key: &Key) -> bool {
        match key.code {
            KeyCode::Char('?') => {
                self.help_open = true;
                true
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Left | KeyCode::BackTab => {
                self.tabs.handle(&Event::Key(*key), TABS.len()).consumed()
            }
            KeyCode::Char('t') => {
                self.toasts
                    .push(ToastLevel::Success, "Toast from the browser");
                true
            }
            _ if self.tab() == 2 => self
                .scroll
                .handle(&Event::Key(*key), LOG.len(), self.viewport_rows())
                .consumed(),
            // `SelectList` reports intent, not just consumption: a confirm is a
            // command, which is why the state machine hands it back rather than
            // swallowing it.
            _ => match self.select.handle(&Event::Key(*key), FILES.len()) {
                SelectOutcome::Moved(flow) => flow.consumed(),
                SelectOutcome::Confirmed(index) => {
                    self.toasts
                        .push(ToastLevel::Success, format!("Opened {}", FILES[index]));
                    true
                }
                SelectOutcome::Cancelled => false,
            },
        }
    }

    fn mouse(&mut self, mouse: &Mouse) -> bool {
        self.scroll
            .handle(
                &Event::Mouse(*mouse),
                LOG.len().max(FILES.len()),
                self.viewport_rows(),
            )
            .consumed()
    }

    /// Rows of log actually visible, from last frame's painted rect. Zero until
    /// the first paint, and one frame stale after a resize — the documented
    /// tradeoff of reading layout back rather than predicting it.
    fn viewport_rows(&self) -> usize {
        usize::from(self.log_rect.rect().height)
    }

    fn tab(&self) -> usize {
        self.tabs.selected()
    }

    fn body(&mut self, area: Rect) -> Element {
        let theme = self.theme;
        let labels: Vec<Line<'static>> = TABS
            .iter()
            .map(|s| Line::from(Span::styled((*s).to_string(), theme.text_style())))
            .collect();

        let content = match self.tab() {
            1 => {
                // First frame has no probed rect yet; fall back to the screen
                // width so the initial paint is close, then settle.
                let width = match self.prose_rect.rect().width {
                    0 => area.width.saturating_sub(6),
                    probed => probed,
                };
                self.markdown_pane(width.max(20))
            }
            2 => self.log_pane(),
            _ => self.overview_pane(),
        };

        let status = StatusBar::new()
            .left(vec![
                Span::styled(" WASM ", theme.selection_style()),
                Span::styled(
                    format!("  {} cols × {} rows", area.width, area.height),
                    theme.text_style(),
                ),
            ])
            .right(vec![Span::styled(
                format!("frame {} ", self.frame),
                theme.muted_style(),
            )])
            .background(Style::default().bg(theme.surface));

        view! {
            col(padding = Padding::all(1), gap = 1) {
                fixed(1) { node(Tabs::new(labels, &self.tabs)) }
                grow(1) { node(content) }
                fixed(1) { node(KeyHints::new([
                    ("tab", "switch"),
                    ("↑/↓", "move"),
                    ("t", "toast"),
                    ("?", "help"),
                ])) }
                fixed(1) { node(status) }
            }
        }
    }

    fn overview_pane(&self) -> Element {
        let theme = self.theme;
        let items: Vec<Line<'static>> = FILES
            .iter()
            .map(|s| Line::from(Span::styled((*s).to_string(), theme.text_style())))
            .collect();
        let selected = FILES[self.select.selected().min(FILES.len() - 1)];
        let progress = tuika::anim::ping_pong(self.frame, 180);

        view! {
            row(gap = 1) {
                fixed(28) {
                    boxed(title = Line::from(Span::styled(" files ", theme.accent_style()))) {
                        node(SelectList::new(items, &self.select))
                    }
                }
                grow(1) {
                    boxed(title = Line::from(Span::styled(" detail ", theme.accent_style())),
                          padding = Padding::all(1)) {
                        col(gap = 1) {
                            fixed(1) { node(Text::new(vec![Line::from(vec![
                                Span::styled("selected  ", theme.muted_style()),
                                Span::styled(selected.to_string(), theme.accent_style()),
                            ])])) }
                            fixed(1) { node(Text::new(vec![Line::from(vec![
                                Span::styled("host      ", theme.muted_style()),
                                Span::styled("canvas 2d, no terminal", theme.text_style()),
                            ])])) }
                            fixed(1) { node(Text::new(vec![Line::from(vec![
                                Span::styled("target    ", theme.muted_style()),
                                Span::styled("wasm32-unknown-unknown", theme.text_style()),
                            ])])) }
                            fixed(1) { spacer() }
                            fixed(1) { node(ProgressBar::determinate(progress).percent(true)) }
                            fixed(1) { node(ProgressBar::indeterminate(self.frame)) }
                            fixed(1) { spacer() }
                            fixed(1) { node(view! {
                                row(gap = 1) {
                                    fixed(2) { node(Spinner::new(self.frame).style(SpinnerStyle::Braille)) }
                                    text("same component, same frame counter")
                                }
                            }) }
                            grow(1) { spacer() }
                        }
                    }
                }
            }
        }
    }

    fn markdown_pane(&mut self, width: u16) -> Element {
        // Reveal a few characters per frame, then hold, then start over — the
        // streaming path a chat UI drives from a token stream.
        let total = DOC.chars().count();
        let pos = (self.frame as usize * 3) % (total + 300);
        let revealed: String = DOC.chars().take(pos.min(total)).collect();
        self.markdown.set(revealed);

        let sheet = StyleSheet::from_theme(&self.theme);
        let lines = self
            .markdown
            .lines(width, &self.theme, &sheet, CodeHighlighter::Plain)
            .to_vec();
        let theme = self.theme;
        view! {
            boxed(title = Line::from(Span::styled(" markdown ", theme.accent_style())),
                  padding = Padding::all(1)) {
                node(self.prose_rect.wrap(element(Text::new(lines))))
            }
        }
    }

    fn log_pane(&mut self) -> Element {
        let theme = self.theme;
        let lines: Vec<Line<'static>> = LOG
            .iter()
            .map(|entry| {
                let (level, rest) = entry.split_at(5);
                let style = match level.trim() {
                    "WARN" => Style::default().fg(theme.accent_alt),
                    "ERROR" => Style::default().fg(theme.accent),
                    "DEBUG" => theme.muted_style(),
                    _ => theme.text_style(),
                };
                Line::from(vec![
                    Span::styled(level.to_string(), style.add_modifier(Modifier::BOLD)),
                    Span::styled(rest.to_string(), theme.text_style()),
                ])
            })
            .collect();
        self.scroll.clamp(LOG.len(), self.viewport_rows());
        view! {
            boxed(title = Line::from(Span::styled(" log ", theme.accent_style()))) {
                node(self.log_rect.wrap(element(Scroll::new(lines, &self.scroll))))
            }
        }
    }

    fn help_dialog(&self) -> Element {
        let theme = self.theme;
        let row = |key: &str, what: &str| {
            Line::from(vec![
                Span::styled(format!("{key:<10}"), theme.accent_style()),
                Span::styled(what.to_string(), theme.text_style()),
            ])
        };
        view! {
            boxed(title = Line::from(Span::styled(" key map ", theme.accent_style())),
                  padding = Padding::all(1)) {
                col {
                    node(Text::new(vec![
                        row("tab", "next tab"),
                        row("shift+tab", "previous tab"),
                        row("↑ / ↓", "move selection · scroll the log"),
                        row("wheel", "scroll the log"),
                        row("t", "push a toast"),
                        row("?  esc", "close this dialog"),
                    ]))
                }
            }
        }
    }
}

/// The overview pane's list contents.
const FILES: [&str; 7] = [
    "src/lib.rs",
    "src/app.rs",
    "src/palette.rs",
    "tuika.js",
    "index.html",
    "build.sh",
    "README.md",
];
