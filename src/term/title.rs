//! Window and tab titles (OSC 0 / OSC 2), with the xterm title stack.
//!
//! A program that names its window should give the old name back when it
//! leaves, or the user's shell tab stays called "build: compiling" long after
//! the build ended. The xterm title stack does that: [`encode_push`] saves the
//! current title before the first change and [`encode_pop`] restores it.
//! Terminals without a title stack ignore both; the worst case is that the
//! program's last title remains, which is no worse than not restoring it.
//!
//! [`WindowTitle`] pairs them for a host with its own loop — push once,
//! set on change, pop on drop — and the runners own one through
//! [`Runner::window_title`](crate::Runner::window_title).
//!
//! The title is the host's text, so it is checked here: control characters
//! (C0, DEL, C1) would end or extend the sequence, and are replaced rather
//! than passed through, and an over-long title is cut at a character boundary.
//!
//! ```
//! use tuika::term::title;
//!
//! assert_eq!(title::encode("build: 40%"), "\x1b]2;build: 40%\x1b\\");
//! assert_eq!(title::encode("evil\x1b]52;c;x\x07"), "\x1b]2;evil ]52;c;x \x1b\\");
//! ```
//!
//! Renaming a tab through a terminal's own RPC (Rex's `rex` CLI, a tmux
//! command) is the host's business; tuika only speaks the escape.

use std::io::{self, Write};

/// The longest title sent, in bytes. Terminals truncate long titles anyway;
/// the cap bounds what a host's runaway string can push down the tty.
pub const MAX_TITLE_BYTES: usize = 512;

/// `CSI 22 ; 0 t`: save the window and icon titles on the xterm title stack.
pub fn encode_push() -> &'static str {
    "\x1b[22;0t"
}

/// `CSI 23 ; 0 t`: restore the titles saved by [`encode_push`].
pub fn encode_pop() -> &'static str {
    "\x1b[23;0t"
}

/// `OSC 2 ; title ST`: set the window (and, in most terminals, tab) title.
/// Control characters become spaces; the title is cut to
/// [`MAX_TITLE_BYTES`].
pub fn encode(title: &str) -> String {
    let mut out = String::with_capacity(title.len().min(MAX_TITLE_BYTES) + 6);
    out.push_str("\x1b]2;");
    let mut used = 0;
    for c in title.chars() {
        let c = if c <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&c) {
            ' '
        } else {
            c
        };
        if used + c.len_utf8() > MAX_TITLE_BYTES {
            break;
        }
        used += c.len_utf8();
        out.push(c);
    }
    out.push_str("\x1b\\");
    out
}

/// Write one title.
pub fn write(out: &mut impl Write, title: &str) -> io::Result<()> {
    out.write_all(encode(title).as_bytes())
}

/// Sets a window title on change and restores the previous one at the end.
///
/// The first [`set`](Self::set) pushes the terminal's current title; [`finish`](Self::finish)
/// pops it back. Setting `None` restores the saved title too, so "no title of
/// my own" means the user's.
#[derive(Debug, Default)]
pub struct WindowTitle {
    current: Option<String>,
    pushed: bool,
}

impl WindowTitle {
    /// A driver that has written nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// The title last written, if any.
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// Show `title`, or restore the saved title for `None`. Writes only on
    /// change; returns whether it wrote.
    pub fn set(&mut self, out: &mut impl Write, title: Option<&str>) -> io::Result<bool> {
        if self.current.as_deref() == title {
            return Ok(false);
        }
        match title {
            Some(title) => {
                if !self.pushed {
                    out.write_all(encode_push().as_bytes())?;
                    self.pushed = true;
                }
                write(out, title)?;
                self.current = Some(title.to_string());
            }
            None => {
                self.finish(out)?;
                return Ok(true);
            }
        }
        out.flush()?;
        Ok(true)
    }

    /// Restore the title saved before the first change. Idempotent.
    pub fn finish(&mut self, out: &mut impl Write) -> io::Result<()> {
        self.current = None;
        if self.pushed {
            out.write_all(encode_pop().as_bytes())?;
            self.pushed = false;
            out.flush()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_sanitizes_controls() {
        assert_eq!(encode("a\nb\tc"), "\x1b]2;a b c\x1b\\");
        assert_eq!(encode("st\u{9c}end\u{7f}"), "\x1b]2;st end \x1b\\");
        assert_eq!(
            encode("caf\u{e9} \u{1f600}"),
            "\x1b]2;caf\u{e9} \u{1f600}\x1b\\"
        );
    }

    #[test]
    fn encode_caps_length_on_a_char_boundary() {
        let long = "\u{e9}".repeat(MAX_TITLE_BYTES);
        let encoded = encode(&long);
        let body = &encoded[4..encoded.len() - 2];
        assert_eq!(body.len(), MAX_TITLE_BYTES);
        assert!(body.chars().all(|c| c == '\u{e9}'));
    }

    #[test]
    fn window_title_pushes_once_sets_on_change_and_pops() {
        let mut out = Vec::new();
        let mut title = WindowTitle::new();
        assert!(title.set(&mut out, Some("a")).unwrap());
        assert!(!title.set(&mut out, Some("a")).unwrap());
        assert!(title.set(&mut out, Some("b")).unwrap());
        title.finish(&mut out).unwrap();
        title.finish(&mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\x1b[22;0t\x1b]2;a\x1b\\\x1b]2;b\x1b\\\x1b[23;0t"
        );
    }

    #[test]
    fn window_title_none_restores_and_can_set_again() {
        let mut out = Vec::new();
        let mut title = WindowTitle::new();
        assert!(!title.set(&mut out, None).unwrap(), "nothing to restore");
        title.set(&mut out, Some("a")).unwrap();
        title.set(&mut out, None).unwrap();
        title.set(&mut out, Some("a")).unwrap();
        title.finish(&mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\x1b[22;0t\x1b]2;a\x1b\\\x1b[23;0t\x1b[22;0t\x1b]2;a\x1b\\\x1b[23;0t"
        );
    }
}
