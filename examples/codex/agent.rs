//! The fake agent: a scripted turn, revealed frame by frame.
//!
//! Nothing here talks to a model. A turn is a queue of [`Step`]s picked from the
//! prompt's keywords, and every tick reveals a little more of the head step —
//! characters for streamed text, one row at a time for command output. That is
//! enough to exercise the parts of the UI that are hard to get right: text that
//! arrives mid-frame, a transcript that grows while the user scrolls, and a turn
//! that blocks on an approval the user has not answered yet.

use std::collections::VecDeque;

use tuika::MarkdownState;

use crate::history::{Cell, ExecStatus, Tone};

/// Characters of streamed text revealed per frame.
const STREAM_CHARS: usize = 7;
/// Frames between two rows of command output.
const OUTPUT_DELAY: u32 = 2;
/// Frames of latency before a turn's first output appears.
const START_DELAY: u32 = 5;
/// Frames between two steps of a turn.
const STEP_DELAY: u32 = 4;
/// Rough token costs, so the footer's context meter moves the way a real
/// session's does: reading a file is expensive, generated text is not.
const TOKENS_PER_CHAR: f32 = 0.4;
const TOKENS_PER_TURN: u32 = 1_400;
const TOKENS_PER_FILE: u32 = 900;
const TOKENS_PER_OUTPUT_ROW: u32 = 45;
const TOKENS_PER_PATCH: u32 = 600;

/// One scripted action within a turn.
pub enum Step {
    /// A reasoning summary, streamed in.
    Think(&'static str),
    /// Read-only exploration; one entry appears at a time.
    Explore(&'static [&'static str]),
    /// A shell command whose output is revealed row by row.
    Exec {
        command: &'static str,
        output: &'static [&'static str],
        /// Whether the sandbox policy makes this command need approval.
        approval: bool,
        /// Exit code reported once the output has all been shown.
        exit: i32,
    },
    /// A file edit with a diff preview.
    Edit {
        path: &'static str,
        added: u32,
        removed: u32,
        hunk: &'static [&'static str],
    },
    /// The final answer, streamed as markdown.
    Say(&'static str),
}

/// What the user chose in the approval prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Run it once.
    Once,
    /// Run it, and stop asking for this command for the rest of the session.
    Session,
    /// Refuse, and let the model pick something else.
    Deny,
}

/// Where the agent is in the current turn.
enum State {
    Idle,
    Running,
    /// The head `Exec` step needs approval for this command first.
    Blocked(String),
}

/// The scripted turn runner.
pub struct Agent {
    queue: VecDeque<Step>,
    state: State,
    /// Frames still to wait before the head step advances again.
    delay: u32,
    /// How much of the head step has been revealed: characters of streamed
    /// text, or rows of output/exploration.
    progress: usize,
    /// Approval already granted for the head step.
    approved: bool,
    /// Command prefixes the user approved for the whole session.
    trusted: Vec<String>,
    /// Tokens billed so far this turn — the composer shows the running total.
    pub tokens: u32,
}

impl Agent {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            state: State::Idle,
            delay: 0,
            progress: 0,
            approved: false,
            trusted: Vec::new(),
            tokens: 0,
        }
    }

    /// True while a turn is in flight — including while it waits for approval.
    pub fn is_running(&self) -> bool {
        !matches!(self.state, State::Idle)
    }

    /// The command the turn is blocked on, if any.
    pub fn pending_approval(&self) -> Option<&str> {
        match &self.state {
            State::Blocked(command) => Some(command),
            _ => None,
        }
    }

    /// Begin a turn for `prompt`.
    pub fn start(&mut self, prompt: &str) {
        self.queue = script_for(prompt);
        self.state = State::Running;
        self.delay = START_DELAY;
        self.progress = 0;
        self.approved = false;
        self.tokens += TOKENS_PER_TURN;
    }

    /// Abandon the turn (Esc / Ctrl+C). Returns true if there was one to abandon.
    pub fn interrupt(&mut self) -> bool {
        if !self.is_running() {
            return false;
        }
        self.queue.clear();
        self.state = State::Idle;
        self.progress = 0;
        true
    }

    /// Answer the pending approval prompt.
    pub fn approve(&mut self, decision: Decision, cells: &mut Vec<Cell>) {
        let State::Blocked(command) = &self.state else {
            return;
        };
        let command = command.clone();
        match decision {
            Decision::Once | Decision::Session => {
                if decision == Decision::Session {
                    // Codex trusts the whole program, not the exact argv.
                    let program = command.split_whitespace().next().unwrap_or(&command);
                    self.trusted.push(program.to_string());
                }
                self.approved = true;
                self.state = State::Running;
            }
            Decision::Deny => {
                cells.push(Cell::Exec {
                    command,
                    output: vec!["not run — the user rejected this command".into()],
                    status: ExecStatus::Rejected,
                });
                // Drop the rest of the plan and answer with what is left to do.
                self.queue.clear();
                self.queue.push_back(Step::Say(REJECTED_ANSWER));
                self.progress = 0;
                self.approved = false;
                self.state = State::Running;
                self.delay = STEP_DELAY;
            }
        }
    }

    /// Advance the turn by one frame, appending to (and streaming into) `cells`.
    pub fn tick(&mut self, cells: &mut Vec<Cell>) {
        if !matches!(self.state, State::Running) {
            return;
        }
        if self.delay > 0 {
            self.delay -= 1;
            return;
        }
        let Some(step) = self.queue.front() else {
            self.state = State::Idle;
            return;
        };

        let finished = match step {
            Step::Think(body) => {
                if self.progress == 0 {
                    cells.push(Cell::Reasoning {
                        body: String::new(),
                    });
                }
                let mut done = true;
                if let Some(Cell::Reasoning { body: shown }) = cells.last_mut() {
                    done = stream(body, &mut self.progress, |chunk| shown.push_str(chunk));
                }
                self.bill(STREAM_CHARS);
                done
            }
            Step::Explore(items) => {
                if self.progress == 0 {
                    cells.push(Cell::Explore { items: Vec::new() });
                }
                if let Some(Cell::Explore { items: shown }) = cells.last_mut()
                    && let Some(next) = items.get(self.progress)
                {
                    shown.push((*next).to_string());
                    self.tokens += TOKENS_PER_FILE;
                }
                self.progress += 1;
                self.delay = OUTPUT_DELAY;
                self.progress >= items.len()
            }
            Step::Exec {
                command,
                output,
                approval,
                exit,
            } => {
                let program = command.split_whitespace().next().unwrap_or(command);
                let needs_approval =
                    *approval && !self.approved && !self.trusted.iter().any(|t| t == program);
                if needs_approval {
                    self.state = State::Blocked((*command).to_string());
                    return;
                }
                if self.progress == 0 {
                    cells.push(Cell::Exec {
                        command: (*command).to_string(),
                        output: Vec::new(),
                        status: ExecStatus::Running,
                    });
                }
                let done = self.progress >= output.len();
                if let Some(Cell::Exec {
                    output: shown,
                    status,
                    ..
                }) = cells.last_mut()
                {
                    if let Some(next) = output.get(self.progress) {
                        shown.push((*next).to_string());
                    }
                    if done {
                        *status = if *exit == 0 {
                            ExecStatus::Ok
                        } else {
                            ExecStatus::Failed(*exit)
                        };
                    }
                }
                self.progress += 1;
                self.delay = OUTPUT_DELAY;
                self.tokens += TOKENS_PER_OUTPUT_ROW;
                done
            }
            Step::Edit {
                path,
                added,
                removed,
                hunk,
            } => {
                cells.push(Cell::Patch {
                    header: format!("Edited {path} (+{added} -{removed})"),
                    hunk: hunk.iter().map(|l| (*l).to_string()).collect(),
                });
                self.tokens += TOKENS_PER_PATCH;
                true
            }
            Step::Say(text) => {
                if self.progress == 0 {
                    cells.push(Cell::Answer(Box::new(MarkdownState::new())));
                }
                let mut done = true;
                if let Some(Cell::Answer(state)) = cells.last_mut() {
                    done = stream(text, &mut self.progress, |chunk| state.push_str(chunk));
                }
                self.bill(STREAM_CHARS);
                done
            }
        };

        if finished {
            self.queue.pop_front();
            self.progress = 0;
            self.approved = false;
            self.delay = STEP_DELAY;
            if self.queue.is_empty() {
                self.state = State::Idle;
            }
        }
    }

    fn bill(&mut self, chars: usize) {
        self.tokens += (chars as f32 * TOKENS_PER_CHAR).ceil() as u32;
    }
}

/// Reveal the next [`STREAM_CHARS`] characters of `src` through `sink`.
/// Returns true once all of `src` has been revealed.
fn stream(src: &str, progress: &mut usize, sink: impl FnOnce(&str)) -> bool {
    let total = src.chars().count();
    let end = (*progress + STREAM_CHARS).min(total);
    let chunk: String = src.chars().take(end).skip(*progress).collect();
    sink(&chunk);
    *progress = end;
    *progress >= total
}

/// Pick a canned turn from the prompt's keywords.
fn script_for(prompt: &str) -> VecDeque<Step> {
    let p = prompt.to_ascii_lowercase();
    let has = |keys: &[&str]| keys.iter().any(|k| p.contains(k));
    // Order matters: "why does the test fail?" is a fix, not an explanation.
    let steps: Vec<Step> = if p.starts_with("/init") {
        init_script()
    } else if has(&["rm ", "clean", "delete", "wipe", "prune"]) {
        cleanup_script()
    } else if has(&["test", "fail", "fix", "bug", "snapshot", "broken", "red"]) {
        fix_script()
    } else if has(&[
        "explain", "what", "how", "why", "describe", "read", "tell me",
    ]) {
        explain_script()
    } else {
        fix_script()
    };
    steps.into()
}

/// The default turn: investigate a failing test, patch it, prove it green.
fn fix_script() -> Vec<Step> {
    vec![
        Step::Think(
            "The snapshot grids are checked in as LF-only, so a CRLF checkout is not it — CI is \
             Linux anyway. More likely the composer grew a row and the stored grid is one line \
             short. Let me look at the snapshot module first.",
        ),
        Step::Explore(&[
            "Read src/snapshots.rs",
            "Search \"UPDATE_SNAPSHOTS\" in src",
            "List src/snapshots",
        ]),
        Step::Exec {
            command: "cargo test --all-features snapshots",
            output: &[
                "   Compiling tuika v0.5.0 (/home/you/code/tuika)",
                "    Finished `test` profile in 6.41s",
                "     Running unittests src/lib.rs",
                "",
                "---- snapshots::composer_screen stdout ----",
                "thread 'snapshots::composer_screen' panicked at src/snapshots.rs:118",
                "snapshot mismatch: expected 14 rows, rendered 15",
                "",
                "test result: FAILED. 341 passed; 1 failed",
            ],
            approval: false,
            exit: 101,
        },
        Step::Think(
            "Confirmed: the stored grid predates the hint row under the composer. The render is \
             the correct one, so the snapshot is what needs refreshing.",
        ),
        Step::Edit {
            path: "src/snapshots/composer.txt",
            added: 2,
            removed: 1,
            hunk: &[
                "@@ -12,4 +12,5 @@",
                " ╰──────────────────────────────╯",
                "-",
                "+  ⏎ send   ⇧⏎ newline   ⌃C quit",
                "+",
            ],
        },
        Step::Exec {
            command: "cargo test --all-features snapshots",
            output: &[
                "    Finished `test` profile in 0.92s",
                "     Running unittests src/lib.rs",
                "",
                "test result: ok. 342 passed; 0 failed; 0 ignored",
            ],
            approval: false,
            exit: 0,
        },
        Step::Say(
            "Fixed. The failure was a **stale golden snapshot**, not a renderer bug.\n\n\
             - `composer_screen` pinned a 14-row grid from before the hint row landed under the \
             composer.\n\
             - The render itself is correct, so I refreshed the grid instead of changing layout \
             code.\n\n\
             ```bash\n\
             UPDATE_SNAPSHOTS=1 cargo test --all-features snapshots\n\
             ```\n\n\
             `cargo test --all-features` is green again: 342 passed, 0 failed.\n",
        ),
    ]
}

/// A turn that trips the approval policy: a destructive command the sandbox
/// will not run unattended.
fn cleanup_script() -> Vec<Step> {
    vec![
        Step::Think(
            "Stale build artifacts are outside the writable workspace roots, so removing them \
             needs an escalation. I'll ask before touching anything on disk.",
        ),
        Step::Exec {
            command: "rm -rf target/debug/incremental",
            output: &["removed 412 files", "reclaimed 1.8 GiB"],
            approval: true,
            exit: 0,
        },
        Step::Exec {
            command: "du -sh target",
            // Spaces, not a tab: transcript rows are cells, and a `\t` would
            // land as one blank instead of aligning a column.
            output: &["612M    target"],
            approval: false,
            exit: 0,
        },
        Step::Say(
            "Cleared the incremental cache — `target/` is down from **2.4 GiB** to 612 MiB.\n\n\
             Nothing else was touched; the compiled test binaries are still there, so the next \
             `cargo test` only relinks.\n",
        ),
    ]
}

/// A read-only turn: explain the repository without editing anything.
fn explain_script() -> Vec<Step> {
    vec![
        Step::Explore(&[
            "Read README.md",
            "Read src/lib.rs",
            "Search \"pub fn paint\" in src",
            "Read src/view.rs",
        ]),
        Step::Say(
            "This crate is a **composable terminal UI toolkit** over `ratatui`.\n\n\
             The model is small:\n\n\
             1. `View`s are rebuilt from application state every frame — no reconciler, because \
             `ratatui` already diffs the cell buffer.\n\
             2. State that must survive a frame (scroll offset, selection, focus) lives in \
             host-owned `*State` structs.\n\
             3. `paint` solves a flexbox layout, then composites any anchored overlays last.\n\n\
             ```rust\n\
             let root = view! {\n\
             \x20   col(gap = 1) {\n\
             \x20       grow(1) { node(transcript) }\n\
             \x20       fixed(3) { node(composer) }\n\
             \x20   }\n\
             };\n\
             ```\n\n\
             Nothing in it knows about the application embedding it — highlighting, image \
             decoding, and data fetching are all seams the host fills.\n",
        ),
    ]
}

/// `/init` — write the agent instructions file.
fn init_script() -> Vec<Step> {
    vec![
        Step::Explore(&[
            "List .",
            "Read Cargo.toml",
            "Read README.md",
            "Read CONTRIBUTING.md",
        ]),
        Step::Think(
            "A workspace with one library crate and a published companion crate. The contributing \
             guide already documents the local checks, so AGENTS.md should point at them rather \
             than restate them.",
        ),
        Step::Edit {
            path: "AGENTS.md",
            added: 24,
            removed: 0,
            hunk: &[
                "@@ -0,0 +1,24 @@",
                "+# Repository guidance",
                "+",
                "+## Local checks",
                "+",
                "+- `cargo fmt --all -- --check`",
                "+- `cargo clippy --all-targets --all-features -- -D warnings`",
                "+- `cargo test --all-features`",
            ],
        },
        Step::Say(
            "Wrote `AGENTS.md` with the build, test, and style conventions I found in \
             `CONTRIBUTING.md` and the workspace manifest.\n\n\
             Edit it freely — it is read on every turn, so keep it short.\n",
        ),
    ]
}

/// What the model says after the user rejects a command.
const REJECTED_ANSWER: &str = "Understood — I left the working tree alone.\n\nIf you would rather do it yourself, the \
     command was:\n\n```bash\nrm -rf target/debug/incremental\n```\n\nOtherwise, tell me what \
     you would like me to try instead.\n";

/// Notice text for an interrupted turn.
pub fn interrupted_notice() -> Cell {
    Cell::Notice {
        tone: Tone::Warn,
        title: "Interrupted".into(),
        body: vec!["the turn was cancelled before it finished".into()],
    }
}
