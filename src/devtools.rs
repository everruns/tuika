//! A devtools inspector: what the frame is doing, drawn over the frame itself.
//!
//! Behind `feature = "devtools"`, so a release build of a host compiles none of
//! it. An [`Inspector`] is a cheap, shareable handle that collects what tuika
//! already knows but never shows — the screen rects views were painted into,
//! which surface owns input and how the last event was routed, the keymap's
//! pending sequence, how long the last frame took to paint and how many cells
//! it changed on the terminal — and paints it as a panel on top of the frame,
//! with optional outlines around the traced views.
//!
//! Attach one to a runner and the runner does the bookkeeping: it intercepts the
//! toggle chord ([`DEFAULT_TOGGLE`], `alt+i`, unless replaced), logs every
//! delivered event with the [`UpdateResult`] it produced, times the paint,
//! counts the damaged cells, and paints the panel last.
//!
//! ```no_run
//! use tuika::devtools::Inspector;
//! use tuika::prelude::*;
//!
//! let inspector = Inspector::new();
//! let runner = Runner::new(RunnerConfig::default()).with_inspector(inspector.clone());
//! # let _ = runner;
//! ```
//!
//! The rest is state only the host has, so the host reports it — typically from
//! `update`, right where it already has the value in hand:
//!
//! - [`trace`](Inspector::trace) wraps a view so its painted rect is listed and
//!   outlined, the way [`RectProbe`](crate::probe::RectProbe) records one;
//! - [`record_focus`](Inspector::record_focus) snapshots a [`FocusRegistry`];
//! - [`record_route`](Inspector::record_route) annotates the event being handled
//!   with its [`Delivery`];
//! - [`record_keymap`](Inspector::record_keymap) shows a pending multi-stroke
//!   sequence.
//!
//! A host that drives its own loop calls the runner's half itself:
//! [`handle`](Inspector::handle) before routing an event,
//! [`log_event`](Inspector::log_event) after, [`begin_frame`](Inspector::begin_frame)
//! and [`paint`](Inspector::paint) around painting, and
//! [`record_frame`](Inspector::record_frame) with
//! [`Terminal::last_damage`](crate::term::terminal::Terminal::last_damage) once
//! the frame is flushed.
//!
//! # What it deliberately does not record
//!
//! Paste contents. The log shows a paste's length, never its text: a debugging
//! overlay is exactly what ends up in a screenshot or a screen share, and a
//! paste is where secrets travel.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::buffer::Buffer;
use crate::event::{Event, KeyCode, MouseButton, MouseKind};
use crate::focus::FocusRegistry;
use crate::geometry::{Rect, Size};
use crate::keymap::{Chord, Keymap};
use crate::routing::{Delivery, RouteStage};
use crate::runner::UpdateResult;
use crate::style::{BorderStyle, Modifier, Style, Theme};
use crate::surface::Surface;
use crate::view::{RenderCtx, ScopedElement, View, element};
use crate::width::str_cols;

/// The chord that cycles the inspector when no other is configured.
///
/// `alt+i` rather than a function key, because tuika's [`KeyCode`] has no
/// function keys, and rather than a Ctrl chord, because those collide with
/// line-editing and shell conventions an application is likely to bind.
pub const DEFAULT_TOGGLE: &str = "alt+i";

/// How many events the log keeps unless [`Inspector::event_capacity`] says
/// otherwise.
pub const DEFAULT_EVENT_CAPACITY: usize = 64;

/// How many frame timings feed the sparkline, average, and worst case.
const FRAME_HISTORY: usize = 48;

/// What the inspector currently draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InspectorMode {
    /// Nothing; the frame is painted untouched.
    #[default]
    Hidden,
    /// The panel only.
    Panel,
    /// The panel, plus an outline and label around every traced view.
    Outlines,
}

impl InspectorMode {
    /// The mode the toggle chord moves to from this one:
    /// hidden → panel → outlines → hidden.
    pub fn next(self) -> Self {
        match self {
            Self::Hidden => Self::Panel,
            Self::Panel => Self::Outlines,
            Self::Outlines => Self::Hidden,
        }
    }

    /// Whether anything is drawn.
    pub fn is_visible(self) -> bool {
        self != Self::Hidden
    }
}

/// Timing and damage of the most recent recorded frame, plus a summary of the
/// recent history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// How many frames have been recorded, this one included.
    pub frames: u64,
    /// How long building and painting the application's tree took.
    pub paint: Duration,
    /// How many cells the terminal flush wrote.
    pub damage: usize,
    /// Mean paint time over the recent history.
    pub average: Duration,
    /// Slowest paint time over the recent history.
    pub worst: Duration,
}

#[derive(Clone, Debug)]
struct EventRecord {
    seq: u64,
    /// The event, described without its payload.
    summary: String,
    /// What a coalesced run of identical events shares, so a drag or a scroll
    /// burst is one line with a count rather than a page of near-duplicates.
    kind: String,
    repeats: u32,
    route: Option<String>,
    result: UpdateResult,
}

#[derive(Clone, Debug, Default)]
struct FocusSnapshot {
    active: Option<String>,
    owner: Option<String>,
    ring: Vec<String>,
}

#[derive(Debug)]
struct State {
    mode: InspectorMode,
    toggle: Chord,
    capacity: usize,
    events: VecDeque<EventRecord>,
    next_seq: u64,
    pending_route: Option<String>,
    history: VecDeque<Duration>,
    stats: FrameStats,
    focus: Option<FocusSnapshot>,
    keys: Option<String>,
    traces: Vec<(String, Rect)>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            mode: InspectorMode::Hidden,
            toggle: Chord::parse(DEFAULT_TOGGLE).expect("the default toggle chord parses"),
            capacity: DEFAULT_EVENT_CAPACITY,
            events: VecDeque::new(),
            next_seq: 0,
            pending_route: None,
            history: VecDeque::new(),
            stats: FrameStats::default(),
            focus: None,
            keys: None,
            traces: Vec::new(),
        }
    }
}

/// A shareable handle collecting one application's devtools data.
///
/// Clones share the same data, so the copy a runner holds and the copy the
/// application records into are one inspector. See the [module docs](self).
#[derive(Clone, Debug, Default)]
pub struct Inspector(Arc<Mutex<State>>);

impl Inspector {
    /// A hidden inspector toggled by [`DEFAULT_TOGGLE`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the chord that cycles the inspector's mode.
    pub fn toggle_key(self, chord: Chord) -> Self {
        self.state().toggle = chord;
        self
    }

    /// Keep at most `capacity` events in the log (at least one).
    pub fn event_capacity(self, capacity: usize) -> Self {
        {
            let mut state = self.state();
            state.capacity = capacity.max(1);
            let capacity = state.capacity;
            while state.events.len() > capacity {
                state.events.pop_front();
            }
        }
        self
    }

    /// Start in `mode` instead of hidden — for an application that wants the
    /// panel open from its first frame.
    pub fn with_mode(self, mode: InspectorMode) -> Self {
        self.set_mode(mode);
        self
    }

    /// What the inspector currently draws.
    pub fn mode(&self) -> InspectorMode {
        self.state().mode
    }

    /// Change what the inspector draws.
    pub fn set_mode(&self, mode: InspectorMode) {
        self.state().mode = mode;
    }

    /// Cycle the mode when `event` is the toggle chord, returning whether it
    /// was. A host gives the inspector this refusal before routing an event, so
    /// the chord never reaches the application.
    pub fn handle(&self, event: &Event) -> bool {
        let Event::Key(key) = event else {
            return false;
        };
        let mut state = self.state();
        if Chord::from_key(*key) != state.toggle {
            return false;
        }
        state.mode = state.mode.next();
        true
    }

    /// Append a delivered event to the log, with what `update` answered and the
    /// route recorded while it was handled.
    ///
    /// Consecutive events of one kind with one answer — pointer moves, a drag,
    /// a scroll burst — fold into one line with a repeat count.
    pub fn log_event(&self, event: &Event, result: UpdateResult) {
        let (kind, summary) = describe(event);
        let mut state = self.state();
        let route = state.pending_route.take();
        state.next_seq += 1;
        let seq = state.next_seq;
        if let Some(last) = state.events.back_mut()
            && last.kind == kind
            && last.result == result
            && last.route == route
            && coalesces(event)
        {
            last.repeats = last.repeats.saturating_add(1);
            last.summary = summary;
            last.seq = seq;
            return;
        }
        state.events.push_back(EventRecord {
            seq,
            summary,
            kind,
            repeats: 1,
            route,
            result,
        });
        while state.events.len() > state.capacity {
            state.events.pop_front();
        }
    }

    /// Record how the event currently being handled was routed. The next
    /// [`log_event`](Self::log_event) — the runner's, after `update` returns —
    /// attaches it to that event's line.
    pub fn record_route(&self, delivery: &Delivery<'_>) {
        let stage = match delivery.stage {
            RouteStage::Pre => "pre",
            RouteStage::Always => "always",
            RouteStage::Target => "target",
            RouteStage::Fallback => "fallback",
            RouteStage::Undelivered => "undelivered",
        };
        let route = match delivery.target {
            Some(target) => format!("{stage} {target} {}", outcome(delivery.outcome)),
            None if delivery.stage == RouteStage::Undelivered => stage.to_owned(),
            None => format!("{stage} {}", outcome(delivery.outcome)),
        };
        self.state().pending_route = Some(route);
    }

    /// Snapshot which surface owns input, whether an overlay holds it, and the
    /// focus ring in tab order.
    pub fn record_focus(&self, focus: &FocusRegistry) {
        let snapshot = FocusSnapshot {
            active: focus.active().map(str::to_owned),
            owner: focus.owner().map(str::to_owned),
            ring: focus.ring().to_vec(),
        };
        self.state().focus = Some(snapshot);
    }

    /// Snapshot the keymap's pending multi-stroke sequence, if any.
    pub fn record_keymap<C: Clone>(&self, keymap: &Keymap<C>) {
        let pending = keymap.pending();
        let keys = (!pending.is_empty()).then(|| {
            pending
                .iter()
                .copied()
                .map(key_label)
                .collect::<Vec<_>>()
                .join(" ")
        });
        self.state().keys = keys;
    }

    /// Start a frame: forget the rects traced by the previous one. A runner
    /// calls this before building the tree.
    pub fn begin_frame(&self) {
        self.state().traces.clear();
    }

    /// Record a finished frame: how long the application's tree took to build
    /// and paint, and how many cells the terminal flush wrote.
    pub fn record_frame(&self, paint: Duration, damage: usize) {
        let mut state = self.state();
        state.history.push_back(paint);
        while state.history.len() > FRAME_HISTORY {
            state.history.pop_front();
        }
        let total: Duration = state.history.iter().sum();
        let average = total / state.history.len() as u32;
        let worst = state.history.iter().copied().max().unwrap_or_default();
        state.stats = FrameStats {
            frames: state.stats.frames + 1,
            paint,
            damage,
            average,
            worst,
        };
    }

    /// Timing and damage of the most recent recorded frame.
    pub fn last_frame(&self) -> FrameStats {
        self.state().stats
    }

    /// The rects traced during the current frame, in paint order.
    pub fn traces(&self) -> Vec<(String, Rect)> {
        self.state().traces.clone()
    }

    /// Wrap `view` so the rect it is painted into is listed in the panel and
    /// outlined in [`InspectorMode::Outlines`]. Layout-transparent: the view
    /// measures and draws exactly as it would unwrapped.
    pub fn trace<'view, V: View + 'view>(
        &self,
        name: impl Into<String>,
        view: V,
    ) -> ScopedElement<'view> {
        element(Traced {
            name: name.into(),
            inspector: self.clone(),
            inner: view,
        })
    }

    /// Paint the inspector over `buffer` within `area`, according to its mode.
    /// Call it last, after the application's tree, so it sits on top.
    pub fn paint(&self, buffer: &mut Buffer, area: Rect, theme: &Theme) {
        let area = area.intersection(buffer.area);
        let state = self.state();
        if !state.mode.is_visible() || area.width == 0 || area.height == 0 {
            return;
        }
        let mut surface = Surface::new(buffer, area);
        if state.mode == InspectorMode::Outlines {
            paint_outlines(&mut surface, &state.traces, theme);
        }
        paint_panel(&mut surface, panel_rect(area), &state, theme);
    }

    fn state(&self) -> MutexGuard<'_, State> {
        // A panic while the lock was held leaves debugging data, not an
        // invariant, half-written; keep showing it rather than poisoning every
        // later frame.
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A view that reports the rect it is painted into to an [`Inspector`].
struct Traced<V> {
    name: String,
    inspector: Inspector,
    inner: V,
}

impl<V: View> View for Traced<V> {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        self.inner.measure(available, ctx)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        // A layout-only pass (`probe::resolve_layout`) is followed by the real
        // paint, which records the trace; recording both would list it twice.
        if !ctx.is_layout_pass() {
            self.inspector
                .state()
                .traces
                .push((self.name.clone(), area));
        }
        self.inner.render(area, surface, ctx);
    }
}

/// `(kind, summary)` for an event. The kind is what coalescing compares; the
/// summary is what the log shows.
fn describe(event: &Event) -> (String, String) {
    match event {
        Event::Key(key) => {
            let chord = key_label(Chord::from_key(*key));
            (format!("key {chord}"), chord)
        }
        Event::Mouse(mouse) => {
            let kind = match mouse.kind {
                MouseKind::Down(button) => format!("down {}", button_name(button)),
                MouseKind::Up(button) => format!("up {}", button_name(button)),
                MouseKind::Drag(button) => format!("drag {}", button_name(button)),
                MouseKind::Moved => "move".to_owned(),
                MouseKind::ScrollUp => "scroll up".to_owned(),
                MouseKind::ScrollDown => "scroll down".to_owned(),
                MouseKind::ScrollLeft => "scroll left".to_owned(),
                MouseKind::ScrollRight => "scroll right".to_owned(),
            };
            let summary = format!("{kind} {},{}", mouse.column, mouse.row);
            (format!("mouse {kind}"), summary)
        }
        // The length only: see the module docs.
        Event::Paste(text) => (
            "paste".to_owned(),
            format!("paste {} chars", text.chars().count()),
        ),
        Event::Resize { width, height } => {
            ("resize".to_owned(), format!("resize {width}×{height}"))
        }
    }
}

/// A chord as typed. [`Chord::display`] is keycap style — `g` reads `G` — which
/// suits a hint bar but misleads in a log, where `g` and `G` are different keys.
fn key_label(chord: Chord) -> String {
    match chord.code {
        KeyCode::Char(c) if c != ' ' => {
            let mut label = String::new();
            if chord.ctrl {
                label.push_str("Ctrl+");
            }
            if chord.alt {
                label.push_str("Alt+");
            }
            label.push(c);
            label
        }
        _ => chord.display(),
    }
}

/// Whether repeats of this event fold into one log line. Keys and clicks stay
/// one line each — a doubled key press is exactly what one is looking for.
fn coalesces(event: &Event) -> bool {
    match event {
        Event::Mouse(mouse) => matches!(
            mouse.kind,
            MouseKind::Moved
                | MouseKind::Drag(_)
                | MouseKind::ScrollUp
                | MouseKind::ScrollDown
                | MouseKind::ScrollLeft
                | MouseKind::ScrollRight
        ),
        Event::Resize { .. } => true,
        Event::Key(_) | Event::Paste(_) => false,
    }
}

fn button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
    }
}

fn outcome(outcome: crate::event::InputOutcome) -> &'static str {
    use crate::event::InputOutcome;
    match outcome {
        InputOutcome::Ignored => "ignored",
        InputOutcome::Consumed => "consumed",
        InputOutcome::Changed => "changed",
        InputOutcome::Submitted => "submitted",
        InputOutcome::Cancelled => "cancelled",
    }
}

fn result_name(result: UpdateResult) -> &'static str {
    match result {
        UpdateResult::Clean => "clean",
        UpdateResult::Consumed => "consumed",
        UpdateResult::Dirty => "dirty",
        UpdateResult::Exit => "exit",
    }
}

/// Where the panel goes: docked right on a screen wide enough to keep the
/// application usable beside it, docked to the bottom otherwise.
fn panel_rect(area: Rect) -> Rect {
    if area.width >= 72 {
        let width = (area.width * 2 / 5).clamp(40, 56);
        Rect::new(area.right() - width, area.y, width, area.height)
    } else {
        let height = area.height.min(12);
        Rect::new(area.x, area.bottom() - height, area.width, height)
    }
}

fn paint_outlines(surface: &mut Surface, traces: &[(String, Rect)], theme: &Theme) {
    let style = Style::new().fg(theme.accent_alt);
    let label = Style::new()
        .fg(theme.background)
        .bg(theme.accent_alt)
        .add_modifier(Modifier::BOLD);
    let glyphs = BorderStyle::Plain.glyphs();
    for (name, rect) in traces {
        let rect = rect.intersection(surface.area());
        if rect.width == 0 || rect.height == 0 {
            continue;
        }
        // Only the glyph and foreground change, so the application's own
        // background still shows which region is which.
        let edge = |surface: &mut Surface, x: u16, y: u16, ch: char| {
            let area = surface.area();
            if x >= area.x && x < area.right() && y >= area.y && y < area.bottom() {
                let cell = &mut surface.buffer_mut()[(x, y)];
                cell.set_char(ch);
                cell.set_style(style);
            }
        };
        if rect.width >= 2 && rect.height >= 2 {
            let (right, bottom) = (rect.right() - 1, rect.bottom() - 1);
            for x in rect.x..rect.right() {
                edge(surface, x, rect.y, glyphs.horizontal);
                edge(surface, x, bottom, glyphs.horizontal);
            }
            for y in rect.y..rect.bottom() {
                edge(surface, rect.x, y, glyphs.vertical);
                edge(surface, right, y, glyphs.vertical);
            }
            edge(surface, rect.x, rect.y, glyphs.top_left);
            edge(surface, right, rect.y, glyphs.top_right);
            edge(surface, rect.x, bottom, glyphs.bottom_left);
            edge(surface, right, bottom, glyphs.bottom_right);
        }
        let text = format!(" {name} {}×{} ", rect.width, rect.height);
        let x = if rect.width >= 2 { rect.x + 1 } else { rect.x };
        let mut clipped = surface.child(Rect::new(x, rect.y, rect.right() - x, 1));
        clipped.set_string(x, rect.y, &text, label);
    }
}

fn paint_panel(surface: &mut Surface, rect: Rect, state: &State, theme: &Theme) {
    let mut panel = surface.child(rect);
    let base = Style::new().fg(theme.text).bg(theme.surface);
    panel.fill(base);
    let border = Style::new().fg(theme.border_focused).bg(theme.surface);
    let inner = if rect.width >= 2 && rect.height >= 2 {
        panel.draw_border(rect, BorderStyle::Rounded.glyphs(), border);
        let title = Style::new()
            .fg(theme.accent)
            .bg(theme.surface)
            .add_modifier(Modifier::BOLD);
        let mut top = panel.child(Rect::new(rect.x + 1, rect.y, rect.width - 2, 1));
        top.set_string(rect.x + 1, rect.y, " inspector ", title);
        let hint = format!(" {} ", state.toggle.display());
        let hint_x = (rect.right() - 1).saturating_sub(str_cols(&hint));
        if hint_x > rect.x + 12 {
            top.set_string(hint_x, rect.y, &hint, border);
        }
        // Inside the border, with a column of padding each side when it fits.
        if rect.width >= 6 {
            Rect::new(rect.x + 2, rect.y + 1, rect.width - 4, rect.height - 2)
        } else {
            Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, rect.height - 2)
        }
    } else {
        rect
    };
    let lines = panel_lines(state, theme, inner);
    let mut body = panel.child(inner);
    for (row, line) in lines.iter().enumerate().take(inner.height as usize) {
        let y = inner.y + row as u16;
        let mut x = inner.x;
        for (text, style) in line {
            x = body.set_string(x, y, text, *style);
        }
    }
}

type PanelLine = Vec<(String, Style)>;

fn panel_lines(state: &State, theme: &Theme, inner: Rect) -> Vec<PanelLine> {
    let on = |style: Style| style.bg(theme.surface);
    let heading = on(Style::new().fg(theme.accent).add_modifier(Modifier::BOLD));
    let label = on(Style::new().fg(theme.muted));
    let value = on(Style::new().fg(theme.text));
    let quiet = on(Style::new().fg(theme.dim));
    let width = inner.width as usize;

    let mut lines: Vec<PanelLine> = Vec::new();
    let stats = state.stats;
    lines.push(vec![
        ("frame ".into(), label),
        (stats.frames.to_string(), value),
        ("  ".into(), label),
        (millis(stats.paint), value),
        ("  ".into(), label),
        (stats.damage.to_string(), value),
        (" cells".into(), label),
    ]);
    let spark_width = width.saturating_sub(23).clamp(4, 24);
    lines.push(vec![
        (
            sparkline(&state.history, spark_width),
            on(Style::new().fg(theme.accent)),
        ),
        (" avg ".into(), label),
        (millis(stats.average), value),
        (" max ".into(), label),
        (millis(stats.worst), value),
    ]);

    lines.push(Vec::new());
    lines.push(vec![("focus".into(), heading)]);
    match &state.focus {
        Some(focus) => {
            lines.push(vec![
                ("active ".into(), label),
                (focus.active.clone().unwrap_or_else(|| "—".into()), value),
            ]);
            if let Some(owner) = &focus.owner {
                lines.push(vec![("owner  ".into(), label), (owner.clone(), value)]);
            }
            if !focus.ring.is_empty() {
                let mut line = vec![("ring   ".into(), label)];
                for (index, id) in focus.ring.iter().enumerate() {
                    if index > 0 {
                        line.push((" · ".into(), quiet));
                    }
                    let active = focus.owner.is_none() && focus.active.as_deref() == Some(id);
                    line.push((id.clone(), if active { heading } else { value }));
                }
                lines.push(line);
            }
        }
        None => lines.push(vec![("not recorded".into(), quiet)]),
    }
    lines.push(vec![
        ("keys   ".into(), label),
        match &state.keys {
            Some(keys) => (format!("{keys} …"), value),
            None => ("—".into(), quiet),
        },
    ]);

    lines.push(Vec::new());
    lines.push(vec![("layout".into(), heading)]);
    if state.traces.is_empty() {
        lines.push(vec![("nothing traced".into(), quiet)]);
    }
    // Leave the events at least a third of the panel.
    let layout_budget = (inner.height as usize).saturating_sub(lines.len()) * 2 / 3;
    let shown = state.traces.len().min(layout_budget.max(1));
    for (name, rect) in state.traces.iter().take(shown) {
        lines.push(vec![
            (format!("{name} "), value),
            (
                format!("{},{} {}×{}", rect.x, rect.y, rect.width, rect.height),
                label,
            ),
        ]);
    }
    if state.traces.len() > shown {
        lines.push(vec![(
            format!("+{} more", state.traces.len() - shown),
            quiet,
        )]);
    }

    lines.push(Vec::new());
    lines.push(vec![("events".into(), heading)]);
    let room = (inner.height as usize).saturating_sub(lines.len());
    if state.events.is_empty() {
        lines.push(vec![("none yet".into(), quiet)]);
    }
    // Newest last, so the log reads top to bottom like a terminal: walk back
    // from the newest record until the room runs out, keeping each record's
    // lines together.
    let mut tail: Vec<Vec<PanelLine>> = Vec::new();
    let mut used = 0;
    for record in state.events.iter().rev() {
        let mut summary = record.summary.clone();
        if record.repeats > 1 {
            let _ = write!(summary, " ×{}", record.repeats);
        }
        let mut block = vec![vec![
            (format!("{:>3} ", record.seq % 1000), quiet),
            (summary, value),
            (format!(" {}", result_name(record.result)), quiet),
        ]];
        // The route gets its own line: it is the longest part, and the part
        // worth reading in full.
        if let Some(route) = &record.route {
            block.push(vec![("    → ".into(), quiet), (route.clone(), label)]);
        }
        if used + block.len() > room {
            break;
        }
        used += block.len();
        tail.push(block);
    }
    lines.extend(tail.into_iter().rev().flatten());
    lines
}

fn millis(duration: Duration) -> String {
    format!("{:.2}ms", duration.as_secs_f64() * 1000.0)
}

fn sparkline(history: &VecDeque<Duration>, width: usize) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let recent: Vec<Duration> = history
        .iter()
        .skip(history.len().saturating_sub(width))
        .copied()
        .collect();
    // Scaled between the window's own floor and peak: frame times cluster, and
    // bars measured from zero would all read full.
    let floor = recent.iter().copied().min().unwrap_or_default();
    let span = recent.iter().copied().max().unwrap_or_default() - floor;
    let mut out = String::with_capacity(width * 3);
    let pad = width - recent.len();
    for sample in recent {
        let level = if span.is_zero() {
            0
        } else {
            ((sample - floor).as_nanos() * 7 / span.as_nanos()) as usize
        };
        out.push(BARS[level.min(7)]);
    }
    // Padded after the bars, so the summary beside it stays put while the
    // history fills.
    out.extend(std::iter::repeat_n(' ', pad));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Flex, Text};
    use crate::event::{InputOutcome, Key, KeyCode, Mouse};
    use crate::testing::{grid, render};

    fn alt_i() -> Event {
        Event::Key(Key {
            code: KeyCode::Char('i'),
            ctrl: false,
            alt: true,
            shift: false,
        })
    }

    fn mouse(kind: MouseKind, column: u16, row: u16) -> Event {
        Event::Mouse(Mouse {
            kind,
            column,
            row,
            shift: false,
            ctrl: false,
            alt: false,
            super_key: false,
        })
    }

    fn painted(inspector: &Inspector, width: u16, height: u16) -> String {
        let theme = Theme::default();
        let mut buffer = render(&Text::raw(""), width, height, &theme);
        let area = buffer.area;
        inspector.paint(&mut buffer, area, &theme);
        grid(&buffer)
    }

    #[test]
    fn the_toggle_chord_cycles_the_mode_and_nothing_else_does() {
        let inspector = Inspector::new();
        assert_eq!(inspector.mode(), InspectorMode::Hidden);
        assert!(!inspector.handle(&Event::Key(Key::new(KeyCode::Char('i')))));
        assert!(inspector.handle(&alt_i()));
        assert_eq!(inspector.mode(), InspectorMode::Panel);
        assert!(inspector.handle(&alt_i()));
        assert_eq!(inspector.mode(), InspectorMode::Outlines);
        assert!(inspector.handle(&alt_i()));
        assert_eq!(inspector.mode(), InspectorMode::Hidden);

        let custom = Inspector::new().toggle_key(Chord::parse("ctrl+t").unwrap());
        assert!(!custom.handle(&alt_i()));
    }

    #[test]
    fn a_hidden_inspector_paints_nothing() {
        let inspector = Inspector::new();
        inspector.record_frame(Duration::from_millis(1), 10);
        assert_eq!(painted(&inspector, 20, 3).trim(), "");
    }

    #[test]
    fn the_panel_shows_frame_focus_keys_layout_and_events() {
        let inspector = Inspector::new().with_mode(InspectorMode::Panel);
        inspector.record_frame(Duration::from_micros(1500), 37);

        let mut focus = FocusRegistry::new();
        focus.begin_frame();
        focus.register("composer");
        focus.register("list");
        inspector.record_focus(&focus);

        let mut keymap = Keymap::new().layer(crate::keymap::Layer::new("g").bind("g g", ()));
        keymap.dispatch(Key::new(KeyCode::Char('g')));
        inspector.record_keymap(&keymap);

        let root = Flex::column()
            .fixed(1, inspector.trace("header", Text::raw("title")))
            .grow(1, inspector.trace("body", Text::raw("text")));
        let _ = render(&root, 30, 6, &Theme::default());

        let event = Event::Key(Key {
            code: KeyCode::Char('r'),
            ctrl: true,
            alt: false,
            shift: false,
        });
        let mut router = crate::routing::Router::new(&focus, &event);
        router.target_fn("composer", |_| InputOutcome::Changed);
        inspector.record_route(&router.finish());
        inspector.log_event(&event, UpdateResult::Dirty);

        let screen = painted(&inspector, 80, 24);
        for expected in [
            "inspector",
            "Alt+I",
            "frame 1  1.50ms  37 cells",
            "active composer",
            "ring   composer · list",
            "keys   g …",
            "header 0,0 30×1",
            "body 0,1 30×5",
            "Ctrl+r dirty",
            "→ target composer changed",
        ] {
            assert!(
                screen.contains(expected),
                "missing {expected:?} in\n{screen}"
            );
        }
    }

    #[test]
    fn overlay_ownership_is_shown_and_the_ring_loses_its_highlight() {
        let inspector = Inspector::new().with_mode(InspectorMode::Panel);
        let mut focus = FocusRegistry::new();
        focus.begin_frame();
        focus.register("composer");
        focus.set_owner("dialog");
        inspector.record_focus(&focus);
        let screen = painted(&inspector, 80, 24);
        assert!(screen.contains("active dialog"), "{screen}");
        assert!(screen.contains("owner  dialog"), "{screen}");
    }

    #[test]
    fn pointer_bursts_coalesce_but_keys_do_not() {
        let inspector = Inspector::new().with_mode(InspectorMode::Panel);
        for column in 0..5 {
            inspector.log_event(&mouse(MouseKind::Moved, column, 2), UpdateResult::Clean);
        }
        let key = Event::Key(Key::new(KeyCode::Char('j')));
        inspector.log_event(&key, UpdateResult::Dirty);
        inspector.log_event(&key, UpdateResult::Dirty);
        let screen = painted(&inspector, 80, 24);
        assert!(screen.contains("move 4,2 ×5 clean"), "{screen}");
        assert_eq!(screen.matches("j dirty").count(), 2, "{screen}");
    }

    #[test]
    fn a_paste_is_logged_by_length_never_by_content() {
        let inspector = Inspector::new().with_mode(InspectorMode::Panel);
        inspector.log_event(&Event::Paste("hunter2".into()), UpdateResult::Dirty);
        let screen = painted(&inspector, 80, 24);
        assert!(screen.contains("paste 7 chars"), "{screen}");
        assert!(!screen.contains("hunter2"), "{screen}");
    }

    #[test]
    fn the_event_log_is_bounded() {
        let inspector = Inspector::new().event_capacity(3);
        for _ in 0..10 {
            inspector.log_event(&Event::Key(Key::new(KeyCode::Enter)), UpdateResult::Clean);
        }
        assert_eq!(inspector.state().events.len(), 3);
        assert_eq!(inspector.state().events.back().unwrap().seq, 10);
    }

    #[test]
    fn frame_stats_summarise_the_recent_history() {
        let inspector = Inspector::new();
        inspector.record_frame(Duration::from_millis(1), 5);
        inspector.record_frame(Duration::from_millis(3), 9);
        let stats = inspector.last_frame();
        assert_eq!(stats.frames, 2);
        assert_eq!(stats.damage, 9);
        assert_eq!(stats.average, Duration::from_millis(2));
        assert_eq!(stats.worst, Duration::from_millis(3));
    }

    #[test]
    fn the_sparkline_spans_the_window_and_pads_after_it() {
        let history: VecDeque<Duration> = [10, 11, 12, 18]
            .into_iter()
            .map(Duration::from_millis)
            .collect();
        assert_eq!(sparkline(&history, 6), "▁▁▂█  ");
        let flat: VecDeque<Duration> = [Duration::from_millis(3); 3].into();
        assert_eq!(sparkline(&flat, 3), "▁▁▁");
        assert_eq!(sparkline(&VecDeque::new(), 2), "  ");
    }

    #[test]
    fn begin_frame_forgets_last_frames_traces() {
        let inspector = Inspector::new();
        let _ = render(
            &inspector.trace("a", Text::raw("x")),
            4,
            1,
            &Theme::default(),
        );
        assert_eq!(
            inspector.traces(),
            vec![("a".to_owned(), Rect::new(0, 0, 4, 1))]
        );
        inspector.begin_frame();
        assert!(inspector.traces().is_empty());
    }

    #[test]
    fn a_layout_only_pass_records_no_trace() {
        let inspector = Inspector::new();
        let theme = Theme::default();
        let root = inspector.trace("a", Text::raw("x"));
        crate::probe::resolve_layout(
            Rect::new(0, 0, 4, 1),
            &crate::RenderCtx::new(&theme),
            &root,
            &[],
        );
        assert!(inspector.traces().is_empty());
        let _ = render(&root, 4, 1, &theme);
        assert_eq!(inspector.traces().len(), 1);
    }

    #[test]
    fn outlines_frame_each_traced_view_with_its_label() {
        let inspector = Inspector::new().with_mode(InspectorMode::Outlines);
        let theme = Theme::default();
        let root = inspector.trace("pane", Text::raw(""));
        let mut buffer = render(&root, 80, 24, &theme);
        // Trace a small rect on the left so the right-docked panel leaves it visible.
        inspector.begin_frame();
        inspector
            .state()
            .traces
            .push(("pane".into(), Rect::new(0, 0, 20, 4)));
        let area = buffer.area;
        inspector.paint(&mut buffer, area, &theme);
        let screen = grid(&buffer);
        let rows: Vec<&str> = screen.lines().collect();
        assert!(rows[0].starts_with("┌ pane 20×4 ───────┐"), "{screen}");
        assert!(rows[3].starts_with("└──────────────────┘"), "{screen}");
        assert_eq!(buffer[(0, 1)].style().fg, Some(theme.accent_alt));
    }

    #[test]
    fn the_panel_docks_right_when_wide_and_bottom_when_narrow() {
        let wide = panel_rect(Rect::new(0, 0, 100, 30));
        assert_eq!((wide.x + wide.width, wide.height), (100, 30));
        let narrow = panel_rect(Rect::new(0, 0, 40, 30));
        assert_eq!((narrow.width, narrow.y + narrow.height), (40, 30));
    }

    #[test]
    fn the_panel_uses_the_theme_surface_and_border_slots() {
        let inspector = Inspector::new().with_mode(InspectorMode::Panel);
        let theme = Theme::default();
        let mut buffer = render(&Text::raw(""), 80, 24, &theme);
        let area = buffer.area;
        inspector.paint(&mut buffer, area, &theme);
        let panel = panel_rect(buffer.area);
        let corner = &buffer[(panel.x, panel.y)];
        assert_eq!(corner.symbol(), "╭");
        assert_eq!(corner.style().fg, Some(theme.border_focused));
        assert_eq!(
            buffer[(panel.x + 3, panel.y + 5)].style().bg,
            Some(theme.surface)
        );
    }

    #[test]
    fn every_size_paints_inside_its_area_without_panicking() {
        let inspector = Inspector::new().with_mode(InspectorMode::Outlines);
        inspector.record_frame(Duration::from_millis(2), 1);
        inspector.log_event(&alt_i(), UpdateResult::Dirty);
        let theme = Theme::default();
        for width in 0..=90u16 {
            for height in [0u16, 1, 2, 3, 5, 13, 30] {
                let root = inspector.trace("全幅 wide name", Text::raw("x"));
                inspector.begin_frame();
                let mut buffer = Buffer::empty(Rect::new(0, 0, width, height));
                let full = buffer.area;
                crate::paint(&mut buffer, full, &theme, &root, &[]);
                let area = Rect::new(0, 0, width / 2 + 1, height);
                let before = buffer.clone();
                inspector.paint(&mut buffer, area, &theme);
                for y in 0..height {
                    for x in area.right().min(width)..width {
                        assert_eq!(
                            buffer[(x, y)],
                            before[(x, y)],
                            "{width}×{height} at {x},{y}"
                        );
                    }
                }
            }
        }
    }
}
