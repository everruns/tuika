//! tuika's own terminal input decoder, used while the runner follows the color
//! scheme.
//!
//! crossterm's reader decodes keys, mouse, and paste well, but anything it does
//! not recognise in a `CSI ?` frame — the mode 2031 report among them — it
//! treats as *incomplete*, and keeps buffering every byte that follows until a
//! `u` or `c` happens to end the frame. Subscribing to color-scheme reports
//! through crossterm would therefore eat the user's typing. So while a runner
//! follows the scheme it reads the tty itself: [`Decoder`] turns bytes into the
//! same crossterm events crossterm would have produced (so
//! [`translate_event`](crate::translate_event) stays the single place that maps
//! them to tuika's [`Event`](crate::Event)), plus the scheme reports.
//!
//! The decoder is pure and byte-driven so it is unit- and fuzz-testable; the
//! Unix reader around it ([`TtyInput`]) adds `poll(2)`, the lone-Escape timeout,
//! and resize detection.

use crossterm::event::{
    Event as CtEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};

use super::color_scheme::ColorScheme;

/// One decoded input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Decoded {
    /// A key, mouse, paste, or resize event, as crossterm would report it.
    Event(CtEvent),
    /// A mode 2031 color-scheme report.
    ColorScheme(ColorScheme),
}

/// Longest escape sequence the decoder buffers before giving up on it. Real
/// sequences are a few dozen bytes; a stream that never terminates one must
/// not grow the buffer without bound.
const MAX_SEQUENCE: usize = 256;

/// Largest bracketed paste kept. Beyond this the rest of the paste is dropped,
/// which bounds memory against a runaway or hostile stream.
const MAX_PASTE: usize = 16 * 1024 * 1024;

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// Incremental byte-to-event decoder.
#[derive(Debug, Default)]
pub(crate) struct Decoder {
    buffer: Vec<u8>,
}

/// What parsing the front of the buffer produced.
enum Step {
    /// `n` bytes were consumed, yielding an input (or nothing, for a sequence
    /// tuika ignores).
    Consumed(usize, Option<Decoded>),
    /// The front of the buffer is a prefix of a longer sequence.
    Incomplete,
}

impl Decoder {
    /// Feed bytes read from the terminal and collect every input they complete.
    pub(crate) fn feed(&mut self, bytes: &[u8], out: &mut Vec<Decoded>) {
        self.buffer.extend_from_slice(bytes);
        self.drain(out, false);
    }

    /// Whether bytes are waiting for the rest of a sequence.
    pub(crate) fn is_pending(&self) -> bool {
        !self.buffer.is_empty()
    }

    /// The input went quiet with bytes still buffered: resolve them as if no
    /// more were coming. A lone `ESC` becomes the Escape key; an `ESC` prefix
    /// becomes Alt plus the key. A bracketed paste still in flight is kept,
    /// since a large paste can legitimately pause mid-stream.
    pub(crate) fn flush(&mut self, out: &mut Vec<Decoded>) {
        if self.buffer.starts_with(PASTE_START) {
            return;
        }
        self.drain(out, true);
        // Whatever is still unresolved is a truncated sequence; drop it rather
        // than replay its bytes as keystrokes.
        self.buffer.clear();
    }

    fn drain(&mut self, out: &mut Vec<Decoded>, flushing: bool) {
        let mut start = 0;
        while start < self.buffer.len() {
            match parse(&self.buffer[start..], flushing) {
                Step::Consumed(n, decoded) => {
                    start += n.max(1);
                    out.extend(decoded);
                }
                Step::Incomplete => {
                    let pending = &self.buffer[start..];
                    if !pending.starts_with(PASTE_START) && pending.len() > MAX_SEQUENCE {
                        // Not a sequence that will ever end; skip its ESC and
                        // resynchronise on whatever follows.
                        start += 1;
                        continue;
                    }
                    break;
                }
            }
        }
        self.buffer.drain(..start);
    }
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Option<Decoded> {
    Some(Decoded::Event(CtEvent::Key(KeyEvent::new_with_kind(
        code,
        modifiers,
        KeyEventKind::Press,
    ))))
}

/// Parse one input from the front of `b` (non-empty).
fn parse(b: &[u8], flushing: bool) -> Step {
    match b[0] {
        0x1b => parse_escape(b, flushing),
        _ => parse_plain(b, KeyModifiers::NONE),
    }
}

/// A byte that is not the start of an escape sequence: a control key or text.
fn parse_plain(b: &[u8], extra: KeyModifiers) -> Step {
    let ctrl = extra | KeyModifiers::CONTROL;
    let (n, code, modifiers) = match b[0] {
        b'\r' => (1, KeyCode::Enter, extra),
        b'\t' => (1, KeyCode::Tab, extra),
        0x7f | 0x08 => (1, KeyCode::Backspace, extra),
        0x00 => (1, KeyCode::Char(' '), ctrl),
        c @ 0x01..=0x1a => (1, KeyCode::Char(char::from(c - 1 + b'a')), ctrl),
        c @ 0x1c..=0x1f => (1, KeyCode::Char(char::from(c - 0x1c + b'4')), ctrl),
        _ => {
            let width = utf8_width(b[0]);
            if width == 0 {
                // A stray continuation byte or invalid lead: skip it.
                return Step::Consumed(1, None);
            }
            if b.len() < width {
                return Step::Incomplete;
            }
            let Ok(text) = std::str::from_utf8(&b[..width]) else {
                return Step::Consumed(1, None);
            };
            let c = text.chars().next().expect("a decoded scalar");
            // Match crossterm: an uppercase letter carries Shift.
            let modifiers = if c.is_uppercase() {
                extra | KeyModifiers::SHIFT
            } else {
                extra
            };
            (width, KeyCode::Char(c), modifiers)
        }
    };
    Step::Consumed(n, key(code, modifiers))
}

fn utf8_width(lead: u8) -> usize {
    match lead {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => 0,
    }
}

fn parse_escape(b: &[u8], flushing: bool) -> Step {
    let Some(&next) = b.get(1) else {
        return if flushing {
            Step::Consumed(1, key(KeyCode::Esc, KeyModifiers::NONE))
        } else {
            Step::Incomplete
        };
    };
    match next {
        b'[' => parse_csi(b, flushing),
        b'O' => parse_ss3(b, flushing),
        // OSC and DCS replies (to queries another part of the app made) end in
        // ST or BEL; none is input.
        b']' | b'P' | b'_' | b'^' => skip_string(b),
        0x1b => {
            // ESC ESC: Alt+Escape when nothing else follows, otherwise a lone
            // Escape followed by a sequence.
            if b.len() == 2 && flushing {
                Step::Consumed(2, key(KeyCode::Esc, KeyModifiers::ALT))
            } else {
                Step::Consumed(1, key(KeyCode::Esc, KeyModifiers::NONE))
            }
        }
        _ => match parse_plain(&b[1..], KeyModifiers::ALT) {
            Step::Consumed(n, decoded) => Step::Consumed(n + 1, decoded),
            Step::Incomplete => Step::Incomplete,
        },
    }
}

/// Skip an OSC/DCS/APC/PM string through its terminator.
fn skip_string(b: &[u8]) -> Step {
    let mut i = 2;
    while i < b.len() {
        match b[i] {
            0x07 => return Step::Consumed(i + 1, None),
            0x1b if b.get(i + 1) == Some(&b'\\') => return Step::Consumed(i + 2, None),
            0x1b if i + 1 == b.len() => return Step::Incomplete,
            _ => i += 1,
        }
    }
    Step::Incomplete
}

fn parse_ss3(b: &[u8], flushing: bool) -> Step {
    let Some(&c) = b.get(2) else {
        return if flushing {
            // `ESC O` with nothing after it was Alt+Shift+O.
            Step::Consumed(
                2,
                key(KeyCode::Char('O'), KeyModifiers::ALT | KeyModifiers::SHIFT),
            )
        } else {
            Step::Incomplete
        };
    };
    let code = match c {
        b'A' => Some(KeyCode::Up),
        b'B' => Some(KeyCode::Down),
        b'C' => Some(KeyCode::Right),
        b'D' => Some(KeyCode::Left),
        b'H' => Some(KeyCode::Home),
        b'F' => Some(KeyCode::End),
        b'P'..=b'S' => Some(KeyCode::F(c - b'P' + 1)),
        b'M' => Some(KeyCode::Enter),
        _ => None,
    };
    Step::Consumed(3, code.and_then(|code| key(code, KeyModifiers::NONE)))
}

fn parse_csi(b: &[u8], flushing: bool) -> Step {
    if b.starts_with(PASTE_START) {
        return parse_paste(b);
    }
    if PASTE_START.starts_with(b) && !flushing {
        return Step::Incomplete;
    }
    // Legacy X10 mouse: `CSI M` plus three raw bytes, which may be anything.
    if b.get(2) == Some(&b'M') {
        if b.len() < 6 {
            return if flushing {
                Step::Consumed(b.len(), None)
            } else {
                Step::Incomplete
            };
        }
        let decoded = x10_mouse(b[3], b[4], b[5]);
        return Step::Consumed(6, decoded);
    }
    // Parameters 0x30–0x3F, intermediates 0x20–0x2F, then one final byte.
    let mut i = 2;
    while i < b.len() && (0x20..=0x3f).contains(&b[i]) {
        i += 1;
    }
    let Some(&last) = b.get(i) else {
        return if flushing {
            Step::Consumed(b.len(), None)
        } else {
            Step::Incomplete
        };
    };
    if !(0x40..=0x7e).contains(&last) {
        // Not a well-formed CSI: drop the introducer and resynchronise.
        return Step::Consumed(2, None);
    }
    let frame = &b[..=i];
    let params = &b[2..i];
    Step::Consumed(i + 1, decode_csi(frame, params, last))
}

fn parse_paste(b: &[u8]) -> Step {
    let body = &b[PASTE_START.len()..];
    let Some(end) = body.windows(PASTE_END.len()).position(|w| w == PASTE_END) else {
        return Step::Incomplete;
    };
    let mut text = String::from_utf8_lossy(&body[..end.min(MAX_PASTE)]).into_owned();
    // Terminals send line breaks inside a paste as CR.
    text = text.replace("\r\n", "\n").replace('\r', "\n");
    Step::Consumed(
        PASTE_START.len() + end + PASTE_END.len(),
        Some(Decoded::Event(CtEvent::Paste(text))),
    )
}

/// Split `params` on `;` into numbers, each optionally with `:`-separated
/// sub-parameters; empty fields read as `None`.
fn fields(params: &[u8]) -> Vec<Vec<Option<u32>>> {
    params
        .split(|&c| c == b';')
        .map(|field| {
            field
                .split(|&c| c == b':')
                .map(|n| {
                    std::str::from_utf8(n)
                        .ok()
                        .filter(|s| !s.is_empty())
                        .and_then(|s| s.parse().ok())
                })
                .collect()
        })
        .collect()
}

/// xterm/kitty modifier parameter (1 + bitmask) to crossterm modifiers.
fn modifiers(param: Option<u32>) -> KeyModifiers {
    let bits = param.unwrap_or(1).saturating_sub(1);
    let mut out = KeyModifiers::NONE;
    if bits & 1 != 0 {
        out |= KeyModifiers::SHIFT;
    }
    if bits & 2 != 0 {
        out |= KeyModifiers::ALT;
    }
    if bits & 4 != 0 {
        out |= KeyModifiers::CONTROL;
    }
    if bits & 8 != 0 {
        out |= KeyModifiers::SUPER;
    }
    if bits & 16 != 0 {
        out |= KeyModifiers::HYPER;
    }
    if bits & 32 != 0 {
        out |= KeyModifiers::META;
    }
    out
}

fn decode_csi(frame: &[u8], params: &[u8], last: u8) -> Option<Decoded> {
    if let Some(private) = params.strip_prefix(b"?") {
        // Replies, not input: DA1 (`c`), kitty keyboard flags (`u`), and the
        // color-scheme report this decoder exists for.
        let _ = private;
        return ColorScheme::parse_report(frame).map(Decoded::ColorScheme);
    }
    if let Some(sgr) = params.strip_prefix(b"<") {
        return sgr_mouse(sgr, last == b'M');
    }
    let fields = fields(params);
    let first = |i: usize| fields.get(i).and_then(|f| f.first().copied().flatten());
    match last {
        b'A' | b'B' | b'C' | b'D' | b'H' | b'F' | b'E' => {
            let code = match last {
                b'A' => KeyCode::Up,
                b'B' => KeyCode::Down,
                b'C' => KeyCode::Right,
                b'D' => KeyCode::Left,
                b'H' => KeyCode::Home,
                b'F' => KeyCode::End,
                _ => KeyCode::KeypadBegin,
            };
            let (mods, kind) = mods_and_kind(fields.get(1));
            press(code, mods, kind)
        }
        b'P' | b'Q' | b'S' => {
            let (mods, kind) = mods_and_kind(fields.get(1));
            press(KeyCode::F(last - b'P' + 1), mods, kind)
        }
        b'Z' => key(KeyCode::BackTab, KeyModifiers::SHIFT),
        b'~' => {
            let code = match first(0)? {
                1 | 7 => KeyCode::Home,
                2 => KeyCode::Insert,
                3 => KeyCode::Delete,
                4 | 8 => KeyCode::End,
                5 => KeyCode::PageUp,
                6 => KeyCode::PageDown,
                n @ 11..=15 => KeyCode::F((n - 10) as u8),
                n @ 17..=21 => KeyCode::F((n - 11) as u8),
                n @ 23..=24 => KeyCode::F((n - 12) as u8),
                _ => return None,
            };
            let (mods, kind) = mods_and_kind(fields.get(1));
            press(code, mods, kind)
        }
        b'u' => kitty_key(&fields),
        // Focus in/out and anything else tuika does not model.
        _ => None,
    }
}

/// Modifiers and event kind from a `mods[:event]` field.
fn mods_and_kind(field: Option<&Vec<Option<u32>>>) -> (KeyModifiers, KeyEventKind) {
    let Some(field) = field else {
        return (KeyModifiers::NONE, KeyEventKind::Press);
    };
    let mods = modifiers(field.first().copied().flatten());
    let kind = match field.get(1).copied().flatten() {
        Some(2) => KeyEventKind::Repeat,
        Some(3) => KeyEventKind::Release,
        _ => KeyEventKind::Press,
    };
    (mods, kind)
}

fn press(code: KeyCode, mods: KeyModifiers, kind: KeyEventKind) -> Option<Decoded> {
    Some(Decoded::Event(CtEvent::Key(KeyEvent::new_with_kind(
        code, mods, kind,
    ))))
}

/// A kitty keyboard protocol key: `CSI code[:shifted[:base]] ; mods[:event] u`.
fn kitty_key(fields: &[Vec<Option<u32>>]) -> Option<Decoded> {
    let codes = fields.first()?;
    let code = codes.first().copied().flatten()?;
    let shifted = codes.get(1).copied().flatten();
    let (mut mods, kind) = mods_and_kind(fields.get(1));
    let key_code = match code {
        13 => KeyCode::Enter,
        27 => KeyCode::Esc,
        9 if mods.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
        9 => KeyCode::Tab,
        127 | 8 => KeyCode::Backspace,
        // Keypad Enter, and the private-use functional keys tuika does not model.
        57414 => KeyCode::Enter,
        57344..=63743 => return None,
        _ => {
            let base = char::from_u32(code)?;
            let c = if mods.contains(KeyModifiers::SHIFT) {
                // Prefer the terminal's own shifted key (layout-correct), and
                // fall back to upper-casing.
                shifted
                    .and_then(char::from_u32)
                    .unwrap_or_else(|| base.to_uppercase().next().unwrap_or(base))
            } else {
                base
            };
            KeyCode::Char(c)
        }
    };
    if matches!(key_code, KeyCode::BackTab) {
        mods |= KeyModifiers::SHIFT;
    }
    press(key_code, mods, kind)
}

fn mouse_modifiers(cb: u32) -> KeyModifiers {
    let mut mods = KeyModifiers::NONE;
    if cb & 4 != 0 {
        mods |= KeyModifiers::SHIFT;
    }
    if cb & 8 != 0 {
        mods |= KeyModifiers::ALT;
    }
    if cb & 16 != 0 {
        mods |= KeyModifiers::CONTROL;
    }
    mods
}

fn mouse_kind(cb: u32, pressed: bool) -> Option<MouseEventKind> {
    let button = match cb & 0b11 {
        0 => Some(MouseButton::Left),
        1 => Some(MouseButton::Middle),
        2 => Some(MouseButton::Right),
        _ => None,
    };
    if cb & 64 != 0 {
        return Some(match cb & 0b11 {
            0 => MouseEventKind::ScrollUp,
            1 => MouseEventKind::ScrollDown,
            2 => MouseEventKind::ScrollLeft,
            _ => MouseEventKind::ScrollRight,
        });
    }
    Some(if cb & 32 != 0 {
        match button {
            Some(button) => MouseEventKind::Drag(button),
            None => MouseEventKind::Moved,
        }
    } else if pressed {
        MouseEventKind::Down(button?)
    } else {
        // SGR names the released button; X10 does not (button 3), and Left is
        // what crossterm reports for that case.
        MouseEventKind::Up(button.unwrap_or(MouseButton::Left))
    })
}

fn mouse(cb: u32, column: u32, row: u32, pressed: bool) -> Option<Decoded> {
    let kind = mouse_kind(cb, pressed)?;
    Some(Decoded::Event(CtEvent::Mouse(MouseEvent {
        kind,
        column: u16::try_from(column.saturating_sub(1)).unwrap_or(u16::MAX),
        row: u16::try_from(row.saturating_sub(1)).unwrap_or(u16::MAX),
        modifiers: mouse_modifiers(cb),
    })))
}

/// SGR mouse: `CSI < cb ; x ; y M` (press/motion) or `m` (release).
fn sgr_mouse(params: &[u8], pressed: bool) -> Option<Decoded> {
    let fields = fields(params);
    let n = |i: usize| fields.get(i).and_then(|f| f.first().copied().flatten());
    mouse(n(0)?, n(1)?, n(2)?, pressed)
}

/// X10 mouse: each byte is the value plus 32.
fn x10_mouse(cb: u8, x: u8, y: u8) -> Option<Decoded> {
    let cb = u32::from(cb.saturating_sub(32));
    let pressed = cb & 0b11 != 3 || cb & 64 != 0;
    mouse(
        cb,
        u32::from(x.saturating_sub(32)),
        u32::from(y.saturating_sub(32)),
        pressed,
    )
}

#[cfg(unix)]
pub(crate) use unix::TtyInput;

#[cfg(unix)]
mod unix {
    use std::collections::VecDeque;
    use std::fs::File;
    use std::io::{self, IsTerminal, Read};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::time::{Duration, Instant};

    use super::{Decoded, Decoder};
    use crossterm::event::Event as CtEvent;

    /// How long a lone `ESC` (or another sequence prefix) waits for the rest
    /// before it is taken at face value. Terminals send a sequence in one
    /// write, so this only needs to cover scheduling jitter.
    const ESCAPE_TIMEOUT: Duration = Duration::from_millis(30);

    /// Resize is polled rather than signalled: installing a SIGWINCH handler
    /// is process-global, and a size check is one ioctl.
    const RESIZE_POLL: Duration = Duration::from_millis(50);

    /// The terminal's input, read and decoded by tuika.
    pub(crate) struct TtyInput {
        file: std::mem::ManuallyDrop<File>,
        /// `/dev/tty` opened because stdin is not a terminal; closed on drop.
        owned: bool,
        decoder: Decoder,
        ready: VecDeque<Decoded>,
        size: Option<(u16, u16)>,
        /// When buffered sequence bytes stop waiting for more.
        pending_since: Option<Instant>,
    }

    impl TtyInput {
        /// Open the terminal the way crossterm does: stdin when it is a tty,
        /// otherwise `/dev/tty`.
        pub(crate) fn open() -> io::Result<Self> {
            let (file, owned) = if io::stdin().is_terminal() {
                // Borrow fd 0 without taking ownership: ManuallyDrop never
                // closes it.
                (unsafe { File::from_raw_fd(0) }, false)
            } else {
                (File::open("/dev/tty")?, true)
            };
            Ok(Self {
                file: std::mem::ManuallyDrop::new(file),
                owned,
                decoder: Decoder::default(),
                ready: VecDeque::new(),
                size: crossterm::terminal::size().ok(),
                pending_since: None,
            })
        }

        /// Wait up to `timeout` for the next input.
        pub(crate) fn poll(&mut self, timeout: Duration) -> io::Result<Option<Decoded>> {
            let deadline = Instant::now() + timeout;
            loop {
                if let Some(decoded) = self.ready.pop_front() {
                    return Ok(Some(decoded));
                }
                if let Ok(size) = crossterm::terminal::size()
                    && self.size != Some(size)
                {
                    self.size = Some(size);
                    return Ok(Some(Decoded::Event(CtEvent::Resize(size.0, size.1))));
                }
                let now = Instant::now();
                if let Some(since) = self.pending_since
                    && now.saturating_duration_since(since) >= ESCAPE_TIMEOUT
                {
                    let mut out = Vec::new();
                    self.decoder.flush(&mut out);
                    self.ready.extend(out);
                    self.pending_since = self.decoder.is_pending().then_some(now);
                    continue;
                }
                if now >= deadline {
                    return Ok(None);
                }
                let mut wait = deadline.saturating_duration_since(now).min(RESIZE_POLL);
                if let Some(since) = self.pending_since {
                    wait = wait.min((since + ESCAPE_TIMEOUT).saturating_duration_since(now));
                }
                if self.readable(wait)? {
                    self.read_available()?;
                }
            }
        }

        fn readable(&self, wait: Duration) -> io::Result<bool> {
            let mut fds = libc::pollfd {
                fd: self.file.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let millis = libc::c_int::try_from(wait.as_millis()).unwrap_or(libc::c_int::MAX);
            // SAFETY: one valid pollfd for the duration of the call.
            let n = unsafe { libc::poll(&mut fds, 1, millis) };
            if n < 0 {
                let error = io::Error::last_os_error();
                return if error.kind() == io::ErrorKind::Interrupted {
                    Ok(false)
                } else {
                    Err(error)
                };
            }
            Ok(n > 0 && fds.revents & (libc::POLLIN | libc::POLLHUP) != 0)
        }

        fn read_available(&mut self) -> io::Result<()> {
            let mut buf = [0u8; 4096];
            let n = match (&*self.file).read(&mut buf) {
                Ok(n) => n,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return Ok(()),
                Err(error) => return Err(error),
            };
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "terminal input closed",
                ));
            }
            let mut out = Vec::new();
            self.decoder.feed(&buf[..n], &mut out);
            self.ready.extend(out);
            self.pending_since = self
                .decoder
                .is_pending()
                .then(|| self.pending_since.unwrap_or_else(Instant::now));
            Ok(())
        }
    }

    impl Drop for TtyInput {
        fn drop(&mut self) {
            if self.owned {
                // SAFETY: an owned `/dev/tty` handle is dropped exactly once.
                unsafe { std::mem::ManuallyDrop::drop(&mut self.file) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(bytes: &[u8]) -> Vec<Decoded> {
        let mut decoder = Decoder::default();
        let mut out = Vec::new();
        decoder.feed(bytes, &mut out);
        decoder.flush(&mut out);
        out
    }

    fn keys(bytes: &[u8]) -> Vec<(KeyCode, KeyModifiers)> {
        decode(bytes)
            .into_iter()
            .filter_map(|d| match d {
                Decoded::Event(CtEvent::Key(k)) if k.kind != KeyEventKind::Release => {
                    Some((k.code, k.modifiers))
                }
                _ => None,
            })
            .collect()
    }

    const NONE: KeyModifiers = KeyModifiers::NONE;

    #[test]
    fn color_scheme_reports_are_decoded_between_keys() {
        assert_eq!(
            decode(b"a\x1b[?997;1nb\x1b[?997;2n"),
            vec![
                key(KeyCode::Char('a'), NONE).unwrap(),
                Decoded::ColorScheme(ColorScheme::Dark),
                key(KeyCode::Char('b'), NONE).unwrap(),
                Decoded::ColorScheme(ColorScheme::Light),
            ]
        );
    }

    #[test]
    fn the_report_does_not_swallow_following_keys() {
        // The bug this decoder exists for: crossterm holds every key after an
        // unrecognised `CSI ?` frame until a `u` or `c` arrives.
        let decoded = keys(b"\x1b[?997;1nhello");
        assert_eq!(decoded.len(), 5);
        assert_eq!(decoded[4], (KeyCode::Char('o'), NONE));
    }

    #[test]
    fn replies_are_not_input() {
        assert!(decode(b"\x1b[?62;22c\x1b[?7u\x1b]11;rgb:0000/0000/0000\x1b\\\x1b[I").is_empty());
    }

    #[test]
    fn text_and_control_keys() {
        assert_eq!(
            keys("aZй\r\t\x7f\x03".as_bytes()),
            vec![
                (KeyCode::Char('a'), NONE),
                (KeyCode::Char('Z'), KeyModifiers::SHIFT),
                (KeyCode::Char('й'), NONE),
                (KeyCode::Enter, NONE),
                (KeyCode::Tab, NONE),
                (KeyCode::Backspace, NONE),
                (KeyCode::Char('c'), KeyModifiers::CONTROL),
            ]
        );
    }

    #[test]
    fn escape_alone_and_as_alt_prefix() {
        assert_eq!(keys(b"\x1b"), vec![(KeyCode::Esc, NONE)]);
        assert_eq!(
            keys(b"\x1bx"),
            vec![(KeyCode::Char('x'), KeyModifiers::ALT)]
        );
        // Without a flush a lone ESC waits for the rest of a sequence.
        let mut decoder = Decoder::default();
        let mut out = Vec::new();
        decoder.feed(b"\x1b", &mut out);
        assert!(out.is_empty() && decoder.is_pending());
        decoder.feed(b"[A", &mut out);
        assert_eq!(out, vec![key(KeyCode::Up, NONE).unwrap()]);
    }

    #[test]
    fn legacy_navigation_with_modifiers() {
        assert_eq!(
            keys(b"\x1b[A\x1bOB\x1b[1;5C\x1b[3~\x1b[5;2~\x1b[H\x1b[Z"),
            vec![
                (KeyCode::Up, NONE),
                (KeyCode::Down, NONE),
                (KeyCode::Right, KeyModifiers::CONTROL),
                (KeyCode::Delete, NONE),
                (KeyCode::PageUp, KeyModifiers::SHIFT),
                (KeyCode::Home, NONE),
                (KeyCode::BackTab, KeyModifiers::SHIFT),
            ]
        );
    }

    #[test]
    fn kitty_keys_report_modifiers_and_skip_releases() {
        assert_eq!(
            keys(b"\x1b[13;2u\x1b[97;5u\x1b[97:65;2u\x1b[27u\x1b[97;1:3u\x1b[9;2u"),
            vec![
                (KeyCode::Enter, KeyModifiers::SHIFT),
                (KeyCode::Char('a'), KeyModifiers::CONTROL),
                (KeyCode::Char('A'), KeyModifiers::SHIFT),
                (KeyCode::Esc, NONE),
                (KeyCode::BackTab, KeyModifiers::SHIFT),
            ]
        );
    }

    #[test]
    fn sgr_mouse() {
        let decoded = decode(b"\x1b[<0;10;5M\x1b[<0;10;5m\x1b[<32;11;5M\x1b[<65;1;1M\x1b[<35;2;2M");
        let kinds: Vec<_> = decoded
            .iter()
            .map(|d| match d {
                Decoded::Event(CtEvent::Mouse(m)) => (m.kind, m.column, m.row),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                (MouseEventKind::Down(MouseButton::Left), 9, 4),
                (MouseEventKind::Up(MouseButton::Left), 9, 4),
                (MouseEventKind::Drag(MouseButton::Left), 10, 4),
                (MouseEventKind::ScrollDown, 0, 0),
                (MouseEventKind::Moved, 1, 1),
            ]
        );
    }

    #[test]
    fn bracketed_paste_spans_reads() {
        let mut decoder = Decoder::default();
        let mut out = Vec::new();
        decoder.feed(b"\x1b[200~line one\r", &mut out);
        decoder.flush(&mut out); // a pause mid-paste keeps it
        assert!(out.is_empty());
        decoder.feed(b"line \x1b[A two\x1b[201~x", &mut out);
        assert_eq!(
            out,
            vec![
                Decoded::Event(CtEvent::Paste("line one\nline \x1b[A two".into())),
                key(KeyCode::Char('x'), NONE).unwrap(),
            ]
        );
    }

    #[test]
    fn split_utf8_and_sequences_reassemble() {
        let bytes = "é\x1b[?997;2n\x1b[1;5D".as_bytes();
        for split in 0..bytes.len() {
            let mut decoder = Decoder::default();
            let mut out = Vec::new();
            decoder.feed(&bytes[..split], &mut out);
            decoder.feed(&bytes[split..], &mut out);
            assert_eq!(out.len(), 3, "split at {split}: {out:?}");
        }
    }

    #[test]
    fn garbage_never_grows_the_buffer_unbounded() {
        let mut decoder = Decoder::default();
        let mut out = Vec::new();
        let mut junk = b"\x1b[".to_vec();
        junk.extend(std::iter::repeat_n(b'1', 10_000));
        decoder.feed(&junk, &mut out);
        assert!(decoder.buffer.len() <= MAX_SEQUENCE + 1);
        decoder.feed(b"q", &mut out);
        decoder.flush(&mut out);
        assert!(!decoder.is_pending());
    }

    #[test]
    fn decoded_keys_translate_like_crossterm_events() {
        let events: Vec<_> = decode(b"\x1b[97;5u")
            .into_iter()
            .filter_map(|d| match d {
                Decoded::Event(e) => crate::translate_event(e),
                Decoded::ColorScheme(_) => None,
            })
            .collect();
        assert_eq!(
            events,
            vec![crate::Event::Key(crate::Key {
                code: crate::KeyCode::Char('a'),
                ctrl: true,
                alt: false,
                shift: false,
            })]
        );
    }
}
