//! Application state and input routing.
//!
//! This is the "host" half of the example: everything tuika deliberately leaves
//! to the application — what the transcript contains, which surface owns the
//! keyboard right now, when the view sticks to the bottom, and what a slash
//! command does. tuika supplies the state types (`TextInputState`,
//! `ScrollState`, `SelectState`) and renders from them; the wiring below is the
//! part a real host writes.

use tuika::ui::Rect;
use tuika::ui::{Color, Modifier, Style};

use tuika::prelude::*;

use crate::agent::{Agent, Decision, interrupted_notice};
use crate::history::{Cell, Tone};

/// Whether the event loop should keep running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

/// What a global chord asked for, applied once the route is finished.
enum Interrupt {
    Requested,
    Quit,
}

/// The slash commands the composer completes, in the order Codex lists them.
pub const COMMANDS: &[(&str, &str)] = &[
    (
        "/init",
        "create an AGENTS.md file with instructions for Codex",
    ),
    ("/status", "show current session configuration"),
    ("/approvals", "choose what Codex can do without approval"),
    ("/model", "choose what model and reasoning effort to use"),
    ("/new", "start a new chat during a conversation"),
    (
        "/compact",
        "summarize conversation to prevent hitting the context limit",
    ),
    ("/diff", "show git diff (including untracked files)"),
    ("/mention", "mention a file"),
    ("/quit", "exit Codex"),
];

const MODELS: &[(&str, &str)] = &[
    ("gpt-5-codex  medium", "default — balances speed and depth"),
    ("gpt-5-codex  high", "slower, more thorough reasoning"),
    ("gpt-5-codex  low", "fastest, for small mechanical edits"),
    ("gpt-5  medium", "the general model, not the coding tune"),
];

/// Files the `@` picker offers. A real host would search the workspace.
const FILES: &[(&str, &str)] = &[
    ("@src/lib.rs", "crate root"),
    ("@src/markdown.rs", "streaming CommonMark renderer"),
    ("@src/components/scroll.rs", "line viewport"),
    ("@AGENTS.md", "repository guidance"),
    ("@README.md", "public entry point"),
];

const APPROVAL_MODES: &[(&str, &str)] = &[
    ("Read Only", "Codex can read files and answer questions"),
    ("Auto", "reads, edits, and runs commands in the workspace"),
    (
        "Full Access",
        "network access and writes outside the workspace",
    ),
];

/// What the composer's trigger characters mean **here**.
///
/// tuika finds and delimits the tokens (see [`Trigger`]); everything below —
/// which character opens what, what the popup lists, what a completion inserts —
/// is this application's. Swap the table and `/` could summon an emoji picker.
pub fn triggers() -> [Trigger; 2] {
    [
        // Codex only treats `/` as a command when it opens the whole message.
        Trigger::new('/').anchor(TriggerAnchor::BufferStart),
        // A file mention can start any word, and completes to a path.
        Trigger::new('@'),
    ]
}

/// Which picker (if any) is stealing the composer's keys.
pub enum Popup {
    /// Completion for the token under the cursor: `/command` or `@file`,
    /// distinguished by the trigger the host declared, not by parsing the text.
    Token { trigger: char, state: SelectState },
    /// `/model`.
    Model(SelectState),
    /// `/approvals`.
    Approvals(SelectState),
}

/// A command the agent wants to run, waiting on the user.
pub struct Approval {
    pub command: String,
    pub state: SelectState,
}

/// The banner values and the meters the composer footer shows.
pub struct Session {
    pub version: String,
    pub model: String,
    pub cwd: String,
    pub approval: String,
    pub sandbox: String,
    pub tokens: u32,
    pub context_window: u32,
}

impl Session {
    fn new() -> Self {
        Self {
            version: "0.45.0".into(),
            model: "gpt-5-codex   medium reasoning".into(),
            cwd: "~/code/tuika".into(),
            approval: "on-request".into(),
            sandbox: "workspace-write".into(),
            tokens: 0,
            context_window: 272_000,
        }
    }

    /// Percentage of the context window still free, as Codex reports it.
    pub fn context_left(&self) -> u16 {
        let used = (self.tokens as f32 / self.context_window as f32 * 100.0).round() as u16;
        100u16.saturating_sub(used)
    }

    fn rows(&self) -> Vec<(String, String)> {
        vec![
            ("model".into(), self.model.clone()),
            ("directory".into(), self.cwd.clone()),
            ("approval".into(), self.approval.clone()),
            ("sandbox".into(), self.sandbox.clone()),
        ]
    }
}

/// The whole application.
pub struct App {
    pub frame: u64,
    pub cells: Vec<Cell>,
    pub composer: TextInputState,
    pub scroll: ScrollState,
    /// Which surface owns input this frame; the router reads it.
    focus: FocusRegistry,
    pub agent: Agent,
    pub popup: Option<Popup>,
    pub approval: Option<Approval>,
    pub session: Session,
    /// Frame the current turn started on, for the `Working (12s …)` timer.
    pub turn_started: Option<u64>,
    /// Previously submitted prompts, newest last (recalled with `Up`).
    history: Vec<String>,
    history_cursor: Option<usize>,
    /// Transcript geometry from the last frame, so paging keys have dimensions
    /// to work against before the next render.
    pub content_h: usize,
    pub viewport_h: usize,
    /// Set by `/quit`, which can be reached from the popup as well as the
    /// composer, so the flag is read once on the way out of `handle`.
    quit_requested: bool,
}

impl App {
    pub fn new() -> Self {
        let session = Session::new();
        let banner = Cell::Banner {
            version: session.version.clone(),
            rows: session.rows(),
            tips: COMMANDS
                .iter()
                .take(4)
                .map(|(c, b)| ((*c).to_string(), (*b).to_string()))
                .collect(),
        };
        Self {
            frame: 0,
            cells: vec![banner],
            composer: TextInputState::new(),
            scroll: ScrollState::new(),
            focus: FocusRegistry::new(),
            agent: Agent::new(),
            popup: None,
            approval: None,
            session,
            turn_started: None,
            history: Vec::new(),
            history_cursor: None,
            content_h: 0,
            viewport_h: 0,
            quit_requested: false,
        }
    }

    /// Advance animation and the scripted turn by one frame.
    pub fn tick(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.agent.tick(&mut self.cells);
        self.session.tokens = self.agent.tokens;
        if !self.agent.is_running() {
            self.turn_started = None;
        }
        // A turn that hits a command needing approval raises the prompt here,
        // rather than from inside the agent, so the UI owns its own state.
        if let Some(command) = self.agent.pending_approval()
            && self.approval.is_none()
        {
            self.approval = Some(Approval {
                command: command.to_string(),
                state: SelectState::new(),
            });
        }
    }

    /// Seconds the current turn has been running, at the example's frame rate.
    pub fn elapsed_secs(&self) -> u64 {
        let started = self.turn_started.unwrap_or(self.frame);
        (self.frame.saturating_sub(started)) * crate::FRAME_MS / 1000
    }

    /// Where the terminal cursor belongs, given the composer's painted rect.
    pub fn cursor(&self, composer_rect: Rect) -> Option<(u16, u16)> {
        if self.approval.is_some() || composer_rect.width == 0 {
            return None;
        }
        Some(self.composer.cursor_screen(composer_rect))
    }

    /// The rows the current popup offers: `(label, blurb)`.
    ///
    /// The *source* is chosen by the trigger that opened the token, so adding a
    /// third trigger is one arm here and one entry in [`triggers`].
    pub fn popup_items(&self) -> Vec<(String, String)> {
        let (pairs, filter): (&[(&str, &str)], String) = match &self.popup {
            Some(Popup::Token { trigger, .. }) => {
                let token = self.composer.active_token(&triggers());
                let filter = token.map(|t| t.text).unwrap_or_default();
                match trigger {
                    '/' => (COMMANDS, filter),
                    _ => (FILES, filter),
                }
            }
            Some(Popup::Model(_)) => (MODELS, String::new()),
            Some(Popup::Approvals(_)) => (APPROVAL_MODES, String::new()),
            None => return Vec::new(),
        };
        pairs
            .iter()
            .filter(|(label, _)| label.starts_with(filter.trim()))
            .map(|(label, blurb)| ((*label).to_string(), (*blurb).to_string()))
            .collect()
    }

    /// Route one translated event.
    ///
    /// The order is declared once, for **every** event kind, by [`Router`]: the
    /// interrupt chords, then the two surfaces that receive regardless of focus
    /// (the transcript scrolls, the picker claims its own keys), then whichever
    /// surface owns input this frame.
    pub fn handle(&mut self, event: &Event) -> Flow {
        let flow = self.route(event);
        if self.quit_requested {
            return Flow::Quit;
        }
        flow
    }

    /// Declare this frame's surfaces and which one owns input.
    ///
    /// A host whose modal is a real overlay gets this from
    /// [`Scene::sync_focus`]; these modes are inline rows, so the app states
    /// them itself. Either way it is one declaration, not a rule re-derived per
    /// event kind.
    fn sync_focus(&mut self) {
        self.focus.begin_frame();
        self.focus.register("composer");
        // Only the approval prompt is *exclusive*. The picker takes precedence
        // over the composer without owning it, so it routes as a declared
        // exception below rather than as this frame's owner.
        match self.approval.is_some() {
            true => self.focus.set_owner("approval"),
            false => self.focus.clear_owner(),
        }
    }

    fn route(&mut self, event: &Event) -> Flow {
        self.sync_focus();

        let mut interrupt = None;
        let mut router = Router::new(&self.focus, event);
        // Interrupt and quit outrank whatever owns input.
        router.pre_fn(|event| {
            let Event::Key(key) = event else {
                return InputOutcome::Ignored;
            };
            if key.ctrl && key.code == KeyCode::Char('c') {
                interrupt = Some(Interrupt::Requested);
                return InputOutcome::Cancelled;
            }
            if key.ctrl && key.code == KeyCode::Char('d') && self.composer.is_empty() {
                interrupt = Some(Interrupt::Quit);
                return InputOutcome::Cancelled;
            }
            InputOutcome::Ignored
        });
        // The transcript pages and scrolls whether or not it holds focus, and
        // the picker claims its navigation keys before the composer types them.
        router.always_fn("transcript", |event| {
            self.scroll.handle(event, self.content_h, self.viewport_h)
        });
        router.always_fn("popup", |event| self.handle_popup(event));
        router.target_fn("approval", |event| self.handle_approval(event));
        router.target_fn("composer", |event| self.handle_composer(event));
        router.finish();

        // Stages decide; the host applies afterwards, so a stage closure never
        // needs a borrow of the whole application while the route is running.
        match interrupt {
            Some(Interrupt::Requested) => self.interrupt_or_quit(),
            Some(Interrupt::Quit) => Flow::Quit,
            None => Flow::Continue,
        }
    }

    /// The composer's own keys: interrupt, prompt recall, then editing.
    fn handle_composer(&mut self, event: &Event) -> InputOutcome {
        if let Event::Key(key) = event
            && key.plain()
        {
            if key.code == KeyCode::Esc {
                if self.agent.interrupt() {
                    self.cells.push(interrupted_notice());
                    self.follow();
                }
                return InputOutcome::Cancelled;
            }
            // `Up` on an empty composer recalls the previous prompt, as in Codex.
            if key.code == KeyCode::Up && self.composer.is_empty() {
                self.recall();
                return InputOutcome::Changed;
            }
        }

        let outcome = self.composer.handle(event);
        match outcome {
            InputOutcome::Submitted => {
                let text = self.composer.text().trim().to_string();
                self.composer.clear();
                self.popup = None;
                self.history_cursor = None;
                if !text.is_empty() {
                    self.submit(&text);
                }
            }
            _ => self.sync_popup(),
        }
        outcome
    }

    fn interrupt_or_quit(&mut self) -> Flow {
        if self.approval.take().is_some() {
            self.agent.approve(Decision::Deny, &mut self.cells);
            return Flow::Continue;
        }
        if self.agent.interrupt() {
            self.cells.push(interrupted_notice());
            self.follow();
            return Flow::Continue;
        }
        Flow::Quit
    }

    fn handle_approval(&mut self, event: &Event) -> InputOutcome {
        let Some(approval) = &mut self.approval else {
            return InputOutcome::Ignored;
        };
        // Codex accepts the digit shortcuts as well as the caret.
        let digit = match event {
            Event::Key(key) if key.plain() => match key.code {
                KeyCode::Char('1') => Some(0),
                KeyCode::Char('2') => Some(1),
                KeyCode::Char('3') => Some(2),
                _ => None,
            },
            _ => None,
        };
        let picked = match digit {
            Some(index) => Some(index),
            None => match approval.state.handle(event, 3) {
                InputOutcome::Submitted => approval.state.selected(),
                InputOutcome::Cancelled => Some(2),
                // A modal answers for everything it is shown for: nothing
                // behind it may act on the same event.
                _ => None,
            },
        };
        let Some(index) = picked else {
            return InputOutcome::Consumed;
        };
        let decision = match index {
            0 => Decision::Once,
            1 => Decision::Session,
            _ => Decision::Deny,
        };
        self.approval = None;
        self.agent.approve(decision, &mut self.cells);
        self.follow();
        InputOutcome::Submitted
    }

    /// The picker's keys. Anything it does not recognize keeps flowing, so
    /// typing continues to filter the list in the composer behind it.
    fn handle_popup(&mut self, event: &Event) -> InputOutcome {
        // A declared exception still yields to an exclusive owner.
        if self.approval.is_some() {
            return InputOutcome::Ignored;
        }
        let items = self.popup_items();
        let Some(popup) = self.popup.as_mut() else {
            return InputOutcome::Ignored;
        };
        let is_token = matches!(popup, Popup::Token { .. });
        let state = match popup {
            Popup::Token { state, .. } | Popup::Model(state) | Popup::Approvals(state) => state,
        };
        // Tab completes the highlighted row in place, without running it.
        if let Event::Key(key) = event
            && key.plain()
            && key.code == KeyCode::Tab
        {
            let completion = state
                .selected()
                .and_then(|selected| items.get(selected))
                .map(|(label, _)| label.clone());
            if is_token && let Some(label) = completion {
                self.complete(&label);
            }
            return InputOutcome::Changed;
        }
        match state.handle(event, items.len()) {
            InputOutcome::Submitted => {
                let label = state
                    .selected()
                    .and_then(|index| items.get(index))
                    .map(|(l, _)| l.clone());
                if let (Some(label), Some(kind)) = (label, self.popup.take()) {
                    self.confirm_popup(kind, &label);
                }
                InputOutcome::Submitted
            }
            InputOutcome::Cancelled => {
                self.popup = None;
                InputOutcome::Cancelled
            }
            outcome => outcome,
        }
    }

    fn confirm_popup(&mut self, popup: Popup, label: &str) {
        match popup {
            // A command runs on confirm; a mention completes into the text and
            // leaves the user typing. Same popup machinery, different verbs —
            // which is the point: the trigger decides, not the widget.
            Popup::Token { trigger: '/', .. } => {
                self.composer.clear();
                self.submit(label);
            }
            Popup::Token { .. } => self.complete(&format!("{label} ")),
            Popup::Model(_) => {
                self.session.model = label.to_string();
                self.cells.push(Cell::Notice {
                    tone: Tone::Info,
                    title: format!("Model set to {label}"),
                    body: Vec::new(),
                });
                self.follow();
            }
            Popup::Approvals(_) => {
                self.session.approval = label.to_ascii_lowercase().replace(' ', "-");
                self.cells.push(Cell::Notice {
                    tone: Tone::Info,
                    title: format!("Approval mode set to {label}"),
                    body: Vec::new(),
                });
                self.follow();
            }
        }
    }

    /// Open, refilter, or close the completion popup after the composer changed.
    ///
    /// The whole rule is "is the cursor inside a token?" — tuika answers that
    /// from the declared triggers, so this host never scans the text itself.
    pub fn sync_popup(&mut self) {
        let active = self.composer.active_token(&triggers());
        match (&self.popup, active) {
            (Some(Popup::Model(_)) | Some(Popup::Approvals(_)), _) => {}
            (_, None) => {
                if matches!(self.popup, Some(Popup::Token { .. })) {
                    self.popup = None;
                }
            }
            (Some(Popup::Token { trigger, state }), Some(token)) if *trigger == token.trigger => {
                // The filter shrank the list under the caret; pull it back in.
                let mut state = *state;
                let len = self.popup_items().len();
                state.clamp(len);
                self.popup = Some(Popup::Token {
                    trigger: token.trigger,
                    state,
                });
            }
            (_, Some(token)) => {
                self.popup = Some(Popup::Token {
                    trigger: token.trigger,
                    state: SelectState::new(),
                });
            }
        }
    }

    /// Replace the token under the cursor with `replacement`.
    fn complete(&mut self, replacement: &str) {
        let Some(token) = self.composer.active_token(&triggers()) else {
            return;
        };
        self.composer.replace_token(&token, replacement);
        self.sync_popup();
    }

    /// The composer's tokens as styled ranges — mentions and commands colored
    /// in the input itself, the way Codex marks them.
    pub fn composer_highlights(&self, theme: &Theme) -> Vec<tuika::components::TextSpan> {
        self.composer
            .tokens(&triggers())
            .iter()
            .map(|token: &Token| {
                let style = match token.trigger {
                    '/' => Style::default()
                        .fg(theme.accent_alt)
                        .add_modifier(Modifier::BOLD),
                    _ => Style::default().fg(theme.accent),
                };
                token.span(style)
            })
            .collect()
    }

    fn recall(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next = match self.history_cursor {
            None => self.history.len() - 1,
            Some(i) => i.saturating_sub(1),
        };
        self.history_cursor = Some(next);
        let text = self.history[next].clone();
        self.composer.set_text(&text);
    }

    /// Accept a submitted prompt: a slash command, or a turn for the agent.
    pub fn submit(&mut self, text: &str) {
        self.history.push(text.to_string());
        if let Some(command) = text.strip_prefix('/') {
            self.slash(command.split_whitespace().next().unwrap_or(""));
            self.follow();
            return;
        }
        self.cells.push(Cell::User(text.to_string()));
        self.agent.start(text);
        self.turn_started = Some(self.frame);
        self.follow();
    }

    fn slash(&mut self, command: &str) {
        match command {
            "init" => {
                self.cells.push(Cell::User("/init".into()));
                self.agent.start("/init");
                self.turn_started = Some(self.frame);
            }
            "status" => self.cells.push(Cell::Config {
                title: "Session".into(),
                rows: {
                    let mut rows = self.session.rows();
                    rows.push((
                        "tokens".into(),
                        format!(
                            "{} used · {}% context left",
                            self.session.tokens,
                            self.session.context_left()
                        ),
                    ));
                    rows
                },
            }),
            "model" => self.popup = Some(Popup::Model(SelectState::new())),
            "approvals" => self.popup = Some(Popup::Approvals(SelectState::new())),
            "new" => {
                let banner = Cell::Banner {
                    version: self.session.version.clone(),
                    rows: self.session.rows(),
                    tips: COMMANDS
                        .iter()
                        .take(4)
                        .map(|(c, b)| ((*c).to_string(), (*b).to_string()))
                        .collect(),
                };
                self.cells = vec![banner];
                self.agent = Agent::new();
                self.session.tokens = 0;
            }
            "compact" => self.cells.push(Cell::Notice {
                tone: Tone::Info,
                title: "Compacted context".into(),
                body: vec![format!(
                    "summarized the conversation so far — {}% context left",
                    self.session.context_left()
                )],
            }),
            "diff" => self.cells.push(Cell::Patch {
                header: "Working tree diff (1 file, +2 -1)".into(),
                hunk: vec![
                    "diff --git a/src/snapshots/composer.txt b/src/snapshots/composer.txt".into(),
                    "@@ -12,4 +12,5 @@".into(),
                    " ╰──────────────────────────────╯".into(),
                    "-".into(),
                    "+  ⏎ send   ⇧⏎ newline   ⌃C quit".into(),
                    "+".into(),
                ],
            }),
            "quit" | "exit" => self.quit_requested = true,
            "mention" => {
                self.composer.set_text("@");
                self.sync_popup();
            }
            other => self.cells.push(Cell::Notice {
                tone: Tone::Error,
                title: format!("Unknown command: /{other}"),
                body: vec!["press / to see the available commands".into()],
            }),
        }
    }

    /// Re-arm the stick-to-bottom follow after appending to the transcript.
    fn follow(&mut self) {
        self.scroll.jump_to_bottom(self.content_h, self.viewport_h);
    }

    /// Take the transcript entries that will never change again, removing them
    /// from the app.
    ///
    /// Used by the split-footer mode, where finished entries are handed to the
    /// terminal's scrollback instead of being redrawn every frame. The agent
    /// only ever appends a cell or streams into the *last* one, so everything
    /// before it is settled; when no turn is in flight, the last one is too.
    /// Ownership moves out with the cell, which is the point: once an entry is
    /// the terminal's, this app cannot repaint it.
    pub fn drain_settled(&mut self) -> Vec<Cell> {
        let in_flight = usize::from(self.agent.is_running());
        let settled = self.cells.len().saturating_sub(in_flight);
        self.cells.drain(..settled).collect()
    }

    /// Rebuild the transcript as one item per cell, laid out to `width`.
    ///
    /// Rebuilding every frame is the model working as intended: only the
    /// streaming answer holds a cache, and `ratatui` diffs the resulting cells.
    pub fn transcript(&mut self, width: u16, theme: &Theme, sheet: &StyleSheet) -> Vec<Element> {
        self.cells
            .iter_mut()
            .map(|cell| cell.view(width, theme, sheet))
            .collect()
    }
}

/// A palette close to what Codex draws on a dark terminal: near-black behind
/// everything, cyan for its own marks, and a muted gray for machine output.
pub fn codex_theme() -> Theme {
    Theme {
        background: Color::Rgb(13, 14, 16),
        surface: Color::Rgb(23, 25, 28),
        text: Color::Rgb(223, 226, 230),
        muted: Color::Rgb(142, 148, 158),
        dim: Color::Rgb(88, 94, 104),
        accent: Color::Rgb(94, 187, 209),
        accent_alt: Color::Rgb(197, 154, 231),
        border: Color::Rgb(60, 66, 74),
        border_focused: Color::Rgb(94, 187, 209),
        selection_bg: Color::Rgb(35, 44, 52),
        selection_fg: Color::Rgb(235, 238, 242),
        code: CodeTheme {
            link: Color::Rgb(122, 172, 240),
            string: Color::Rgb(140, 200, 140),
            heading: Color::Rgb(223, 226, 230),
            ..CodeTheme::default()
        },
    }
}
