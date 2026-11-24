//! Opt-in desktop notifications and focus reporting outside the cell grid.
//!
//! Hosts choose when to notify, which protocol to use, and what text may leave
//! the terminal. Unsupported terminals ignore these sequences. Keep an in-app
//! indication as the portable fallback. Focus reporting delivers ordinary
//! terminal focus events; [`FocusReporting`] disables it on every exit path.

use std::io::{self, Write};

/// Notification encoding selected explicitly by the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// OSC 9, supported by iTerm2 and compatible terminals.
    Osc9,
    /// OSC 777 `notify`, supported by compatible desktop terminals.
    Osc777,
}

/// Encode bounded notification text without terminal control characters.
///
/// Each field is limited to 256 Unicode scalar values. Semicolons are replaced
/// so untrusted titles cannot introduce protocol fields or OSC 9 subcommands.
pub fn encode(title: &str, body: &str, protocol: Protocol) -> String {
    let title = field(title);
    let body = field(body);
    match protocol {
        Protocol::Osc9 => format!("\x1b]9;{title}: {body}\x1b\\"),
        Protocol::Osc777 => format!("\x1b]777;notify;{title};{body}\x1b\\"),
    }
}

fn field(text: &str) -> String {
    text.chars()
        .take(256)
        .filter_map(|ch| {
            if matches!(ch, '\n' | '\r' | '\t') {
                Some(' ')
            } else if ch.is_control() {
                None
            } else if ch == ';' {
                Some(',')
            } else {
                Some(ch)
            }
        })
        .collect()
}

/// Write and flush one notification. No permission or capability is negotiated.
pub fn write(
    writer: &mut impl Write,
    title: &str,
    body: &str,
    protocol: Protocol,
) -> io::Result<()> {
    writer.write_all(encode(title, body, protocol).as_bytes())?;
    writer.flush()
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
}
