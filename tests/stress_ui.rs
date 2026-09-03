//! Whole-session UI stress sweep: every screen mode, every shell shape, driven
//! through the published runners while the terminal resizes underneath them.
//!
//! `tests/robustness.rs` sweeps one component at a time into a static rect. That
//! answers "does this component survive a hostile geometry". It cannot answer
//! "does this program survive being used", because everything that makes a
//! session hard happens *between* paints. This suite is that other half:
//!
//! - Every [`ScreenMode`], both mouse-capture variants included, driven through
//!   `Runner::run_driven_by` and `AsyncRunner::run_driven_by`.
//! - Real resizes, not just [`Event::Resize`] payloads. The screen applies a
//!   queued size inside `Backend::size`, so the renderer learns about the drag
//!   exactly where a real one does — which is the window in which `autoresize`,
//!   footer pinning, and scrollback commits can each be working from a geometry
//!   that no longer exists.
//! - Mode changes mid-session: a host that tears one renderer down and brings
//!   another up on the same screen must leave the terminal usable — the
//!   split-footer rows handed back, the published scrollback intact.
//! - Frame shapes that change under state that does not: the tree swaps for a
//!   list, the dialog opens, the row count moves, while selection, scroll,
//!   expansion, and a half-typed chord all persist across it.
//! - Shell chrome: [`AppShell`] region combinations under overlays, dialogs, and
//!   docks at every degenerate size.
//!
//! Everything is hermetic: an in-memory screen, scripted events, and a clock
//! that advances per reading. No tty, no raw mode, no sleeping — so the storms
//! are reproducible from their seeds and cannot go flaky on a loaded machine.
//! Raise [`TUIKA_STRESS_EVENTS`](storm_length) when hunting.

use std::cell::{Cell, RefCell};
use std::convert::Infallible;
use std::rc::Rc;
use std::time::Duration;

use tuika::buffer::Cell as BufferCell;
use tuika::geometry::{Position, Size as BackendSize};
use tuika::term::terminal::{Terminal, TerminalOptions};
use tuika::term::testbackend::TestBackend;
use tuika::term::traits::{Backend, ClearType, WindowSize};

use tuika::overlay::Anchor;
use tuika::prelude::*;
use tuika::runner::scripted_events;

// ---------------------------------------------------------------------------
// A screen two renderers can share, and that resizes under them.
// ---------------------------------------------------------------------------

/// The emulator's screen: one `TestBackend` behind a shared handle, plus the
/// size the "user" has dragged the window to but the renderer has not observed
/// yet.
///
/// Sharing matters for two reasons. A `Terminal` consumes its backend and never
/// gives it back, so a mid-session mode change — which needs a second
/// `Terminal` with a different viewport — can only reach the same screen through
/// a handle. And the event script needs to move the window *while the renderer
/// is between frames*, which is what a pending size is.
#[derive(Clone)]
struct Screen {
    backend: Rc<RefCell<TestBackend>>,
    pending: Rc<Cell<Option<(u16, u16)>>>,
}

impl Screen {
    fn new(width: u16, height: u16) -> Self {
        Self {
            backend: Rc::new(RefCell::new(TestBackend::new(width, height))),
            pending: Rc::new(Cell::new(None)),
        }
    }

    /// Queue a resize the renderer will see on its next size query.
    fn request_resize(&self, width: u16, height: u16) {
        self.pending.set(Some((width, height)));
    }

    /// A backend handle to hand to one `Terminal`.
    fn handle(&self) -> ScreenHandle {
        ScreenHandle(self.clone())
    }

    /// Read the visible grid, one string per row.
    fn rows(&self) -> Vec<String> {
        self.with(|backend| {
            let buffer = backend.buffer();
            (buffer.area.top()..buffer.area.bottom())
                .map(|y| {
                    (buffer.area.left()..buffer.area.right())
                        .map(|x| buffer[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect()
        })
    }

    fn with<T>(&self, f: impl FnOnce(&mut TestBackend) -> T) -> T {
        f(&mut self.backend.borrow_mut())
    }
}

/// One renderer's view of the shared screen.
struct ScreenHandle(Screen);

impl ScreenHandle {
    fn apply_pending(&self) {
        if let Some((width, height)) = self.0.pending.take() {
            self.0.with(|backend| {
                backend.resize(width, height);
                // A real emulator clamps the cursor into the new screen;
                // `TestBackend` leaves it where it was, and then indexes with it.
                let position = backend.get_cursor_position().expect("cursor");
                backend
                    .set_cursor_position(Position::new(
                        position.x.min(width.saturating_sub(1)),
                        position.y.min(height.saturating_sub(1)),
                    ))
                    .expect("cursor");
            });
        }
    }

    fn with<T>(&self, f: impl FnOnce(&mut TestBackend) -> T) -> T {
        self.0.with(f)
    }
}

impl Backend for ScreenHandle {
    type Error = Infallible;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a BufferCell)>,
    {
        self.with(|backend| backend.draw(content))
    }

    /// Same degenerate-screen accommodation as [`clear_region`](Self::clear_region):
    /// a real backend writes newlines and is done, while `TestBackend` divides
    /// the appended rows by a width that can be zero.
    fn append_lines(&mut self, n: u16) -> Result<(), Self::Error> {
        self.with(|backend| {
            if backend.buffer().area.is_empty() {
                return Ok(());
            }
            backend.append_lines(n)
        })
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.with(Backend::hide_cursor)
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.with(Backend::show_cursor)
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        self.with(Backend::get_cursor_position)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        let position = position.into();
        self.with(|backend| backend.set_cursor_position(position))
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.with(Backend::clear)
    }

    /// A screen with no cells has nothing to clear. Every real backend writes
    /// an escape sequence and is done; `TestBackend` indexes its buffer, so the
    /// degenerate sizes in the sweep would fail inside the harness instead of
    /// telling us anything about tuika.
    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.with(|backend| {
            if backend.buffer().area.is_empty() {
                return Ok(());
            }
            backend.clear_region(clear_type)
        })
    }

    /// The one method that is not a plain delegation: a size *query* is when a
    /// renderer learns the window moved, so the queued resize lands here.
    fn size(&self) -> Result<BackendSize, Self::Error> {
        self.apply_pending();
        self.0.with(|backend| Backend::size(&*backend))
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        self.apply_pending();
        self.with(Backend::window_size)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.with(Backend::flush)
    }

    #[cfg(feature = "scrolling-regions")]
    fn scroll_region_up(
        &mut self,
        region: std::ops::Range<u16>,
        lines: u16,
    ) -> Result<(), Self::Error> {
        self.with(|backend| backend.scroll_region_up(region, lines))
    }

    #[cfg(feature = "scrolling-regions")]
    fn scroll_region_down(
        &mut self,
        region: std::ops::Range<u16>,
        lines: u16,
    ) -> Result<(), Self::Error> {
        self.with(|backend| backend.scroll_region_down(region, lines))
    }
}

// ---------------------------------------------------------------------------
// Deterministic randomness and the event storm.
// ---------------------------------------------------------------------------

/// xorshift64*, so a failure replays exactly from its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn pick<'item, T>(&mut self, items: &'item [T]) -> &'item T {
        &items[self.below(items.len() as u64) as usize]
    }
}

/// Seeds replayed for every mode, so a failure is reproducible from its name.
const SEEDS: &[u64] = &[
    0x1234_5678_9abc_def0,
    0xdead_beef_cafe_f00d,
    0x5eed_5eed_5eed_5eed,
    0x9e37_79b9_7f4a_7c15,
];

/// Events per storm. The default keeps the suite inside a normal `cargo test`;
/// raise it when hunting, the way the property tests raise `PROPTEST_CASES`:
///
/// ```text
/// TUIKA_STRESS_EVENTS=5000 cargo test --test stress_ui
/// ```
fn storm_length() -> usize {
    std::env::var("TUIKA_STRESS_EVENTS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(250)
}

/// Sizes a terminal can plausibly be dragged to, degenerate ones included.
const SIZES: &[(u16, u16)] = &[
    (0, 0),
    (0, 8),
    (8, 0),
    (1, 1),
    (2, 3),
    (5, 2),
    (12, 4),
    (40, 12),
    (80, 24),
    (200, 1),
    (1, 60),
];

/// An event script that also drags the window, so a `Resize` the application
/// sees is a screen that really changed.
struct Storm {
    screen: Screen,
    rng: Rng,
    remaining: usize,
}

impl Storm {
    fn new(screen: Screen, seed: u64, events: usize) -> Self {
        Self {
            screen,
            rng: Rng(seed),
            remaining: events,
        }
    }
}

impl EventSource<Infallible> for Storm {
    fn poll_event(&mut self, _timeout: Duration) -> Result<Option<Event>, Infallible> {
        if self.remaining == 0 {
            // A source that reports nothing only means "idle" to the runner, so
            // the script has to say when the session is over.
            self.remaining = usize::MAX;
            return Ok(Some(Event::Key(Key::new(KeyCode::Char('q')))));
        }
        if self.remaining == usize::MAX {
            panic!("the runner kept polling after the quit key");
        }
        self.remaining -= 1;
        let keys = [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Tab,
            KeyCode::Backspace,
            KeyCode::Char('x'),
            KeyCode::Char('界'),
            KeyCode::Char('\u{7}'),
        ];
        // Pointer gestures are generated as *sequences* — press, drag, release
        // — because the runner's own drag selection is a state machine over
        // them, and a resize landing mid-gesture is the case that matters.
        let pointer = [
            MouseKind::Down(MouseButton::Left),
            MouseKind::Drag(MouseButton::Left),
            MouseKind::Drag(MouseButton::Left),
            MouseKind::Up(MouseButton::Left),
            MouseKind::Moved,
            MouseKind::ScrollUp,
            MouseKind::ScrollDown,
            MouseKind::ScrollLeft,
            MouseKind::ScrollRight,
            MouseKind::Down(MouseButton::Right),
        ];
        let event = match self.rng.below(7) {
            0 | 1 => {
                let &(width, height) = self.rng.pick(SIZES);
                self.screen.request_resize(width, height);
                Event::Resize { width, height }
            }
            2 | 3 => Event::Mouse(Mouse::at(
                *self.rng.pick(&pointer),
                self.rng.below(220) as u16,
                self.rng.below(70) as u16,
            )),
            4 => Event::Paste("pasted 日本語 👩‍👩‍👦\ttext".into()),
            5 => Event::Paste("\u{1b}[200~nested\u{7}".into()),
            _ => Event::Key(Key::new(*self.rng.pick(&keys))),
        };
        Ok(Some(event))
    }
}

/// A fixed event script that drags the window in step with the `Resize` events
/// it delivers, so the renderer learns the new size exactly when a real one
/// would: on the frame that follows the event, not before the run starts.
struct Script {
    screen: Screen,
    events: std::vec::IntoIter<Event>,
    idle: usize,
}

/// Idle polls inserted after each scripted event.
///
/// A repaint forced by a resize is deliberately coalesced: the loop schedules it
/// at least `RESIZE_FRAME_INTERVAL` after the last frame, so a script whose next
/// event is the quit key would exit before that frame is ever drawn. Idling
/// between events is what a real user does anyway, and it lets the deferred
/// frame land — the clock advances per reading, so no wall time passes.
const IDLE_ROUNDS: usize = 40;

impl Script {
    fn new(screen: &Screen, events: Vec<Event>) -> Self {
        Self {
            screen: screen.clone(),
            events: events.into_iter(),
            idle: 0,
        }
    }
}

impl EventSource<Infallible> for Script {
    fn poll_event(&mut self, _timeout: Duration) -> Result<Option<Event>, Infallible> {
        if self.idle > 0 {
            self.idle -= 1;
            return Ok(None);
        }
        let event = self.events.next();
        if let Some(Event::Resize { width, height }) = event {
            self.screen.request_resize(width, height);
        }
        self.idle = IDLE_ROUNDS;
        Ok(event)
    }
}

/// A clock that advances a fixed step per reading rather than with wall time,
/// so ticks, animation frames, and redraw deadlines all fire — deterministically,
/// and without a storm becoming flaky on a loaded machine.
#[derive(Clone)]
struct StepClock {
    origin: std::time::Instant,
    steps: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl StepClock {
    fn new() -> Self {
        Self {
            origin: std::time::Instant::now(),
            steps: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }
}

impl Clock for StepClock {
    fn now(&self) -> std::time::Instant {
        let step = self
            .steps
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.origin + Duration::from_millis(step)
    }
}

// ---------------------------------------------------------------------------
// The application under stress: chrome, overlays, and every stateful component
// that can disagree with a viewport.
// ---------------------------------------------------------------------------

const PROSE: &str = "# heading\n\nthe quick 日本語 👩‍👩‍👦 e\u{301} fox\n\n- one\n- two\n\n\
| a | b |\n| - | - |\n| 1 | 2 |\n\n```rust\nfn f() {}\n```\n\nsee https://example.dev";

struct StressApp {
    select: SelectState,
    scroll: ScrollState,
    input: TextInputState,
    tabs: TabsState,
    tree: TreeState<usize>,
    tree_rows: Vec<TreeRow<'static, usize>>,
    slider: SliderState,
    form: FormState,
    console: ConsoleLog,
    toasts: Toasts,
    dock: DockState,
    focus: FocusRegistry,
    keymap: Keymap<&'static str>,
    rows: usize,
    shape: u8,
    overlay: bool,
    frames: Cell<u64>,
}

impl StressApp {
    fn new() -> Self {
        let mut toasts = Toasts::new(4);
        toasts.push(ToastLevel::Error, "toast 日本語");
        let mut tree = TreeState::with_selected(0usize);
        tree.expand(0);
        Self {
            select: SelectState::new(),
            scroll: ScrollState::new(),
            input: TextInputState::from_text("composer 界"),
            tabs: TabsState::default(),
            tree,
            tree_rows: Vec::new(),
            slider: SliderState::new(0.0, 1.0, 0.5),
            form: FormState::default(),
            console: ConsoleLog::new(32),
            toasts,
            dock: DockState::new(),
            focus: FocusRegistry::new(),
            keymap: Keymap::new().layer(
                Layer::new("stress")
                    .bind("ctrl+p", "palette")
                    .bind("g g", "top")
                    .bind("esc", "cancel"),
            ),
            rows: 7,
            shape: 0,
            overlay: false,
            frames: Cell::new(0),
        }
    }
}

impl Application for StressApp {
    fn update(&mut self, signal: Signal) -> UpdateResult {
        let Signal::Event(event) = signal else {
            return UpdateResult::Dirty;
        };
        // Every event churns some state, and some of them change the *shape* of
        // the frame — an overlay appearing, the dock toggling, the row count
        // moving — which is the mid-render mode change a component tree has to
        // absorb between one paint and the next.
        let _ = self.select.handle(&event, self.rows);
        let _ = self.scroll.handle(&event, self.rows, 5);
        let _ = self.input.handle(&event);
        let _ = self.tabs.handle(&event, 4);
        let _ = self.slider.handle(&event);
        let _ = self.form.handle(&event, 2);
        // A focus registry with real members, so focus traversal has somewhere
        // to move and the dock's focused state means something.
        self.focus.begin_frame();
        self.focus.register("main");
        self.focus.register("composer");
        self.focus.register("panel");
        if self.dock.is_focused() {
            self.focus.set_owner("panel");
        }
        let _ = self.focus.handle(&event);
        if let Event::Key(key) = &event {
            // A chord engine holding pending state across a resize: `g g` may be
            // half-entered when the frame it was typed into no longer exists.
            let _ = self.keymap.dispatch(*key);
        }
        self.console.line(format!("{event:?}"));
        // Rows a borrowed view reads must outlive the frame, so the collection
        // the tree renders is rebuilt here rather than in `view`.
        self.tree_rows = (0..self.rows)
            .map(|i| {
                if i == 0 {
                    TreeRow::root(i, Line::from("root 日本語"), true)
                } else {
                    TreeRow::new(i, Some(0), 1, Line::from(format!("child {i} 界")), false)
                }
            })
            .collect();
        match &event {
            Event::Key(key) if key.code == KeyCode::Char('q') => return UpdateResult::Exit,
            Event::Key(key) if key.code == KeyCode::Tab => {
                self.overlay = !self.overlay;
                self.dock.toggle();
            }
            Event::Key(key) if key.code == KeyCode::Enter => {
                self.toasts.push(ToastLevel::Info, "pushed");
                self.rows = (self.rows + 3) % 13;
            }
            Event::Resize { width, height } => {
                self.rows = (*width as usize) % 11;
                self.shape = ((*width as u8) ^ (*height as u8)) % 2;
            }
            _ => {}
        }
        UpdateResult::Dirty
    }

    fn view(&self, frame: u64) -> ScopedElement<'_> {
        self.frames.set(self.frames.get() + 1);
        let rows: Vec<Line<'static>> = (0..self.rows)
            .map(|i| Line::from(format!("row {i} 日本語 👩‍👩‍👦")))
            .collect();

        // Two frame shapes, swapped by resizes. A component tree that changes
        // between one paint and the next is the mid-render mode change a host
        // really performs — a pane opening, a view switching — and the state
        // below it (selection, scroll, tree expansion) does not reset with it.
        let main = if self.shape == 0 {
            element(
                Flex::column()
                    .gap(1)
                    .grow(2, element(SelectList::new(rows.clone(), &self.select)))
                    .grow(1, element(Markdown::new(PROSE)))
                    .fixed(3, element(TextInput::new(&self.input))),
            )
        } else {
            element(
                Flex::row()
                    .gap(1)
                    .grow(
                        1,
                        element(
                            Flex::column()
                                .grow(1, element(TreeList::new(&self.tree_rows, &self.tree)))
                                .fixed(
                                    5,
                                    element(Table::new(
                                        vec![
                                            Column::new(Line::from("name"), Dimension::Flex(1)),
                                            Column::new(Line::from("n"), Dimension::Fixed(4)),
                                        ],
                                        rows.iter()
                                            .map(|row| vec![row.clone(), Line::from("1")])
                                            .collect(),
                                        &self.select,
                                    )),
                                ),
                        ),
                    )
                    .grow(
                        1,
                        element(
                            Flex::column()
                                .grow(1, element(Console::new(&self.console).title(" log ")))
                                .fixed(1, element(Slider::new(&self.slider).label(&self.slider)))
                                .fixed(
                                    4,
                                    element(Form::new(
                                        vec![
                                            FormField::new("field 界", element(Text::raw(PROSE)))
                                                .help(PROSE),
                                            FormField::new("second", element(Text::raw("x")))
                                                .error("bad"),
                                        ],
                                        &self.form,
                                    )),
                                )
                                .fixed(
                                    4,
                                    element(
                                        Diff::new(PROSE, &format!("{PROSE}\nchanged"))
                                            .line_numbers(true),
                                    ),
                                ),
                        ),
                    ),
            )
        };

        let shell = AppShell::new(Tabs::new(
            vec![
                Line::from("one"),
                Line::from("two"),
                Line::from("three"),
                Line::from("four"),
            ],
            &self.tabs,
        ))
        .header(
            Flow::new()
                .gap(1)
                .item(element(Spinner::new(frame)))
                .item(element(Loader::new(frame, "loading 日本語").hint(PROSE)))
                .item(element(
                    ActivityList::new(vec![
                        ActivityItem::new("task 界", ActivityStatus::Running)
                            .detail(PROSE)
                            .progress((frame % 100) as f32 / 100.0),
                        ActivityItem::new("queued", ActivityStatus::Queued),
                    ])
                    .frame(frame),
                ))
                .item(element(
                    StatusBar::new().left(vec![Span::raw(format!("frame {frame}"))]),
                )),
        )
        .top_rule()
        .before_main(Scroll::new(rows, &self.scroll))
        .after_main(ToastList::new(&self.toasts))
        .bottom_rule()
        .status(ProgressBar::determinate((frame % 100) as f32 / 100.0))
        .footer(KeyHints::new([("q", "quit"), ("tab", "overlay")]));

        element(OverlayScene {
            root: element(DockFrame {
                state: self.dock,
                spec: DockSpec::right(40, 24),
                main: element(Flex::column().grow(1, element(shell)).grow(1, main)),
                panel: element(
                    Boxed::new(element(Console::new(&self.console).title(" panel ")))
                        .title(Line::from("dock 界"))
                        .padding(Padding::all(1)),
                ),
            }),
            dialog: self.overlay,
        })
    }
}

/// The dock resolved the way a host resolves it: at render time, against the
/// area the layout actually handed out, with both regions painted through the
/// same surface. Its geometry is a decision per frame, so a resize is exactly
/// what changes the answer.
struct DockFrame<'view> {
    state: DockState,
    spec: DockSpec,
    main: ScopedElement<'view>,
    panel: ScopedElement<'view>,
}

impl View for DockFrame<'_> {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        available
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        let layout = self.state.resolve(area, self.spec);
        self.main.render(layout.main, surface, ctx);
        if let Some(panel) = layout.panel {
            self.panel.render(panel, surface, ctx);
        }
    }
}

/// A borrowed application tree under Tuika-owned overlays.
///
/// `Application::view` hands back a frame-scoped element, so a scene whose root
/// borrows state cannot be built once and returned — [`ScopedScene`] borrows its
/// root for the scene's own lifetime. A host composes it per paint instead,
/// which is what this does: the overlays are rebuilt inside `render`, over a
/// root the frame already owns.
struct OverlayScene<'view> {
    root: ScopedElement<'view>,
    dialog: bool,
}

impl OverlayScene<'_> {
    fn scene(&self) -> ScopedScene<'_, dyn View + '_> {
        let scene = ScopedScene::new(self.root.as_ref());
        if self.dialog {
            scene.dialog(
                Dialog::new("dialog 界", element(Text::raw(PROSE)))
                    .key_hints([("esc", "close")])
                    .dim_backdrop(true),
            )
        } else {
            scene
        }
    }
}

impl View for OverlayScene<'_> {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        self.scene().measure(available, ctx)
    }

    fn measure_request(&self, request: MeasureRequest, ctx: &RenderCtx) -> Size {
        self.scene().measure_request(request, ctx)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        self.scene().render(area, surface, ctx);
    }
}

/// Drive one full session in `mode` over `screen`, returning the frames drawn.
fn run_session(screen: &Screen, mode: ScreenMode, seed: u64, events: usize) -> u64 {
    let runner = Runner::with_clock(
        RunnerConfig {
            // Short enough that ticks land between the storm's events, so
            // animation frames and tick-driven repaints are in the mix too.
            tick_rate: Duration::from_millis(4),
            screen_mode: mode,
        },
        StepClock::new(),
    );
    let scrollback = runner.scrollback();
    let mut terminal = Terminal::with_options(
        screen.handle(),
        TerminalOptions {
            viewport: mode.viewport(),
        },
    )
    .expect("terminal");

    let mut app = StressApp::new();
    // A background producer publishing into the scrollback the whole time: in a
    // split footer these commit above the footer while it is being resized, and
    // in an alternate screen they must be discarded rather than pile up.
    for i in 0..8 {
        scrollback.write(move |width| {
            element(Text::raw(format!("published block {i} at width {width}")))
        });
    }
    runner
        .run_driven_by(
            &mut terminal,
            &Theme::default(),
            &mut app,
            Storm::new(screen.clone(), seed, events),
        )
        .expect("run");

    if mode.is_alternate() {
        let _ = terminal.clear();
    } else {
        let _ = tuika::screen::close_footer(&mut terminal);
    }
    app.frames.get()
}

/// Every mode a host can pick, both capture variants included.
fn all_modes() -> Vec<ScreenMode> {
    vec![
        ScreenMode::Alternate,
        ScreenMode::Alternate.with_mouse_capture(),
        ScreenMode::split_footer(1),
        ScreenMode::split_footer(4),
        ScreenMode::split_footer(4).with_mouse_capture(),
        ScreenMode::default_split_footer(),
    ]
}

/// Strip OSC 8 hyperlink wrappers from a cell symbol, leaving what the terminal
/// actually displays. tuika embeds `ESC ] 8 ; ; url ST` runs in the cells of a
/// linked label, so an escape *there* is the feature; anywhere else it is
/// terminal corruption, and stripping the wrapper is what tells the two apart.
fn strip_osc8(symbol: &str) -> String {
    let mut out = String::with_capacity(symbol.len());
    let mut rest = symbol;
    while let Some(open) = rest.find("\u{1b}]8;;") {
        out.push_str(&rest[..open]);
        let after = &rest[open + "\u{1b}]8;;".len()..];
        match after.find("\u{1b}\\") {
            Some(end) => rest = &after[end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// A control byte left in a cell corrupts the terminal for the rest of the
/// session, so no path through a storm may put one there.
fn assert_no_control_bytes(screen: &Screen, context: &str) {
    screen.with(|backend| {
        let buffer = backend.buffer().clone();
        for y in buffer.area.top()..buffer.area.bottom() {
            for x in buffer.area.left()..buffer.area.right() {
                let symbol = buffer[(x, y)].symbol();
                assert!(
                    !strip_osc8(symbol).chars().any(char::is_control),
                    "{context}: control byte in the cell at ({x}, {y}): {symbol:?}"
                );
            }
        }
    });
}

#[test]
fn every_screen_mode_survives_a_resize_and_input_storm() {
    for mode in all_modes() {
        for seed in SEEDS {
            let screen = Screen::new(40, 12);
            let frames = run_session(&screen, mode, *seed, storm_length());
            assert!(frames > 0, "{mode:?} drew no frame");
            assert_no_control_bytes(&screen, &format!("{mode:?} seed {seed:#x}"));
        }
    }
}

#[test]
fn switching_screen_mode_mid_session_leaves_the_terminal_usable() {
    // A host that swaps renderers on a live screen — a full-screen view opening
    // over a split footer and closing again — must hand the footer's rows back
    // and leave the published scrollback where the user can still read it.
    let modes = all_modes();
    for (index, first) in modes.iter().enumerate() {
        for second in modes.iter().skip(index) {
            let screen = Screen::new(48, 14);
            run_session(&screen, *first, 0x51de_51de_51de_51de, 60);
            // Whatever the storm dragged the window to, the next renderer comes
            // up on that size, not the one it was written for.
            run_session(&screen, *second, 0xfeed_face_0000_0001, 60);
            assert_no_control_bytes(&screen, &format!("{first:?} -> {second:?}"));

            // "Usable" is not just "did not panic": a third renderer must come
            // up correctly on the screen the first two left behind. A footer
            // session at a known size says so — its rows are the last rows and
            // the blocks it publishes are above them.
            let rows = footer_session(&screen, 3, (20, 8), 1);
            assert_eq!(rows.len(), 8, "{first:?} -> {second:?}: {rows:?}");
            assert!(
                rows[5..].iter().all(|row| row.starts_with("FOOTER")),
                "{first:?} -> {second:?}: the footer did not reclaim the last rows: {rows:?}"
            );
            assert!(
                rows[..5].iter().any(|row| row.starts_with("BLOCK0")),
                "{first:?} -> {second:?}: the published block never reached the screen: {rows:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The split footer's geometry, under a screen that keeps moving.
// ---------------------------------------------------------------------------

/// Run a scripted (not randomized) split-footer session: resize to `size`,
/// publish `blocks` marker blocks, then quit. Returns the rows left on screen
/// after the footer is closed.
fn footer_session(screen: &Screen, footer: u16, size: (u16, u16), blocks: usize) -> Vec<String> {
    let mode = ScreenMode::split_footer(footer);
    let runner = Runner::with_clock(
        RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            screen_mode: mode,
        },
        StepClock::new(),
    );
    let scrollback = runner.scrollback();
    let mut terminal = Terminal::with_options(
        screen.handle(),
        TerminalOptions {
            viewport: mode.viewport(),
        },
    )
    .expect("terminal");

    // Published from the update callback rather than before the run: a queue
    // flushed against a screen that is still degenerate is dropped by design,
    // so the blocks are queued once the session has a real size.
    let publisher = scrollback.clone();
    let mut state = ();
    runner
        .run_driven_by(
            &mut terminal,
            &Theme::default(),
            from_fn(
                &mut state,
                |(), _frame| {
                    element(Text::new(
                        (0..64).map(|_| Line::from("FOOTER")).collect::<Vec<_>>(),
                    ))
                },
                |(), signal| match signal {
                    Signal::Event(Event::Key(_)) => UpdateResult::Exit,
                    Signal::Event(Event::Resize { .. }) => {
                        for i in 0..blocks {
                            publisher.write(move |_width| element(Text::raw(format!("BLOCK{i}"))));
                        }
                        UpdateResult::Dirty
                    }
                    _ => UpdateResult::Clean,
                },
            ),
            Script::new(
                screen,
                vec![
                    Event::Resize {
                        width: size.0,
                        height: size.1,
                    },
                    Event::Key(Key::new(KeyCode::Char('q'))),
                ],
            ),
        )
        .expect("run");
    let rows = screen.rows();
    let _ = tuika::screen::close_footer(&mut terminal);
    rows
}

#[test]
fn a_split_footer_stays_pinned_to_the_bottom_at_every_screen_size() {
    // The footer is a *footer*: whatever the screen was resized to between the
    // last frame and this one, its rows are the last rows, and the blocks
    // published before it are above — never overlapping, never floating in the
    // middle of a screen that grew.
    for &footer in &[1u16, 2, 4, 12] {
        for &(start_width, start_height) in SIZES {
            for &(width, height) in SIZES {
                let screen = Screen::new(start_width, start_height);
                let rows = footer_session(&screen, footer, (width, height), 2);
                assert_eq!(rows.len() as u16, height, "the screen is {width}x{height}");
                if width < 6 || height == 0 {
                    // Too narrow to hold either marker, or no rows at all: the only
                    // property left is that nothing panicked and nothing was
                    // painted off-screen, which the backend itself enforces.
                    continue;
                }
                let footer_rows = footer.min(height) as usize;
                let split = rows.len() - footer_rows;
                for (y, row) in rows.iter().enumerate().skip(split) {
                    assert!(
                        row.starts_with("FOOTER"),
                        "{width}x{height} from {start_width}x{start_height} footer {footer}: \
                     row {y} is not the footer: {row:?}"
                    );
                }
                // Rows above the footer belong to the terminal, so what is on them
                // after a resize is whatever the emulator's own reflow left there —
                // not something tuika painted, and not tuika's to clear. The
                // "nothing above" half of the invariant is therefore asserted only
                // where no reflow happened.
                if (start_width, start_height) == (width, height) {
                    for (y, row) in rows.iter().enumerate().take(split) {
                        assert!(
                            !row.starts_with("FOOTER"),
                            "{width}x{height} footer {footer}: the footer painted row {y} above its \
                         own rows: {row:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn closing_a_footer_hands_its_rows_back_and_keeps_the_scrollback() {
    // What a user sees after the program exits: the lines it published are
    // still there to scroll back to, and the rows the footer borrowed are
    // blank with the cursor parked on the first of them, so the shell prompt
    // resumes below the output instead of on top of the last frame.
    let screen = Screen::new(20, 8);
    let before = footer_session(&screen, 3, (20, 8), 2);
    assert!(
        before.iter().any(|row| row.starts_with("BLOCK0")),
        "the published blocks never reached the screen: {before:?}"
    );

    let after = screen.rows();
    assert!(
        after.iter().any(|row| row.starts_with("BLOCK0")),
        "closing the footer lost the published scrollback: {after:?}"
    );
    assert!(
        after[5..].iter().all(|row| row.trim().is_empty()),
        "the footer kept its rows after close_footer: {after:?}"
    );
    let cursor = screen.with(|backend| backend.get_cursor_position().expect("cursor"));
    assert_eq!(
        cursor.y, 5,
        "the cursor did not park on the first row handed back: {after:?}"
    );
}

// ---------------------------------------------------------------------------
// Publishing: adversarial blocks against a screen that may not exist.
// ---------------------------------------------------------------------------

#[test]
fn publishing_survives_adversarial_blocks_and_a_degenerate_screen() {
    // Every way a host can hand the queue something unreasonable: a block that
    // measures itself taller than the terminal will ever be, one pinned to zero
    // rows, one pinned past the clamp, one full of control bytes, and a
    // producer on another thread — all flushed against screens down to 0x0.
    for &(width, height) in SIZES {
        let screen = Screen::new(width.max(1), height.max(1));
        let mode = ScreenMode::split_footer(2);
        let runner = Runner::with_clock(
            RunnerConfig {
                tick_rate: Duration::from_secs(3600),
                screen_mode: mode,
            },
            StepClock::new(),
        );
        let scrollback = runner.scrollback();
        let mut terminal = Terminal::with_options(
            screen.handle(),
            TerminalOptions {
                viewport: mode.viewport(),
            },
        )
        .expect("terminal");

        scrollback.write(|_width| element(Text::new(vec![Line::from("tall"); 9_000])));
        scrollback.write_rows(0, |_width| element(Text::raw("zero rows")));
        scrollback.write_rows(u16::MAX, |_width| element(Text::raw("past the clamp")));
        scrollback.write_lines(vec![Line::from("bell\u{7}esc\u{1b}del\u{7f}")]);
        scrollback.write(|width| element(Markdown::new(format!("# {width}\n\n{PROSE}"))));
        let producer = scrollback.clone();
        std::thread::spawn(move || producer.write_lines(vec![Line::from("from another thread")]))
            .join()
            .expect("producer");

        let mut state = ();
        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                from_fn(
                    &mut state,
                    |(), _frame| element(Text::raw("footer")),
                    |(), signal| match signal {
                        Signal::Event(Event::Key(_)) => UpdateResult::Exit,
                        _ => UpdateResult::Clean,
                    },
                ),
                Script::new(
                    &screen,
                    vec![
                        Event::Resize { width, height },
                        Event::Key(Key::new(KeyCode::Char('q'))),
                    ],
                ),
            )
            .expect("run");

        assert!(
            scrollback.is_empty(),
            "{width}x{height} left blocks queued after the run"
        );
        assert_no_control_bytes(&screen, &format!("publishing at {width}x{height}"));
    }
}

#[test]
fn an_alternate_screen_discards_published_blocks_instead_of_queueing_them() {
    // There is no scrollback of the host's to write into, so a background
    // producer that keeps publishing must not grow the queue without bound.
    let screen = Screen::new(24, 8);
    let runner = Runner::with_clock(
        RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            screen_mode: ScreenMode::Alternate,
        },
        StepClock::new(),
    );
    let scrollback = runner.scrollback();
    let mut terminal = Terminal::new(screen.handle()).expect("terminal");
    for i in 0..1_000 {
        scrollback.write(move |_width| element(Text::raw(format!("discarded {i}"))));
    }

    let mut state = ();
    runner
        .run_driven_by(
            &mut terminal,
            &Theme::default(),
            from_fn(
                &mut state,
                |(), _frame| element(Text::raw("alternate")),
                |(), signal| match signal {
                    Signal::Event(Event::Key(_)) => UpdateResult::Exit,
                    _ => UpdateResult::Clean,
                },
            ),
            scripted_events([Event::Key(Key::new(KeyCode::Char('q')))]),
        )
        .expect("run");

    assert!(
        scrollback.is_empty(),
        "the queue grew on the alternate screen"
    );
    assert!(
        !screen.rows().iter().any(|row| row.contains("discarded")),
        "a block reached the alternate screen: {:?}",
        screen.rows()
    );
}

#[test]
fn a_block_published_on_a_resize_is_rendered_at_the_new_width() {
    // The most ordinary publishing moment there is: the terminal is resized and
    // the host writes a line about it. A block is rendered and inserted at the
    // geometry the terminal last knew, and the loop commits queued blocks
    // *before* it paints the next frame — so the size has to be learned first,
    // or the block lands wrapped to the previous width.
    let screen = Screen::new(30, 10);
    let mode = ScreenMode::split_footer(2);
    let runner = Runner::with_clock(
        RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            screen_mode: mode,
        },
        StepClock::new(),
    );
    let scrollback = runner.scrollback();
    let mut terminal = Terminal::with_options(
        screen.handle(),
        TerminalOptions {
            viewport: mode.viewport(),
        },
    )
    .expect("terminal");

    let published = scrollback.clone();
    let mut state = ();
    runner
        .run_driven_by(
            &mut terminal,
            &Theme::default(),
            from_fn(
                &mut state,
                |(), _frame| element(Text::raw("footer")),
                |(), signal| match signal {
                    Signal::Event(Event::Resize { .. }) => {
                        published.write(|width| element(Text::raw(format!("W{width}"))));
                        UpdateResult::Dirty
                    }
                    Signal::Event(Event::Key(_)) => UpdateResult::Exit,
                    _ => UpdateResult::Clean,
                },
            ),
            Script::new(
                &screen,
                vec![
                    Event::Resize {
                        width: 12,
                        height: 10,
                    },
                    Event::Key(Key::new(KeyCode::Char('q'))),
                ],
            ),
        )
        .expect("run");

    let rows = screen.rows();
    assert!(
        rows.iter().any(|row| row.starts_with("W12")),
        "the block was published at the width the terminal had before the resize: {rows:?}"
    );
}

// ---------------------------------------------------------------------------
// Shell chrome, overlays, and docks: every shape, every degenerate size.
// ---------------------------------------------------------------------------

/// Paint `view` into an inset rect of a margined buffer and assert it stayed
/// inside, wrote no control byte, and painted the same cells twice.
fn assert_contained(name: &str, view: &dyn View, theme: &Theme, sheet: StyleSheet) {
    const MARGIN: u16 = 2;
    for &(width, height) in SIZES {
        let outer = Rect::new(0, 0, width + MARGIN * 2, height + MARGIN * 2);
        let inner = Rect::new(MARGIN, MARGIN, width, height);
        let mut first = tuika::buffer::Buffer::empty(outer);
        paint_with_sheet(&mut first, inner, theme, sheet, view, &[]);

        let blank = BufferCell::default();
        for y in outer.top()..outer.bottom() {
            for x in outer.left()..outer.right() {
                let cell = &first[(x, y)];
                if !inner.contains(Position::new(x, y)) {
                    assert_eq!(
                        cell, &blank,
                        "{name} at {width}x{height} painted ({x}, {y}) outside {inner:?}"
                    );
                    continue;
                }
                assert!(
                    !cell
                        .symbol()
                        .chars()
                        .any(|ch| ch.is_control() && ch != '\u{1b}'),
                    "{name} at {width}x{height} put a control byte at ({x}, {y}): {:?}",
                    cell.symbol()
                );
            }
        }

        let mut second = tuika::buffer::Buffer::empty(outer);
        paint_with_sheet(&mut second, inner, theme, sheet, view, &[]);
        assert_eq!(
            first, second,
            "{name} at {width}x{height} painted differently on the second frame"
        );
    }
}

/// Build one `AppShell` shape from a five-bit mask over its optional regions.
fn shell_shape(mask: u8, main: ScopedElement<'static>) -> AppShell<'static> {
    let mut shell = AppShell::new(main);
    if mask & 1 != 0 {
        shell = shell.header(StatusBar::new().left(vec![Span::raw("header 日本語 👩‍👩‍👦")]));
    }
    if mask & 2 != 0 {
        shell = shell.top_rule();
    }
    if mask & 4 != 0 {
        shell = shell.status(ProgressBar::determinate(0.5).label("status").percent(true));
    }
    if mask & 8 != 0 {
        shell = shell.bottom_rule();
    }
    if mask & 16 != 0 {
        shell = shell.footer(KeyHints::new([("ctrl+c", "quit 界"), ("?", "help")]));
    }
    shell
}

#[test]
fn every_shell_shape_stays_inside_its_rect_at_every_size() {
    // A shell is chrome around one growing region, and the chrome is what runs
    // out of room first. Every combination of header, rules, status, and footer
    // is squeezed into every degenerate geometry, over a main content that is
    // itself unbounded.
    let theme = Theme::default();
    let sheet = StyleSheet::from_theme(&theme);
    for mask in 0..32u8 {
        let main = element(
            Flex::column()
                .gap(1)
                .grow(1, element(Markdown::new(PROSE)))
                .fixed(4, element(CodeBlock::new("rust", PROSE).line_numbers(true))),
        );
        assert_contained(
            &format!("shell mask {mask:05b}"),
            &shell_shape(mask, main),
            &theme,
            sheet,
        );
    }
}

#[test]
fn a_shell_under_every_theme_and_stylesheet_paints_the_same_cells_twice() {
    // Styling is resolved per frame from the theme and the stylesheet. Swapping
    // either between frames — a live theme switch, a host installing its own
    // sheet — must change the colors and nothing else: no hidden per-render
    // state, no drift in what is painted where.
    for named in tuika::themes::PRESETS {
        for sheet in [
            StyleSheet::from_theme(&named.theme),
            // A host that overrides roles: every accent flattened to one color
            // and every rule bolded, which is the shape of a real restyle.
            {
                let mut sheet = StyleSheet::from_theme(&named.theme);
                sheet.heading = StyleBundle::new().fg(named.theme.accent).bold();
                sheet.rule = StyleBundle::new().fg(named.theme.accent).bold();
                sheet.panel = StyleBundle::new().bg(named.theme.surface);
                sheet.key_hint_key = StyleBundle::new().bold();
                sheet
            },
            // And one that sets nothing at all, so every role falls back.
            {
                let mut sheet = StyleSheet::from_theme(&named.theme);
                sheet.heading = StyleBundle::new();
                sheet.link = StyleBundle::new();
                sheet.rule = StyleBundle::new();
                sheet
            },
        ] {
            let shell = shell_shape(0b11111, element(Markdown::new(PROSE)));
            assert_contained(
                &format!("shell under {}", named.name),
                &shell,
                &named.theme,
                sheet,
            );
        }
    }
}

#[test]
fn an_overlay_never_resolves_outside_the_screen_it_was_given() {
    // Anchored placement is arithmetic on a screen that may be smaller than the
    // overlay's own minimum, or smaller than twice its margin. The resolved
    // rect is what a host paints into, so it has to stay on screen whatever the
    // request was.
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
    let sides = [
        TargetSide::Above,
        TargetSide::Below,
        TargetSide::Left,
        TargetSide::Right,
    ];
    let aligns = [TargetAlign::Start, TargetAlign::Center, TargetAlign::End];

    for &(width, height) in SIZES {
        let screen = Rect::new(0, 0, width, height);
        for anchor in anchors {
            for (pct_w, pct_h) in [(0u16, 0u16), (50, 40), (100, 100), (200, 200)] {
                for margin in [0u16, 1, 40] {
                    let spec = OverlaySpec::centered(pct_w, pct_h)
                        .anchor(anchor)
                        .min_size(34, 7)
                        .max_size(4, 2)
                        .margin(margin);
                    let rect = spec.resolve(screen);
                    assert!(
                        rect.right() <= screen.right() && rect.bottom() <= screen.bottom(),
                        "{anchor:?} at {width}x{height} margin {margin} resolved {rect:?} \
                         outside {screen:?}"
                    );

                    // The same spec, placed against a target that may itself
                    // straddle or sit outside the screen.
                    for target in [
                        Rect::new(0, 0, 1, 1),
                        Rect::new(width / 2, height / 2, 10, 4),
                        Rect::new(width.saturating_sub(1), height.saturating_sub(1), 30, 10),
                    ] {
                        for side in sides {
                            for align in aligns {
                                let placement =
                                    TargetPlacement::new(side).align(align).gap(2).flip(true);
                                let rect = spec.resolve_target(screen, target, placement);
                                assert!(
                                    rect.right() <= screen.right()
                                        && rect.bottom() <= screen.bottom(),
                                    "{side:?}/{align:?} at {width}x{height} against {target:?} \
                                     resolved {rect:?} outside {screen:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn a_dock_never_hands_out_a_panel_outside_the_frame() {
    // The dock is the other responsive decision a shell makes: docked beside
    // the content, a drawer over it, or hidden. Its two rects come back to the
    // host as areas to paint, so neither may leave the frame and the main area
    // may not vanish while the panel is merely passive.
    for &(width, height) in SIZES {
        let area = Rect::new(0, 0, width, height);
        for spec in [
            DockSpec::right(0, 0),
            DockSpec::right(40, 24),
            DockSpec::left(1, u16::MAX),
            DockSpec::left(u16::MAX, 3),
        ] {
            for state in [
                DockState::new(),
                {
                    let mut state = DockState::new();
                    state.show_passive();
                    state
                },
                {
                    let mut state = DockState::new();
                    state.focus();
                    state
                },
            ] {
                let layout = state.resolve(area, spec);
                assert!(
                    layout.main.right() <= area.right() && layout.main.bottom() <= area.bottom(),
                    "{spec:?} at {width}x{height} left main {:?} outside {area:?}",
                    layout.main
                );
                if let Some(panel) = layout.panel {
                    assert!(
                        panel.right() <= area.right() && panel.bottom() <= area.bottom(),
                        "{spec:?} at {width}x{height} placed the panel {panel:?} outside {area:?}"
                    );
                    assert_ne!(
                        layout.placement,
                        DockPlacement::Hidden,
                        "a hidden dock still handed out {panel:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_scene_full_of_overlays_stays_inside_the_screen() {
    // Dialogs over a shell that is already out of room. Scene overlays are
    // owned, so this is the composition a host builds from `Scene`; the
    // borrowed form goes through `paint`'s overlay slice, below.
    let theme = Theme::default();
    let sheet = StyleSheet::from_theme(&theme);
    let scene = Scene::new(element(shell_shape(0b11111, element(Markdown::new(PROSE)))))
        .dialog(
            Dialog::new("dialog 日本語", element(Text::raw(PROSE)))
                .key_hints([("esc", "close")])
                .dim_backdrop(true)
                .min_size(40, 12),
        )
        .layer(
            element(Boxed::new(element(Text::raw(PROSE))).title(Line::from("layer"))),
            OverlaySpec::centered(60, 30)
                .anchor(Anchor::BottomRight)
                .margin(1),
        )
        .layer(
            element(AsciiFont::new("OVER")),
            OverlaySpec::centered(100, 100)
                .anchor(Anchor::Top)
                .margin(2),
        );

    assert_eq!(scene.overlay_count(), 3);
    assert_contained("scene with three overlays", &scene, &theme, sheet);
}

#[test]
fn borrowed_overlays_stay_inside_the_screen_at_every_size() {
    // The other half of the same picture: overlays whose views borrow host
    // state — a toast stack and a completion palette — can only reach a frame
    // through `paint`'s overlay slice, since a `Scene` layer is owned. They are
    // painted last, into rects the spec resolved, which is the last place an
    // off-by-one can reach a neighbouring cell.
    const MARGIN: u16 = 2;
    let theme = Theme::default();
    let sheet = StyleSheet::from_theme(&theme);

    let items: Vec<CompletionItem> = (0..12)
        .map(|i| CompletionItem::new(format!("item {i} 日本語")).detail(PROSE))
        .collect();
    let mut completion = CompletionState::new();
    completion.sync("it", &items);
    let mut toasts = Toasts::new(4);
    toasts.push(ToastLevel::Error, PROSE);
    toasts.push(ToastLevel::Info, "second 界");

    let toast_list = ToastList::new(&toasts);
    let palette = CompletionPalette::new(&items, &completion).title("completions");
    let root = shell_shape(0b11111, element(Markdown::new(PROSE)));

    for &(width, height) in SIZES {
        let outer = Rect::new(0, 0, width + MARGIN * 2, height + MARGIN * 2);
        let inner = Rect::new(MARGIN, MARGIN, width, height);
        let specs = [
            OverlaySpec::centered(60, 30)
                .anchor(Anchor::BottomRight)
                .margin(1),
            OverlaySpec::centered(100, 100)
                .anchor(Anchor::Top)
                .margin(2),
        ];
        let views: [&dyn View; 2] = [&toast_list, &palette];
        let overlays: Vec<Overlay<'_>> = specs
            .iter()
            .zip(views)
            .map(|(spec, view)| Overlay {
                area: spec.resolve(inner),
                view,
                clear: true,
            })
            .collect();

        let mut buffer = tuika::buffer::Buffer::empty(outer);
        paint_with_sheet(&mut buffer, inner, &theme, sheet, &root, &overlays);

        let blank = BufferCell::default();
        for y in outer.top()..outer.bottom() {
            for x in outer.left()..outer.right() {
                if inner.contains(Position::new(x, y)) {
                    continue;
                }
                assert_eq!(
                    &buffer[(x, y)],
                    &blank,
                    "a borrowed overlay at {width}x{height} painted ({x}, {y}) outside {inner:?}"
                );
            }
        }
    }
}

#[test]
fn a_terminal_resized_before_the_first_frame_is_pinned_at_its_new_size() {
    // Startup is not exempt from resizing: a window dragged between the
    // terminal being constructed and the loop's first frame leaves the
    // `Terminal` holding a geometry that no longer exists, and pinning the
    // footer writes rows at whatever geometry it last knew.
    let screen = Screen::new(30, 10);
    let mode = ScreenMode::split_footer(2);
    let runner = Runner::with_clock(
        RunnerConfig {
            tick_rate: Duration::from_secs(3600),
            screen_mode: mode,
        },
        StepClock::new(),
    );
    let mut terminal = Terminal::with_options(
        screen.handle(),
        TerminalOptions {
            viewport: mode.viewport(),
        },
    )
    .expect("terminal");
    screen.request_resize(11, 6);

    let mut state = ();
    runner
        .run_driven_by(
            &mut terminal,
            &Theme::default(),
            from_fn(
                &mut state,
                |(), _frame| element(Text::raw("FOOTERFOOTERFOOTER")),
                |(), signal| match signal {
                    Signal::Event(Event::Key(_)) => UpdateResult::Exit,
                    _ => UpdateResult::Clean,
                },
            ),
            scripted_events([Event::Key(Key::new(KeyCode::Char('q')))]),
        )
        .expect("run");

    let rows = screen.rows();
    assert_eq!(rows.len(), 6, "the frame was painted at the stale height");
    assert!(
        rows.iter().all(|row| row.chars().count() == 11),
        "the frame was painted at the stale width: {rows:?}"
    );
    assert!(
        rows[4..].iter().any(|row| row.starts_with("FOOTER")),
        "the footer did not land on the last rows of the resized screen: {rows:?}"
    );
}

// ---------------------------------------------------------------------------
// The same session, on the async runner.
// ---------------------------------------------------------------------------

/// The async half of the sweep. `AsyncRunner` runs the same loop over Tokio
/// timers and event streams, and keeps its own copy of the split-footer
/// sequencing — so the ordering invariants have to be asserted here too, not
/// inferred from the synchronous loop passing.
#[cfg(feature = "async")]
mod asynchronous {
    use super::*;
    use tokio_stream::Stream;

    /// The same application, driven by the async runner. The synchronous
    /// `Application` impl is reused verbatim so both loops are stressed by one
    /// state machine rather than two that could drift apart.
    impl AsyncApplication for StressApp {
        async fn update(&mut self, signal: Signal) -> UpdateResult {
            Application::update(self, signal)
        }

        fn view(&self, frame: u64) -> ScopedElement<'_> {
            Application::view(self, frame)
        }
    }

    /// The scripted source as a `Stream`, dragging the window in step with the
    /// `Resize` events it yields.
    struct AsyncScript {
        screen: Screen,
        events: std::vec::IntoIter<Event>,
    }

    impl AsyncScript {
        fn new(screen: &Screen, events: Vec<Event>) -> Self {
            Self {
                screen: screen.clone(),
                events: events.into_iter(),
            }
        }
    }

    impl Stream for AsyncScript {
        type Item = Result<Event, Infallible>;

        fn poll_next(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            let this = self.get_mut();
            let event = this.events.next();
            if let Some(Event::Resize { width, height }) = event {
                this.screen.request_resize(width, height);
            }
            std::task::Poll::Ready(event.map(Ok))
        }
    }

    /// A storm as a finite script, since an async stream ends the run when it
    /// runs dry rather than idling.
    fn storm_script(seed: u64, events: usize) -> Vec<Event> {
        let mut rng = Rng(seed);
        let mut script = Vec::with_capacity(events + 1);
        for _ in 0..events {
            script.push(match rng.below(4) {
                0 => {
                    let &(width, height) = rng.pick(SIZES);
                    Event::Resize { width, height }
                }
                1 => Event::Mouse(Mouse::at(
                    MouseKind::Drag(MouseButton::Left),
                    rng.below(220) as u16,
                    rng.below(70) as u16,
                )),
                2 => Event::Paste("pasted 日本語 👩‍👩‍👦".into()),
                _ => Event::Key(Key::new(KeyCode::Down)),
            });
        }
        script.push(Event::Key(Key::new(KeyCode::Char('q'))));
        script
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn every_screen_mode_survives_a_resize_storm_on_the_async_runner() {
        for mode in all_modes() {
            for seed in SEEDS.iter().take(2) {
                let screen = Screen::new(40, 12);
                let runner = AsyncRunner::new(RunnerConfig {
                    tick_rate: Duration::from_millis(4),
                    screen_mode: mode,
                });
                let scrollback = runner.scrollback();
                let mut terminal = Terminal::with_options(
                    screen.handle(),
                    TerminalOptions {
                        viewport: mode.viewport(),
                    },
                )
                .expect("terminal");
                for i in 0..8 {
                    scrollback.write(move |width| {
                        element(Text::raw(format!("published {i} at {width}")))
                    });
                }

                let mut app = StressApp::new();
                runner
                    .run_driven_by(
                        &mut terminal,
                        &Theme::default(),
                        &mut app,
                        AsyncScript::new(&screen, storm_script(*seed, storm_length().min(150))),
                        no_messages(),
                    )
                    .await
                    .expect("run");

                if mode.is_alternate() {
                    let _ = terminal.clear();
                } else {
                    let _ = tuika::screen::close_footer(&mut terminal);
                }
                assert_no_control_bytes(&screen, &format!("async {mode:?} seed {seed:#x}"));
            }
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_block_published_on_a_resize_is_rendered_at_the_new_width() {
        // The synchronous loop's invariant, asserted against the async one: the
        // queue is drained before the next frame, so the size has to be learned
        // before the block is rendered and inserted.
        let screen = Screen::new(30, 10);
        let mode = ScreenMode::split_footer(2);
        let runner = AsyncRunner::new(RunnerConfig {
            tick_rate: Duration::from_millis(4),
            screen_mode: mode,
        });
        let scrollback = runner.scrollback();
        let mut terminal = Terminal::with_options(
            screen.handle(),
            TerminalOptions {
                viewport: mode.viewport(),
            },
        )
        .expect("terminal");

        let published = scrollback.clone();
        let mut state = ();
        runner
            .run_driven_by(
                &mut terminal,
                &Theme::default(),
                async_from_fn(
                    &mut state,
                    |(), _frame| element(Text::raw("footer")),
                    async |(), signal| match signal {
                        Signal::Event(Event::Resize { .. }) => {
                            published.write(|width| element(Text::raw(format!("W{width}"))));
                            UpdateResult::Dirty
                        }
                        Signal::Event(Event::Key(_)) => UpdateResult::Exit,
                        _ => UpdateResult::Clean,
                    },
                ),
                AsyncScript::new(
                    &screen,
                    vec![
                        Event::Resize {
                            width: 12,
                            height: 10,
                        },
                        Event::Key(Key::new(KeyCode::Char('q'))),
                    ],
                ),
                no_messages(),
            )
            .await
            .expect("run");

        let rows = screen.rows();
        assert!(
            rows.iter().any(|row| row.starts_with("W12")),
            "the block was published at the width the terminal had before the resize: {rows:?}"
        );
    }
}
