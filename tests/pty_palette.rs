//! Terminal-palette probe smoke, with the test playing the terminal.
//!
//! `tests/pty_smoke.rs` watches what tuika *writes*. This does the inverse:
//! tuika's color probe is a conversation, so the only way to prove it works is
//! to answer it. The test spawns the `inherit` example under a pty, waits for
//! the query bytes, replies with a palette of its own choosing, and checks that
//! the theme the example resolved is the one implied by those replies.
//!
//! Three things are only observable here, never in a buffer test:
//!
//! - the queries actually reach the terminal, and the reply parses back into a
//!   theme end-to-end;
//! - the read stops at the Device Attributes fence and **consumes no input past
//!   it** — a keystroke sent right behind the replies must still reach the
//!   application, not be eaten by the probe;
//! - a terminal that answers nothing does not hang the application, it falls
//!   back.
#![cfg(unix)]

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

/// The palette this fake terminal claims to have. Chosen so every value lands
/// clear of tuika's contrast floors, which keeps the expected colors exact:
/// nothing below is adjusted on its way into the theme.
const BACKGROUND: &str = "#1c1e26";
const FOREGROUND: &str = "#d5d0c8";
const BRIGHT_BLUE: &str = "#5c9fec";
const BRIGHT_GREEN: &str = "#88cc66";

/// The replies a terminal supporting the color queries would send, ending with
/// the Device Attributes answer that fences them.
fn replies() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"\x1b]10;rgb:d5d5/d0d0/c8c8\x1b\\");
    out.extend_from_slice(b"\x1b]11;rgb:1c1c/1e1e/2626\x1b\\");
    out.extend_from_slice(b"\x1b]4;10;rgb:8888/cccc/6666\x1b\\");
    out.extend_from_slice(b"\x1b]4;12;rgb:5c5c/9f9f/ecec\x1b\\");
    out.extend_from_slice(b"\x1b[?62;1;4;22c");
    out
}

/// Locate the compiled `inherit` example, built beside this test binary.
fn inherit_bin() -> PathBuf {
    let test_bin = std::env::current_exe().expect("test binary path");
    let profile_dir = test_bin
        .parent()
        .and_then(|deps| deps.parent())
        .expect("target/<profile> directory");
    let bin = profile_dir.join("examples").join("inherit");
    assert!(
        bin.is_file(),
        "the `inherit` example was not built at {} — `cargo test` should build \
         examples; run `cargo build --example inherit` if invoking the test \
         binary directly",
        bin.display()
    );
    bin
}

struct Run {
    output: Vec<u8>,
    exited_ok: bool,
}

impl Run {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.output).replace("\r\n", "\n")
    }
}

/// Spawn `inherit` under a pty. Once it has asked its questions, send `answer`
/// (if any), then wait for it to exit.
///
/// `answer` is written as a single burst so that anything after the Device
/// Attributes reply is already sitting in the terminal's buffer when the probe
/// stops reading — which is exactly the condition that catches a probe that
/// reads too far.
fn run_inherit(args: &[&str], answer: Option<Vec<u8>>) -> Run {
    let pty = NativePtySystem::default();
    let pair = pty
        .openpty(PtySize {
            rows: 30,
            cols: 90,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open pty");

    let mut cmd = CommandBuilder::new(inherit_bin());
    cmd.env("TERM", "xterm-256color");
    for arg in args {
        cmd.arg(arg);
    }

    let mut child = pair.slave.spawn_command(cmd).expect("spawn inherit");
    drop(pair.slave);

    let reader = pair.master.try_clone_reader().expect("clone reader");
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let reader_buffer = Arc::clone(&buffer);
    let reader_handle = thread::spawn(move || {
        let mut reader = reader;
        let mut chunk = [0u8; 8192];
        while let Ok(n) = reader.read(&mut chunk) {
            if n == 0 {
                break;
            }
            reader_buffer
                .lock()
                .expect("lock read buffer")
                .extend_from_slice(&chunk[..n]);
        }
    });

    let mut writer = pair.master.take_writer().expect("pty writer");
    if let Some(answer) = answer {
        // Wait until the Device Attributes request arrives: that is the last
        // thing the probe writes, so seeing it means every query is out.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let seen = buffer.lock().expect("lock read buffer").clone();
            if seen.windows(3).any(|w| w == b"\x1b[c") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the probe never sent its Device Attributes request; saw: {:?}",
                String::from_utf8_lossy(&seen)
            );
            thread::sleep(Duration::from_millis(20));
        }
        writer.write_all(&answer).expect("send replies");
        writer.flush().expect("flush replies");
    }

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
            None => thread::sleep(Duration::from_millis(50)),
        }
    }

    drop(writer);
    drop(pair.master);
    let _ = reader_handle.join();
    Run {
        output: Arc::try_unwrap(buffer)
            .expect("reader thread finished")
            .into_inner()
            .expect("read buffer"),
        exited_ok,
    }
}

#[test]
fn the_probe_asks_for_the_palette_and_fences_with_device_attributes() {
    let run = run_inherit(&["--probe"], Some(replies()));
    let bytes = &run.output;
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);

    assert!(has(b"\x1b]10;?"), "asked for the default foreground");
    assert!(has(b"\x1b]11;?"), "asked for the default background");
    for i in 0..16 {
        let q = format!("\x1b]4;{i};?");
        assert!(has(q.as_bytes()), "asked for ANSI slot {i}");
    }
    assert!(has(b"\x1b[c"), "fenced the queries with Device Attributes");
}

#[test]
fn a_terminals_answer_becomes_the_theme() {
    let run = run_inherit(&["--probe"], Some(replies()));
    let text = run.text();
    assert!(run.exited_ok, "clean exit; got:\n{text}");

    assert!(
        text.contains("source: probed"),
        "the theme came from the probe, not the fallback; got:\n{text}"
    );
    // The reported foreground and background are used verbatim…
    assert!(
        text.contains(&format!("background: {BACKGROUND}")),
        "background is the one this terminal reported; got:\n{text}"
    );
    assert!(
        text.contains(&format!("text: {FOREGROUND}")),
        "text is the reported foreground; got:\n{text}"
    );
    // …and the hues come from the ANSI slots it reported.
    assert!(
        text.contains(&format!("accent: {BRIGHT_BLUE}")),
        "accent is the reported bright blue; got:\n{text}"
    );
    assert!(
        text.contains(&format!("code.string: {BRIGHT_GREEN}")),
        "the syntax palette follows the terminal; got:\n{text}"
    );
    // The in-between tones are derived, so they are *not* simply the background.
    assert!(
        !text.contains(&format!("surface: {BACKGROUND}")),
        "surface is a derived raise, not the base fill; got:\n{text}"
    );
}

/// The fence must stop the read exactly at the Device Attributes reply. A `q`
/// sent immediately behind it belongs to the application: if the probe swallows
/// it, the example never sees the quit key and this times out instead of
/// exiting.
#[test]
fn the_probe_leaves_input_after_the_fence_for_the_application() {
    let mut answer = replies();
    answer.push(b'q');
    let run = run_inherit(&[], Some(answer));
    assert!(
        run.exited_ok,
        "the quit key sent behind the replies must survive the probe; got:\n{}",
        run.text()
    );
}

/// A terminal that implements none of this says nothing at all. The application
/// must come up anyway, on the ANSI-slot theme.
#[test]
fn a_silent_terminal_falls_back_instead_of_hanging() {
    let run = run_inherit(&["--probe"], None);
    let text = run.text();
    assert!(run.exited_ok, "clean exit; got:\n{text}");
    assert!(
        text.contains("themes::TERMINAL"),
        "fell back to the ANSI-slot theme; got:\n{text}"
    );
    assert!(
        text.contains("background: terminal default"),
        "the fallback defers every color to the terminal; got:\n{text}"
    );
}
