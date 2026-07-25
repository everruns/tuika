//! Terminal-protocol smoke for the host, in both screen modes.
//!
//! Unit, property, and snapshot tests render into an in-memory ratatui `Buffer`
//! and never touch a real terminal. This drives real examples under a
//! pseudo-terminal (`portable-pty`) and asserts the terminal-facing behavior a
//! buffer test cannot see: entering and restoring the alternate screen, enhanced
//! keyboard reporting, cursor and mouse-capture lifecycle, the native OSC 9;4
//! progress indicator, OSC 8 hyperlinks, truecolor and Braille cells surviving a
//! reference terminal parser, resize survival, and a clean exit — the `gallery`
//! example for the alternate screen, and `split_footer` for a footer over live
//! scrollback.
//!
//! This covers the *protocol* a terminal receives. Whether a specific emulator
//! paints those bytes correctly is the cross-terminal nightly plus the manual
//! matrix in `knowledge/processes/release.md`.
#![cfg(unix)]

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

/// The URL rendered in the gallery footer, emitted as an OSC 8 hyperlink target.
const GALLERY_URL: &str = "https://github.com/everruns/tuika";

/// Locate a compiled example binary.
///
/// Examples get no `CARGO_BIN_EXE_*` env var, but `cargo test` builds them, and
/// they land in `examples/` next to this test binary's own directory
/// (`target/<profile>/deps/<test>` → `target/<profile>/examples/<name>`).
fn example_bin(name: &str) -> PathBuf {
    let test_bin = std::env::current_exe().expect("test binary path");
    let profile_dir = test_bin
        .parent()
        .and_then(|deps| deps.parent())
        .expect("target/<profile> directory");
    let bin = profile_dir.join("examples").join(name);
    assert!(
        bin.is_file(),
        "the `{name}` example was not built at {} — `cargo test` should build \
         examples; run `cargo build --example {name}` if invoking the test \
         binary directly",
        bin.display()
    );
    bin
}

struct ExampleRun {
    /// Everything the terminal received, through clean exit. Its final vt100
    /// state is the *restored* main screen, so byte-level assertions use this
    /// but grid assertions must use `live` (see below).
    output: Vec<u8>,
    /// A snapshot of the byte stream taken while the example was still painting,
    /// i.e. before `q`. Parsing this into a vt100 screen reconstructs the live
    /// grid; for an alternate-screen example, parsing `output` would instead
    /// show the blank main screen the teardown restores.
    live: Vec<u8>,
    /// The byte stream as it stood after each scripted key, so a test can read
    /// the grid *between* inputs — the only way to assert a transition (a footer
    /// that grows and then shrinks again) rather than just its end state.
    steps: Vec<Vec<u8>>,
    exited_ok: bool,
    rows: u16,
    cols: u16,
}

impl ExampleRun {
    /// Reconstruct the live grid: a reference terminal (vt100) applies every
    /// escape, so assertions read the resulting cell grid rather than the raw
    /// byte stream.
    fn live_screen(&self) -> vt100::Parser {
        let mut parser = vt100::Parser::new(self.rows, self.cols, 0);
        parser.process(&self.live);
        parser
    }

    /// The grid as it stood just after scripted key `index`.
    fn step_screen(&self, index: usize) -> vt100::Parser {
        let mut parser = vt100::Parser::new(self.rows, self.cols, 0);
        parser.process(&self.steps[index]);
        parser
    }

    /// The grid the user is left with after the example exits — for a split
    /// footer, the published scrollback with the footer's rows handed back.
    fn final_screen(&self) -> vt100::Parser {
        let mut parser = vt100::Parser::new(self.rows, self.cols, 0);
        parser.process(&self.output);
        parser
    }

    /// Everything the terminal *holds* at the end: the visible screen plus its
    /// scrollback buffer. Published output that scrolled off the top is still
    /// the user's — reading it back is how "this really is terminal scrollback"
    /// is asserted, rather than "it happened to still be on screen".
    fn scrollback_text(&self) -> String {
        const PAGES: usize = 12;
        let mut parser = vt100::Parser::new(self.rows, self.cols, PAGES * self.rows as usize);
        parser.process(&self.output);
        let mut text = String::new();
        for page in (0..=PAGES).rev() {
            parser
                .screen_mut()
                .set_scrollback(page * self.rows as usize);
            text.push_str(&parser.screen().contents());
            text.push('\n');
        }
        text
    }
}

/// A scripted run of one example under a pty.
///
/// Defaults are the gallery's: a 24×80 screen, a second or so of painting, and
/// `q` to quit. Anything else — a flag, a resize, keys typed mid-run, a
/// different quit key — is a builder call, so each test reads as the session it
/// drives.
struct Script {
    name: &'static str,
    args: Vec<&'static str>,
    rows: u16,
    cols: u16,
    settle: Duration,
    resize_to: Option<(u16, u16)>,
    /// Bytes to type after the settle, each followed by a pause.
    keys: Vec<(&'static [u8], Duration)>,
    quit: &'static [u8],
}

impl Script {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            args: Vec::new(),
            rows: 24,
            cols: 80,
            settle: Duration::from_millis(1200),
            resize_to: None,
            keys: Vec::new(),
            quit: b"q",
        }
    }

    fn size(mut self, rows: u16, cols: u16) -> Self {
        (self.rows, self.cols) = (rows, cols);
        self
    }

    fn arg(mut self, arg: &'static str) -> Self {
        self.args.push(arg);
        self
    }

    fn settle(mut self, settle: Duration) -> Self {
        self.settle = settle;
        self
    }

    /// Resize the pty mid-run to `cols × rows`.
    fn resize_to(mut self, cols: u16, rows: u16) -> Self {
        self.resize_to = Some((cols, rows));
        self
    }

    fn key(mut self, bytes: &'static [u8], pause: Duration) -> Self {
        self.keys.push((bytes, pause));
        self
    }

    fn quit_with(mut self, bytes: &'static [u8]) -> Self {
        self.quit = bytes;
        self
    }

    /// Spawn the example, follow the script, then collect everything the
    /// terminal received.
    fn run(self) -> ExampleRun {
        let pty = NativePtySystem::default();
        let pair = pty
            .openpty(PtySize {
                rows: self.rows,
                cols: self.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open pty");

        let mut cmd = CommandBuilder::new(example_bin(self.name));
        for arg in &self.args {
            cmd.arg(arg);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        // The recorder's own environment must not leak into the child: an
        // inherited `TMUX` or `TERM_PROGRAM` would change which keyboard and
        // palette protocols the session negotiates.
        for inherited in [
            "NO_COLOR",
            "TERM_PROGRAM",
            "LC_TERMINAL",
            "TMUX",
            "TMUX_PANE",
        ] {
            cmd.env_remove(inherited);
        }

        let mut child = pair.slave.spawn_command(cmd).expect("spawn example");
        drop(pair.slave);

        // One writer, shared: the reader thread answers cursor-position queries
        // on it, and this thread types the script.
        let writer = Arc::new(Mutex::new(pair.master.take_writer().expect("pty writer")));
        // The size the reader's reference terminal models, updated on resize.
        let size = Arc::new(Mutex::new((self.rows, self.cols)));

        // Reader thread accumulates all bytes until the pty closes (child exit),
        // and plays enough of a terminal to keep the child running: an inline
        // viewport is anchored with a DSR cursor-position query (`ESC [ 6 n`),
        // and an unanswered query is a hard error inside ratatui's
        // initialization. The reply has to be *truthful* — the viewport is
        // placed at the row it reports — so the answer comes from a vt100 parser
        // fed the same byte stream, rather than a fixed row.
        let reader = pair.master.try_clone_reader().expect("clone reader");
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let reader_buffer = Arc::clone(&buffer);
        let reader_writer = Arc::clone(&writer);
        let reader_size = Arc::clone(&size);
        let (rows, cols) = (self.rows, self.cols);
        let reader_handle = thread::spawn(move || {
            let mut reader = reader;
            let mut parser = vt100::Parser::new(rows, cols, 0);
            let mut modeled = (rows, cols);
            let mut chunk = [0u8; 8192];
            while let Ok(n) = reader.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let bytes = &chunk[..n];
                reader_buffer
                    .lock()
                    .expect("lock read buffer")
                    .extend_from_slice(bytes);

                let current = *reader_size.lock().expect("lock size");
                if current != modeled {
                    parser.screen_mut().set_size(current.0, current.1);
                    modeled = current;
                }
                parser.process(bytes);
                if contains(bytes, b"\x1b[6n") {
                    let (row, col) = parser.screen().cursor_position();
                    let reply = format!("\x1b[{};{}R", row + 1, col + 1);
                    let mut writer = reader_writer.lock().expect("lock pty writer");
                    let _ = writer.write_all(reply.as_bytes());
                    let _ = writer.flush();
                }
            }
        });

        let send = |bytes: &[u8]| {
            let mut writer = writer.lock().expect("lock pty writer");
            writer.write_all(bytes).expect("send input");
            writer.flush().expect("flush input");
        };

        // Let it paint a few frames, then follow the script, keeping the stream
        // as it stood after each key so a test can read the grid between them.
        thread::sleep(self.settle);
        let mut steps = Vec::with_capacity(self.keys.len());
        for (bytes, pause) in &self.keys {
            send(bytes);
            thread::sleep(*pause);
            steps.push(buffer.lock().expect("lock read buffer").clone());
        }

        let (mut live_rows, mut live_cols) = (self.rows, self.cols);
        if let Some((c, r)) = self.resize_to {
            *size.lock().expect("lock size") = (r, c);
            pair.master
                .resize(PtySize {
                    rows: r,
                    cols: c,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .expect("resize pty");
            (live_rows, live_cols) = (r, c);
            thread::sleep(Duration::from_millis(800));
        }

        // Snapshot the byte stream while the example is still painting, before
        // the quit key tears the frame down — this is what live grid assertions
        // parse.
        let live = buffer.lock().expect("lock read buffer").clone();

        send(self.quit);

        // Wait for a clean exit.
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut exited_ok = false;
        loop {
            match child.try_wait().expect("poll child") {
                Some(status) => {
                    exited_ok = status.success();
                    break;
                }
                None if Instant::now() > deadline => {
                    let _ = child.kill();
                    break;
                }
                None => thread::sleep(Duration::from_millis(100)),
            }
        }

        // The child has exited, so the reader hits EOF; join it and take the
        // bytes.
        drop(pair.master);
        let _ = reader_handle.join();
        let output = Arc::try_unwrap(buffer)
            .expect("reader thread finished")
            .into_inner()
            .expect("read buffer");
        ExampleRun {
            output,
            live,
            steps,
            exited_ok,
            rows: live_rows,
            cols: live_cols,
        }
    }
}

/// The gallery at a given size, optionally resized mid-run.
fn run_gallery(rows: u16, cols: u16, resize_to: Option<(u16, u16)>) -> ExampleRun {
    let script = Script::new("gallery").size(rows, cols);
    match resize_to {
        Some((c, r)) => script.resize_to(c, r).run(),
        None => script.run(),
    }
}

/// The split-footer example, painting long enough for its worker to publish.
fn run_split_footer() -> ExampleRun {
    Script::new("split_footer")
        .size(20, 80)
        .settle(Duration::from_millis(1600))
        .run()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Whether `text` contains a codepoint from the U+2800 Braille block — what the
/// gallery's default (Braille) spinner draws.
fn has_braille(text: &str) -> bool {
    text.chars().any(|c| ('\u{2800}'..='\u{28ff}').contains(&c))
}

/// Whether any cell in the parsed screen is painted with a 24-bit RGB color
/// (foreground or background) — proof the truecolor theme survived a real
/// terminal parser, not just that RGB bytes appeared in the stream.
fn has_rgb_cell(screen: &vt100::Screen) -> bool {
    let (rows, cols) = screen.size();
    (0..rows).any(|r| {
        (0..cols).any(|c| {
            screen.cell(r, c).is_some_and(|cell| {
                matches!(cell.fgcolor(), vt100::Color::Rgb(..))
                    || matches!(cell.bgcolor(), vt100::Color::Rgb(..))
            })
        })
    })
}

/// Strip CSI/OSC sequences so we can look for rendered text.
fn visible_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // Skip an escape sequence: ESC [ ... <final>, or ESC ] ... BEL.
            match chars.next() {
                Some('[') => {
                    for e in chars.by_ref() {
                        if e.is_ascii_alphabetic() || e == '~' {
                            break;
                        }
                    }
                }
                Some(']') => {
                    for e in chars.by_ref() {
                        if e == '\u{7}' {
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[test]
fn gallery_drives_altscreen_and_native_progress() {
    let run = run_gallery(24, 80, None);
    let out = &run.output;

    assert!(
        run.exited_ok,
        "gallery should exit cleanly on `q`: {}",
        visible_text(out)
    );
    assert!(
        contains(out, b"\x1b[?1049h"),
        "should enter the alternate screen"
    );
    assert!(
        contains(out, b"\x1b[?1049l"),
        "should restore the main screen on exit"
    );
    assert!(contains(out, b"\x1b[?25l"), "should hide the cursor");
    assert!(
        contains(out, b"\x1b[?25h"),
        "should restore the cursor on exit"
    );
    assert!(contains(out, b"\x1b[?1000h"), "should enable mouse capture");
    assert!(
        contains(out, b"\x1b[?1000l"),
        "should disable mouse capture on exit"
    );
    assert!(
        contains(out, b"\x1b[>7u"),
        "should request modified-key and key-event reporting"
    );
    assert!(
        contains(out, b"\x1b[<1u"),
        "should pop enhanced keyboard reporting on exit"
    );
    // OSC 9;4: indeterminate on start, cleared on exit.
    assert!(
        contains(out, b"\x1b]9;4;3"),
        "should emit the native indeterminate progress sequence"
    );
    assert!(
        contains(out, b"\x1b]9;4;0"),
        "should clear the native progress indicator on exit"
    );

    let text = visible_text(out);
    assert!(
        text.contains("spinners") && text.contains("progress"),
        "gallery chrome should render: {text:?}"
    );
}

#[test]
fn gallery_paints_truecolor_and_braille() {
    let run = run_gallery(24, 80, None);
    assert!(
        run.exited_ok,
        "gallery should exit cleanly: {}",
        visible_text(&run.output)
    );

    // Assert against the parsed grid — a reference terminal (vt100) applies
    // every escape — so these cover the matrix rows at the cell level, not just
    // as a substring in the byte stream a real emulator might interpret away.
    let parser = run.live_screen();
    let screen = parser.screen();

    // Truecolor: the default theme is 24-bit RGB, so at least one painted cell
    // must carry an RGB color after parsing — the "truecolor" matrix row.
    assert!(
        has_rgb_cell(screen),
        "expected a 24-bit truecolor cell from the RGB theme:\n{}",
        screen.contents()
    );

    // Braille: the default spinner draws from the U+2800 block, so a Braille
    // glyph must land in a grid cell — the "Braille glyphs" matrix row.
    assert!(
        has_braille(&screen.contents()),
        "expected Braille spinner glyphs on screen:\n{}",
        screen.contents()
    );
}

#[test]
fn gallery_survives_resize() {
    // Start wide, shrink to a small size mid-run.
    let run = run_gallery(24, 100, Some((40, 12)));
    assert!(
        run.exited_ok,
        "gallery should survive a resize and exit cleanly: {}",
        visible_text(&run.output)
    );
    assert!(
        contains(&run.output, b"\x1b[?1049l"),
        "should still restore the screen after a resize"
    );
}

#[test]
fn gallery_emits_osc8_hyperlink() {
    // Taller than the other cases on purpose: the footer sits below every demo
    // panel, so on a short screen the layout clips it away and there is no URL
    // run for the backend to wrap. 44 rows leaves headroom above the ~38 the
    // gallery needs to reach the footer.
    let run = run_gallery(44, 100, None);
    assert!(run.exited_ok, "gallery should exit cleanly");
    // The gallery wraps its footer URL through `HyperlinkBackend`, so the byte
    // stream must carry `ESC ] 8 ; ; <url> ST` targeting the URL itself —
    // proven end-to-end through a real terminal, not just the unit tests.
    let osc8 = format!("\x1b]8;;{GALLERY_URL}\x1b\\");
    assert!(
        contains(&run.output, osc8.as_bytes()),
        "expected an OSC 8 hyperlink for {GALLERY_URL}"
    );

    // vt100 does not expose OSC 8 targets, but it does prove the wrapping left
    // the surrounding footer text intact — the "surrounding text/wrapping
    // undamaged" concern in the OSC 8 matrix row. The footer reads
    // `docs <url>  ·  …`, so both the label and the full URL survive on one row.
    let parser = run.live_screen();
    let footer = parser
        .screen()
        .contents()
        .lines()
        .find(|line| line.contains(GALLERY_URL))
        .map(ToOwned::to_owned)
        .unwrap_or_default();
    assert!(
        footer.contains("docs") && footer.contains(GALLERY_URL),
        "OSC 8 wrapping should leave the footer text intact: {footer:?}"
    );
}

/// Rows the `split_footer` example reserves (`FOOTER_ROWS` in that example).
const FOOTER_ROWS: u16 = 5;

/// The last `FOOTER_ROWS` rows of a parsed screen, joined for assertions.
fn footer_rows(screen: &vt100::Screen, rows: u16) -> String {
    screen
        .contents()
        .lines()
        .skip(rows.saturating_sub(FOOTER_ROWS) as usize)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The rows above the footer: the terminal's own scrollback while tuika runs.
fn scrollback_rows(screen: &vt100::Screen, rows: u16) -> String {
    screen
        .contents()
        .lines()
        .take(rows.saturating_sub(FOOTER_ROWS) as usize)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn split_footer_stays_on_the_main_screen_and_leaves_the_mouse_alone() {
    let run = run_split_footer();
    let out = &run.output;

    assert!(
        run.exited_ok,
        "split_footer should exit cleanly on `q`: {}",
        visible_text(out)
    );
    // The whole point of the mode: the user's screen and scrollback are never
    // swapped out from under them.
    assert!(
        !contains(out, b"\x1b[?1049h"),
        "a split footer must not enter the alternate screen"
    );
    // Nor is the wheel and drag-selection taken away from the terminal, which
    // is what would make the preserved scrollback unusable.
    assert!(
        !contains(out, b"\x1b[?1000h"),
        "a split footer must not capture the mouse by default"
    );
    assert!(contains(out, b"\x1b[?25l"), "should hide the cursor");
    assert!(
        contains(out, b"\x1b[?25h"),
        "should restore the cursor on exit"
    );
}

#[test]
fn split_footer_pins_the_footer_and_publishes_above_it() {
    let run = run_split_footer();
    assert!(
        run.exited_ok,
        "split_footer should exit cleanly: {}",
        visible_text(&run.output)
    );

    let parser = run.live_screen();
    let screen = parser.screen();

    // The footer is pinned to the bottom rows, not left floating where the
    // shell prompt happened to be.
    let footer = footer_rows(screen, run.rows);
    assert!(
        footer.contains("split footer"),
        "the footer box should occupy the last {FOOTER_ROWS} rows:\n{}",
        screen.contents()
    );

    // Published blocks land above it, as ordinary terminal lines.
    let above = scrollback_rows(screen, run.rows);
    assert!(
        above.contains("build finished in"),
        "published output should sit above the footer:\n{}",
        screen.contents()
    );
    assert!(
        !above.contains("split footer"),
        "the footer must not be duplicated into the scrollback:\n{}",
        screen.contents()
    );
}

#[test]
fn split_footer_hands_its_rows_back_and_keeps_the_scrollback_on_exit() {
    let run = run_split_footer();
    assert!(run.exited_ok, "split_footer should exit cleanly");

    // Unlike the alternate screen, what the app published is still on screen
    // after it exits — that is the scrollback the user keeps.
    let parser = run.final_screen();
    let screen = parser.screen();
    assert!(
        screen.contents().contains("build finished in"),
        "published scrollback should survive the exit:\n{}",
        screen.contents()
    );
    // The reserved rows are given back blank, so the shell prompt does not
    // print over a leftover frame.
    assert!(
        !screen.contents().contains("split footer"),
        "the footer's rows should be released on exit:\n{}",
        screen.contents()
    );
}

#[test]
fn split_footer_survives_resize() {
    let run = Script::new("split_footer")
        .size(24, 100)
        .resize_to(60, 14)
        .run();
    assert!(
        run.exited_ok,
        "split_footer should survive a resize and exit cleanly: {}",
        visible_text(&run.output)
    );

    // Re-pinned against the new height rather than left where the old bottom
    // was: the footer is still on the last rows after the resize.
    let parser = run.live_screen();
    let screen = parser.screen();
    assert!(
        footer_rows(screen, run.rows).contains("split footer"),
        "the footer should be re-pinned to the bottom after a resize:\n{}",
        screen.contents()
    );
}

#[test]
fn split_footer_captures_the_mouse_only_when_asked() {
    let run = Script::new("split_footer")
        .arg("--mouse")
        .size(20, 80)
        .settle(Duration::from_millis(900))
        .run();
    assert!(
        run.exited_ok,
        "split_footer --mouse should exit cleanly: {}",
        visible_text(&run.output)
    );
    // The opt-in is a real capture/release pair, so a footer that wants mouse
    // input gets it — and still hands it back on exit.
    assert!(
        contains(&run.output, b"\x1b[?1000h"),
        "`--mouse` should enable mouse capture"
    );
    assert!(
        contains(&run.output, b"\x1b[?1000l"),
        "`--mouse` should disable mouse capture on exit"
    );
    // Opting into the mouse must not opt into the alternate screen.
    assert!(
        !contains(&run.output, b"\x1b[?1049h"),
        "still a main-screen footer"
    );
}

/// Rows the `codex` example reserves in split-footer mode (`FOOTER_ROWS`).
const CODEX_FOOTER_ROWS: u16 = 12;

/// The codex example in split-footer mode: type a prompt, let the scripted turn
/// stream for a while, `Esc` to end it (so the run does not depend on how long
/// the streamed answer takes), then `⌃C` to quit from idle.
fn run_codex_split_footer() -> ExampleRun {
    Script::new("codex")
        .arg("--split-footer")
        .size(24, 84)
        .settle(Duration::from_millis(900))
        .key(b"explain this repo\r", Duration::from_millis(5000))
        .key(b"\x1b", Duration::from_millis(600))
        .quit_with(b"\x03")
        .run()
}

#[test]
fn codex_publishes_its_transcript_and_keeps_the_composer_pinned() {
    let run = run_codex_split_footer();
    assert!(
        run.exited_ok,
        "codex --split-footer should exit cleanly on ⌃C: {}",
        visible_text(&run.output)
    );
    assert!(
        !contains(&run.output, b"\x1b[?1049h"),
        "a split footer must not enter the alternate screen"
    );

    let parser = run.live_screen();
    let screen = parser.screen();
    let contents = screen.contents();
    let footer: String = contents
        .lines()
        .skip((run.rows - CODEX_FOOTER_ROWS) as usize)
        .collect::<Vec<_>>()
        .join("\n");
    let above: String = contents
        .lines()
        .take((run.rows - CODEX_FOOTER_ROWS) as usize)
        .collect::<Vec<_>>()
        .join("\n");

    // The composer and its footer own the reserved rows, and only those...
    assert!(
        footer.contains("Ask Codex") && footer.contains("context left"),
        "the composer should own the footer rows:\n{contents}"
    );
    assert!(
        !above.contains("Ask Codex"),
        "nothing of the composer may leak above the footer:\n{contents}"
    );
    // ...while the turn's finished entries went to the terminal, where this app
    // can no longer repaint them.
    let held = run.scrollback_text();
    assert!(
        held.contains("Interrupted"),
        "the turn's last entry should be published above the footer:\n{contents}"
    );
    // Everything before it has scrolled off the visible screen by now, so
    // finding those rows proves the stronger claim: published output entered the
    // terminal's *scrollback buffer*, not merely the last screenful.
    if SCROLLBACK_IS_KEPT {
        assert!(
            held.contains("explain this repo") && held.contains("Read README.md"),
            "the whole session should be in the terminal's scrollback:\n{held}"
        );
    }
}

/// Whether rows that scroll off the top reach the terminal's scrollback buffer.
///
/// They do on the portable publish path, which scrolls the whole screen. They do
/// *not* under `feature = "scrolling-regions"`: a DECSTBM region discards what
/// scrolls out of it, so published output survives only while it is on screen.
/// That is why the feature is a compatibility mirror rather than an optimization
/// a split footer should reach for — see `Cargo.toml` and
/// `knowledge/specs/screen-modes.md`.
const SCROLLBACK_IS_KEPT: bool = !cfg!(feature = "scrolling-regions");

#[test]
fn codex_leaves_the_session_in_the_scrollback_after_exit() {
    let run = run_codex_split_footer();
    assert!(run.exited_ok, "codex --split-footer should exit cleanly");

    // What a coding agent's split-footer mode is *for*: the transcript is still
    // in the terminal after the tool exits.
    let held = run.scrollback_text();
    assert!(
        held.contains("Interrupted"),
        "the session should survive the exit:\n{held}"
    );
    if SCROLLBACK_IS_KEPT {
        assert!(
            held.contains("explain this repo") && held.contains("Read README.md"),
            "the whole transcript should survive, not just the last screen:\n{held}"
        );
    }
    // And the reserved rows are gone with it.
    let parser = run.final_screen();
    let contents = parser.screen().contents();
    assert!(
        !contents.contains("Ask Codex"),
        "the composer's rows should be released:\n{contents}"
    );
}

/// Rows the codex footer currently reserves, measured from the terminal's own
/// grid: the footer paints a themed background across its whole area, and the
/// scrollback above it keeps the terminal's default. Counting painted rows from
/// the bottom is therefore a direct reading of how many rows the app took —
/// which is the thing a resizable footer has to get right.
fn codex_footer_rows(screen: &vt100::Screen) -> usize {
    let (rows, _) = screen.size();
    (0..rows)
        .rev()
        .take_while(|&row| {
            screen
                .cell(row, 0)
                .is_some_and(|cell| cell.bgcolor() != vt100::Color::Default)
        })
        .count()
}

/// The last `rows` lines of a screen, joined — the region the footer owns.
fn last_rows(screen: &vt100::Screen, rows: usize) -> String {
    let contents = screen.contents();
    let lines: Vec<&str> = contents.lines().collect();
    lines[lines.len().saturating_sub(rows)..].join("\n")
}

#[test]
fn codex_takes_more_rows_for_a_popup_and_gives_them_back() {
    // `/` opens the slash-command popup above the composer, which is exactly the
    // case a fixed-height footer has to over-reserve for. Esc closes it again.
    let run = Script::new("codex")
        .arg("--split-footer")
        .size(24, 84)
        .settle(Duration::from_millis(900))
        .key(b"/", Duration::from_millis(800))
        .key(b"\x1b", Duration::from_millis(800))
        .quit_with(b"\x03")
        .run();
    assert!(
        run.exited_ok,
        "codex --split-footer should exit cleanly: {}",
        visible_text(&run.output)
    );

    let opened = run.step_screen(0);
    let opened = opened.screen();
    let closed = run.step_screen(1);
    let closed = closed.screen();

    let with_popup = codex_footer_rows(opened);
    let without = codex_footer_rows(closed);

    // The popup's rows are real rows of the reserved region, taken from the
    // scrollback while it is open and handed straight back when it closes.
    assert!(
        with_popup > without,
        "the popup should grow the reserved region \
         (with popup: {with_popup} rows, without: {without})\n{}",
        opened.contents()
    );
    assert!(
        (3..=6).contains(&without),
        "with the popup closed the footer is just the composer and its status \
         row, not the tallest state it ever needed ({without} rows)\n{}",
        closed.contents()
    );
    // And the popup really is what those extra rows hold.
    assert!(
        last_rows(opened, with_popup).contains("/model"),
        "the popup should be inside the reserved region:\n{}",
        opened.contents()
    );
    assert!(
        !last_rows(closed, without).contains("/model"),
        "and gone from it once it closes:\n{}",
        closed.contents()
    );

    // Whatever the region did, the session above it is untouched: the banner
    // published at startup is still in the terminal.
    let held = run.scrollback_text();
    assert!(
        held.contains("OpenAI Codex"),
        "published scrollback should survive a footer resize:\n{held}"
    );
}
