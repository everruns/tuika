//! Desktop notifications and focus reporting outside the cell grid.
//!
//! A notification is how a program that finished, failed, or needs an answer
//! reaches a user who switched away. The terminal turns the escape into a
//! system notification; a terminal that knows none of them ignores the unknown
//! OSC, so the worst case is silence. Keep an in-app indication as the portable
//! fallback. Focus reporting delivers ordinary terminal focus events;
//! [`FocusReporting`] disables it on every exit path.
//!
//! Three notification protocols, because terminals disagree:
//!
//! | Protocol | Sequence | Spoken by |
//! | --- | --- | --- |
//! | [`Protocol::Osc99`] | `OSC 99 ; metadata ; payload ST` | kitty, foot, Ghostty |
//! | [`Protocol::Osc777`] | `OSC 777 ; notify ; title ; body ST` | rxvt-unicode, WezTerm, Konsole |
//! | [`Protocol::Osc9`] | `OSC 9 ; text ST` | iTerm2, Windows Terminal |
//!
//! Hosts can choose a protocol themselves or ask [`Protocol::detect`], which
//! returns `None` for a terminal it cannot place rather than guessing: an
//! unsolicited popup is the one out-of-band effect a user notices when it is
//! wrong. A [`Notifier`] adds the rate limit every notifying program needs, and
//! [`Runner::notifications`](crate::Runner::notifications) queues them on the
//! runner, which detects and rate-limits for you.
//!
//! Text is the host's and may be untrusted. Each field is limited to 256
//! Unicode scalar values and loses its control characters. OSC 99 carries it
//! base64-encoded; for OSC 777 and OSC 9, which carry plain text, `;` (their
//! field separator, and the start of OSC 9 subcommands such as `9;4` progress)
//! becomes `,`.
//!
//! ```
//! use tuika::term::attention::{Notification, Protocol};
//!
//! let done = Notification::new("Build finished").body("12 crates, 0 warnings");
//! assert_eq!(
//!     done.encode(Protocol::Osc777),
//!     "\x1b]777;notify;Build finished;12 crates, 0 warnings\x1b\\"
//! );
//! ```

use std::io::{self, Write};
use std::time::{Duration, Instant};

/// Which notification escape a terminal understands.
///
/// Non-exhaustive: terminals keep adding protocols.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Protocol {
    /// OSC 9, supported by iTerm2, Windows Terminal, and compatible terminals.
    /// One line of text: title and body joined by `: `.
    Osc9,
    /// OSC 777 `notify`, supported by rxvt-unicode, WezTerm, Konsole, and
    /// compatible desktop terminals. Title and body.
    Osc777,
    /// Kitty's OSC 99, also spoken by foot and Ghostty. Title and body as
    /// separate base64 parts, with an [`Urgency`].
    Osc99,
}

impl Protocol {
    /// Pick a protocol from the process environment, or `None` for a terminal
    /// that cannot be placed. Inside tmux or GNU Screen the answer is `None`:
    /// neither forwards these escapes to the outer terminal.
    pub fn detect() -> Option<Self> {
        let env = |k: &str| std::env::var(k).ok();
        Self::detect_from(
            env("TERM").as_deref(),
            env("TERM_PROGRAM").as_deref(),
            env("KITTY_WINDOW_ID").as_deref(),
            env("WT_SESSION").is_some(),
            env("TMUX").is_some() || env("STY").is_some(),
        )
    }

    /// The pure core of [`detect`](Self::detect).
    pub fn detect_from(
        term: Option<&str>,
        term_program: Option<&str>,
        kitty_window_id: Option<&str>,
        windows_terminal: bool,
        multiplexed: bool,
    ) -> Option<Self> {
        if multiplexed {
            return None;
        }
        let term = term.unwrap_or_default();
        let program = term_program.unwrap_or_default().to_ascii_lowercase();
        if kitty_window_id.is_some()
            || term.contains("kitty")
            || term.contains("foot")
            || program == "ghostty"
        {
            Some(Self::Osc99)
        } else if program == "wezterm" || term.starts_with("rxvt") || program == "konsole" {
            Some(Self::Osc777)
        } else if program == "iterm.app" || windows_terminal {
            Some(Self::Osc9)
        } else {
            None
        }
    }
}

/// How loudly to notify. Only [`Protocol::Osc99`] carries it (`u=`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Urgency {
    /// Informational.
    Low,
    /// The default.
    #[default]
    Normal,
    /// Needs attention now: a prompt waiting on the user.
    Critical,
}

/// One desktop notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    title: String,
    body: Option<String>,
    urgency: Urgency,
}

impl Notification {
    /// A notification with `title` and no body.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: None,
            urgency: Urgency::Normal,
        }
    }

    /// Secondary text.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// How loudly to notify.
    pub fn urgency(mut self, urgency: Urgency) -> Self {
        self.urgency = urgency;
        self
    }

    /// The title.
    pub fn title_text(&self) -> &str {
        &self.title
    }

    /// The body, if any.
    pub fn body_text(&self) -> Option<&str> {
        self.body.as_deref()
    }

    /// Encode this notification in `protocol`, bounded and control-safe.
    pub fn encode(&self, protocol: Protocol) -> String {
        match protocol {
            Protocol::Osc9 => {
                let title = field(&self.title);
                match &self.body {
                    Some(body) => format!("\x1b]9;{title}: {}\x1b\\", field(body)),
                    None => format!("\x1b]9;{title}\x1b\\"),
                }
            }
            Protocol::Osc777 => format!(
                "\x1b]777;notify;{};{}\x1b\\",
                field(&self.title),
                field(self.body.as_deref().unwrap_or_default())
            ),
            Protocol::Osc99 => {
                let urgency = match self.urgency {
                    Urgency::Low => 0,
                    Urgency::Normal => 1,
                    Urgency::Critical => 2,
                };
                let body = self.body.as_deref().map(text);
                // `d=0` holds the notification open until the body part
                // arrives; `e=1` marks a base64 payload, so host text can
                // never terminate the sequence.
                let mut out = format!(
                    "\x1b]99;i=tuika:d={}:u={urgency}:e=1;{}\x1b\\",
                    u8::from(body.is_none()),
                    base64(text(&self.title).as_bytes())
                );
                if let Some(body) = body {
                    out.push_str(&format!(
                        "\x1b]99;i=tuika:p=body:e=1;{}\x1b\\",
                        base64(body.as_bytes())
                    ));
                }
                out
            }
        }
    }

    /// Write and flush this notification in `protocol`.
    pub fn write(&self, writer: &mut impl Write, protocol: Protocol) -> io::Result<()> {
        writer.write_all(self.encode(protocol).as_bytes())?;
        writer.flush()
    }
}

/// Encode bounded notification text without terminal control characters.
///
/// Each field is limited to 256 Unicode scalar values. Semicolons are replaced
/// so untrusted titles cannot introduce protocol fields or OSC 9 subcommands.
/// The same as `Notification::new(title).body(body).encode(protocol)`.
pub fn encode(title: &str, body: &str, protocol: Protocol) -> String {
    Notification::new(title).body(body).encode(protocol)
}

/// Bounded text: at most 256 scalar values, line breaks and tabs as spaces,
/// other control characters dropped.
fn text(text: &str) -> String {
    text.chars()
        .take(256)
        .filter_map(|ch| {
            if matches!(ch, '\n' | '\r' | '\t') {
                Some(' ')
            } else if ch.is_control() {
                None
            } else {
                Some(ch)
            }
        })
        .collect()
}

/// A plain-text protocol field: [`text`], with `;` as `,`.
fn field(raw: &str) -> String {
    text(raw).replace(';', ",")
}

fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= chunk.len() {
                ALPHABET[((n >> shift) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

/// Write and flush one notification. No permission or capability is negotiated.
pub fn write(
    writer: &mut impl Write,
    title: &str,
    body: &str,
    protocol: Protocol,
) -> io::Result<()> {
    Notification::new(title).body(body).write(writer, protocol)
}

/// Sends notifications in one protocol, at most one per interval.
///
/// A program's state can change several times a second; a user should get one
/// popup, not a stack. Within [`min_interval`](Self::with_min_interval) of the
/// last notification sent, a new one is held, replacing any already held,
/// and goes out from [`poll`](Self::poll) once the interval has passed — so
/// the *latest* news arrives, late by at most the interval. An exact repeat of
/// the last notification sent is dropped.
#[derive(Debug, Clone)]
pub struct Notifier {
    protocol: Protocol,
    min_interval: Duration,
    last: Option<(Instant, Notification)>,
    held: Option<Notification>,
}

impl Notifier {
    /// A notifier for `protocol`, with a 2-second interval.
    pub fn new(protocol: Protocol) -> Self {
        Self {
            protocol,
            min_interval: Duration::from_secs(2),
            last: None,
            held: None,
        }
    }

    /// Set the minimum time between notifications.
    pub fn with_min_interval(mut self, interval: Duration) -> Self {
        self.min_interval = interval;
        self
    }

    /// Send `notification` now if the interval allows, else hold it for
    /// [`poll`](Self::poll). Returns whether it was written.
    pub fn notify(
        &mut self,
        out: &mut impl Write,
        notification: Notification,
        now: Instant,
    ) -> io::Result<bool> {
        if let Some((at, last)) = &self.last {
            let within = now.saturating_duration_since(*at) < self.min_interval;
            if within && *last == notification {
                self.held = None;
                return Ok(false);
            }
            if within {
                self.held = Some(notification);
                return Ok(false);
            }
        }
        self.send(out, notification, now)?;
        Ok(true)
    }

    /// Send a held notification once the interval has passed. Returns whether
    /// it wrote.
    pub fn poll(&mut self, out: &mut impl Write, now: Instant) -> io::Result<bool> {
        let due = self
            .last
            .as_ref()
            .is_none_or(|(at, _)| now.saturating_duration_since(*at) >= self.min_interval);
        match self.held.take() {
            Some(notification) if due => {
                self.send(out, notification, now)?;
                Ok(true)
            }
            held => {
                self.held = held;
                Ok(false)
            }
        }
    }

    /// Whether a notification is waiting for the interval.
    pub fn has_held(&self) -> bool {
        self.held.is_some()
    }

    fn send(
        &mut self,
        out: &mut impl Write,
        notification: Notification,
        now: Instant,
    ) -> io::Result<()> {
        notification.write(out, self.protocol)?;
        self.last = Some((now, notification));
        Ok(())
    }
}

/// A handle for queueing notifications on a runner — see
/// [`Runner::notifications`](crate::Runner::notifications). Cloneable and
/// `Send`, so a worker thread can notify; sending wakes the runner, which
/// delivers through its [`Notifier`].
#[derive(Clone)]
pub struct Notifications {
    queue: std::sync::Arc<std::sync::Mutex<Vec<Notification>>>,
    wake: crate::live::RedrawHandle,
}

impl std::fmt::Debug for Notifications {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notifications").finish_non_exhaustive()
    }
}

impl Notifications {
    pub(crate) fn new(wake: crate::live::RedrawHandle) -> Self {
        Self {
            queue: Default::default(),
            wake,
        }
    }

    /// Queue `notification`. The runner sends it on its next turn, subject to
    /// the rate limit; a runner without a notification protocol drops it.
    pub fn send(&self, notification: Notification) {
        self.queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(notification);
        self.wake.request();
    }

    pub(crate) fn drain(&self) -> Vec<Notification> {
        std::mem::take(&mut *self.queue.lock().unwrap_or_else(|error| error.into_inner()))
    }
}

/// Encode the terminal's focus-reporting switch (`CSI ? 1004 h/l`).
pub fn encode_focus_reporting(enabled: bool) -> &'static str {
    if enabled {
        "\x1b[?1004h"
    } else {
        "\x1b[?1004l"
    }
}

/// Own an opt-in focus-reporting lifetime, restoring it even on early returns.
///
/// Reporting is not reference-counted. One host scope owns this mode, as it
/// owns raw mode and alternate-screen entry. Enabling does not reveal current
/// focus immediately; hosts should treat focus as unknown until an event arrives.
pub struct FocusReporting<W: Write> {
    writer: W,
}

impl<W: Write> FocusReporting<W> {
    /// Enable focus events on this writer and take responsibility for cleanup.
    pub fn enable(writer: W) -> io::Result<Self> {
        let mut reporting = Self { writer };
        reporting
            .writer
            .write_all(encode_focus_reporting(true).as_bytes())?;
        reporting.writer.flush()?;
        Ok(reporting)
    }
}

impl<W: Write> Drop for FocusReporting<W> {
    fn drop(&mut self) {
        let _ = self
            .writer
            .write_all(encode_focus_reporting(false).as_bytes());
        let _ = self.writer.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_both_protocols_and_flushes_the_writer() {
        assert_eq!(
            encode("Build", "Ready", Protocol::Osc9),
            "\x1b]9;Build: Ready\x1b\\"
        );
        let mut bytes = Vec::new();
        write(&mut bytes, "Build", "Ready", Protocol::Osc777).unwrap();
        assert_eq!(bytes, b"\x1b]777;notify;Build;Ready\x1b\\");
    }

    #[test]
    fn untrusted_text_cannot_terminate_or_extend_a_notification() {
        let encoded = encode(
            "4;1\x07\x1b]52;c;secret",
            "a\n\r\tb\u{009c}\x1b\\",
            Protocol::Osc9,
        );
        assert_eq!(encoded, "\x1b]9;4,1]52,c,secret: a   b\\\x1b\\");
        assert_eq!(encoded.matches('\x1b').count(), 2);
        assert_eq!(encoded.matches(';').count(), 1);
        assert!(
            encode(
                &"界".repeat(100_000),
                &"🙂".repeat(100_000),
                Protocol::Osc777
            )
            .len()
                < 2048
        );
    }

    #[test]
    fn scope_restores_focus_reporting_on_early_return() {
        fn run(writer: &mut Vec<u8>) -> io::Result<()> {
            let _reporting = FocusReporting::enable(writer)?;
            Err(io::Error::other("host failure"))
        }
        let mut bytes = Vec::new();
        assert!(run(&mut bytes).is_err());
        assert_eq!(bytes, b"\x1b[?1004h\x1b[?1004l");
    }

    #[test]
    fn a_failed_enable_flush_still_attempts_cleanup() {
        struct FailingFlush<'a> {
            bytes: &'a mut Vec<u8>,
            first: bool,
        }
        impl Write for FailingFlush<'_> {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                if std::mem::take(&mut self.first) {
                    Err(io::Error::other("flush failed"))
                } else {
                    Ok(())
                }
            }
        }
        let mut bytes = Vec::new();
        assert!(
            FocusReporting::enable(FailingFlush {
                bytes: &mut bytes,
                first: true
            })
            .is_err()
        );
        assert_eq!(bytes, b"\x1b[?1004h\x1b[?1004l");
    }

    #[test]
    fn osc99_carries_base64_parts() {
        let n = Notification::new("Done")
            .body("ok;\x1b]x")
            .urgency(Urgency::Critical);
        assert_eq!(
            n.encode(Protocol::Osc99),
            "\x1b]99;i=tuika:d=0:u=2:e=1;RG9uZQ==\x1b\\\x1b]99;i=tuika:p=body:e=1;b2s7XXg=\x1b\\"
        );
        assert_eq!(
            Notification::new("Hi").encode(Protocol::Osc99),
            "\x1b]99;i=tuika:d=1:u=1:e=1;SGk=\x1b\\"
        );
    }

    #[test]
    fn a_title_alone_has_no_separator() {
        assert_eq!(
            Notification::new("4;3;0").encode(Protocol::Osc9),
            "\x1b]9;4,3,0\x1b\\",
            "never an OSC 9 subcommand"
        );
        assert_eq!(
            Notification::new("Build").encode(Protocol::Osc777),
            "\x1b]777;notify;Build;\x1b\\"
        );
    }

    #[test]
    fn detection_places_known_terminals_only() {
        let d =
            |term, program, kitty, wt, mux| Protocol::detect_from(term, program, kitty, wt, mux);
        assert_eq!(
            d(Some("xterm-kitty"), None, None, false, false),
            Some(Protocol::Osc99)
        );
        assert_eq!(
            d(None, None, Some("1"), false, false),
            Some(Protocol::Osc99)
        );
        assert_eq!(
            d(None, Some("ghostty"), None, false, false),
            Some(Protocol::Osc99)
        );
        assert_eq!(
            d(None, Some("WezTerm"), None, false, false),
            Some(Protocol::Osc777)
        );
        assert_eq!(
            d(Some("rxvt-unicode-256color"), None, None, false, false),
            Some(Protocol::Osc777)
        );
        assert_eq!(
            d(None, Some("iTerm.app"), None, false, false),
            Some(Protocol::Osc9)
        );
        assert_eq!(
            d(Some("xterm-256color"), None, None, true, false),
            Some(Protocol::Osc9)
        );
        assert_eq!(d(Some("xterm-256color"), None, None, false, false), None);
        assert_eq!(
            d(Some("xterm-kitty"), None, None, false, true),
            None,
            "multiplexed"
        );
    }

    #[test]
    fn notifier_rate_limits_and_delivers_the_latest() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut out = Vec::new();
        let mut notifier = Notifier::new(Protocol::Osc9);
        assert!(
            notifier
                .notify(&mut out, Notification::new("a"), at(0))
                .unwrap()
        );
        assert!(
            !notifier
                .notify(&mut out, Notification::new("b"), at(100))
                .unwrap()
        );
        assert!(
            !notifier
                .notify(&mut out, Notification::new("c"), at(200))
                .unwrap()
        );
        assert!(notifier.has_held());
        assert!(
            !notifier.poll(&mut out, at(1000)).unwrap(),
            "interval not over"
        );
        assert!(notifier.poll(&mut out, at(2000)).unwrap());
        assert!(!notifier.poll(&mut out, at(9000)).unwrap(), "nothing held");
        // An exact repeat within the interval is dropped, not held.
        assert!(
            !notifier
                .notify(&mut out, Notification::new("c"), at(2500))
                .unwrap()
        );
        assert!(!notifier.has_held());
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\x1b]9;a\x1b\\\x1b]9;c\x1b\\",
            "b was superseded by c"
        );
    }
}
