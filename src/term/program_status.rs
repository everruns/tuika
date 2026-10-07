//! OSC 7501 program status reports.
//!
//! A host emits these while it works so the terminal can show something
//! better than a spinning cursor: idle, working (with optional 0-100
//! progress), done, blocked on the user, or errored. Records form a tree
//! via [`Report::id`]: absent means the root record, `parent/child` means
//! a child of `parent`. The host owns the semantics; tuika only encodes.
//!
//! ```rust
//! use tuika::term::program_status::{Report, State};
//!
//! let report = Report::new(State::Working)
//!     .id("build")
//!     .progress(42)
//!     .app("cargo");
//! let bytes = report.encode().expect("valid report");
//! assert!(bytes.starts_with("\x1b]7501;"));
//! ```
//!
//! Anything tuika did not produce itself is untrusted: ids, app names,
//! titles, and messages come from the host and are validated here before
//! they reach the terminal. Invalid reports return `None` from
//! [`Report::encode`] and [`crate::term::program_status::write`] emits nothing rather than a
//! half-valid sequence. Unknown terminals swallow the OSC whole, so no
//! capability detection is needed.
//!
//! Spec: <https://www.superlogical.com/rex/docs/build/program-status>

use std::io::Write;

/// Program activity states (`state=` key).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Nothing running. Also the reset for the root record.
    Idle,
    /// Work in progress. May carry `progress=`, `title=`, `msg=`.
    Working,
    /// Finished successfully.
    Done,
    /// Waiting on the user. May carry `kind=`.
    Blocked,
    /// Failed.
    Error,
    /// Clear this record (and its children).
    Clear,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Working => "working",
            State::Done => "done",
            State::Blocked => "blocked",
            State::Error => "error",
            State::Clear => "clear",
        }
    }
}

/// Why a record is blocked (`kind=` key, only with [`State::Blocked`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// Needs a permission grant.
    Permission,
    /// Needs an answer.
    Question,
    /// Needs authentication.
    Auth,
}

impl BlockKind {
    fn as_str(self) -> &'static str {
        match self {
            BlockKind::Permission => "permission",
            BlockKind::Question => "question",
            BlockKind::Auth => "auth",
        }
    }
}

/// One OSC 7501 status report.
///
/// Built with the builder methods, then either [`Report::encode`] for the
/// raw bytes or [`crate::term::program_status::write`] to push them at an I/O handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    state: State,
    id: Option<String>,
    kind: Option<BlockKind>,
    progress: Option<u8>,
    app: Option<String>,
    title: Option<String>,
    msg: Option<String>,
}

impl Report {
    /// A report in `state` for the root record.
    pub fn new(state: State) -> Self {
        Report {
            state,
            id: None,
            kind: None,
            progress: None,
            app: None,
            title: None,
            msg: None,
        }
    }

    /// Clear the record `id` (root when `None`).
    pub fn clear(id: Option<&str>) -> Self {
        let mut report = Report::new(State::Clear);
        if let Some(id) = id {
            report.id = Some(id.to_string());
        }
        report
    }

    /// Record id: `segment(/segment)*`, each 1-32 chars of
    /// `[A-Za-z0-9_.+-]`, at most 8 segments and 128 bytes in all. Absent
    /// means the root record.
    pub fn id(mut self, id: &str) -> Self {
        self.id = Some(id.to_string());
        self
    }

    /// Block reason. Ignored unless the state is [`State::Blocked`].
    pub fn kind(mut self, kind: BlockKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Completion percent. Kept only with `working`/`blocked` and only
    /// when `0..=100`; anything else is dropped at encode time.
    pub fn progress(mut self, percent: u8) -> Self {
        self.progress = Some(percent);
        self
    }

    /// Application name, e.g. `cargo`. Same grammar as one id segment.
    pub fn app(mut self, app: &str) -> Self {
        self.app = Some(app.to_string());
        self
    }

    /// Short label for non-root records. Plain UTF-8; encoded to base64
    /// here. Must be free of control characters.
    pub fn title(mut self, title: &str) -> Self {
        self.title = Some(title.to_string());
        self
    }

    /// One human-readable line. Plain UTF-8; encoded to base64 here.
    /// Must be free of control characters.
    pub fn msg(mut self, msg: &str) -> Self {
        self.msg = Some(msg.to_string());
        self
    }

    /// Encode the report as `OSC 7501 … ST`, or `None` when invalid.
    ///
    /// Invalid means: bad `id`/`app` grammar, a control character in
    /// `title`/`msg`, or any spec length limit overflow. Combinations
    /// the spec ignores (`kind=` without `blocked`, `progress=` without
    /// `working`/`blocked`) are dropped, not rejected.
    pub fn encode(&self) -> Option<String> {
        let mut pairs: Vec<(String, String)> = Vec::with_capacity(7);
        pairs.push(("state".to_string(), self.state.as_str().to_string()));

        if let Some(id) = &self.id {
            if !is_valid_id(id) {
                return None;
            }
            pairs.push(("id".to_string(), id.clone()));
        }

        if self.state == State::Blocked
            && let Some(kind) = self.kind
        {
            pairs.push(("kind".to_string(), kind.as_str().to_string()));
        }

        if matches!(self.state, State::Working | State::Blocked)
            && let Some(percent) = self.progress
            && percent <= 100
        {
            pairs.push(("progress".to_string(), percent.to_string()));
        }

        if let Some(app) = &self.app {
            if !is_valid_segment(app) {
                return None;
            }
            pairs.push(("app".to_string(), app.clone()));
        }

        if let Some(title) = &self.title {
            let encoded = encode_text(title, 192, 256)?;
            pairs.push(("title".to_string(), encoded));
        }

        if let Some(msg) = &self.msg {
            let encoded = encode_text(msg, 2048, 2732)?;
            pairs.push(("msg".to_string(), encoded));
        }

        let mut out = String::with_capacity(32 + pairs.len() * 16);
        out.push_str("\x1b]7501;");
        for (i, (key, value)) in pairs.iter().enumerate() {
            if i > 0 {
                out.push(':');
            }
            out.push_str(key);
            out.push('=');
            out.push_str(value);
        }
        out.push_str("\x1b\\");

        // Whole-sequence cap from the spec.
        if out.len() > 4096 {
            return None;
        }
        Some(out)
    }
}

/// Encode the `OSC 7501 ; ? ST` feature-detection query.
pub fn encode_query() -> String {
    "\x1b]7501;?\x1b\\".to_string()
}

/// Write one report. Invalid reports emit nothing and still succeed, so a
/// host can fire-and-forget without branching on validation.
pub fn write(out: &mut impl Write, report: &Report) -> std::io::Result<()> {
    if let Some(bytes) = report.encode() {
        out.write_all(bytes.as_bytes())?;
    }
    Ok(())
}

fn is_valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 32
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'+' | b'-'))
}

/// Spec limits: at most 128 bytes in all and 8 segments deep. A longer or
/// deeper id is discarded whole by the terminal, never truncated, so it must
/// not be emitted at all.
fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.split('/').count() <= 8
        && id.split('/').all(is_valid_segment)
}

/// Check plain text, then base64 it. `None` on control characters or
/// over either the decoded or the encoded limit.
///
/// The spec's control set is C0, DEL, *and* C1 (U+0080..=U+009F): a
/// terminal discards a report whose decoded text holds any of them, so a
/// byte-level C0 check alone would let e.g. U+0085 (NEL) through.
fn encode_text(text: &str, max_decoded: usize, max_encoded: usize) -> Option<String> {
    if text
        .chars()
        .any(|c| c <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&c))
    {
        return None;
    }
    if text.len() > max_decoded {
        return None;
    }
    let encoded = base64_encode(text.as_bytes());
    if encoded.len() > max_encoded {
        return None;
    }
    Some(encoded)
}

fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((triple >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(triple & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_working_encodes_with_state_only() {
        assert_eq!(
            Report::new(State::Working).encode(),
            Some("\x1b]7501;state=working\x1b\\".to_string())
        );
    }

    #[test]
    fn full_record_pairs_in_fixed_order() {
        let report = Report::new(State::Working)
            .id("build/lint")
            .progress(42)
            .app("cargo")
            .title("Lint")
            .msg("Deny warnings");
        assert_eq!(
            report.encode(),
            Some(
                "\x1b]7501;state=working:id=build/lint:progress=42:app=cargo:title=TGludA==:msg=RGVueSB3YXJuaW5ncw==\x1b\\"
                    .to_string()
            )
        );
    }

    #[test]
    fn kind_kept_only_with_blocked() {
        let blocked = Report::new(State::Blocked)
            .id("deploy")
            .kind(BlockKind::Permission)
            .encode()
            .unwrap();
        assert!(blocked.contains(":kind=permission"));

        let working = Report::new(State::Working)
            .kind(BlockKind::Permission)
            .encode()
            .unwrap();
        assert!(!working.contains("kind="));
    }

    #[test]
    fn progress_kept_only_with_working_or_blocked() {
        let working = Report::new(State::Working).progress(7).encode().unwrap();
        assert!(working.contains(":progress=7"));

        let done = Report::new(State::Done).progress(7).encode().unwrap();
        assert!(!done.contains("progress="));

        let out_of_range = Report::new(State::Working).progress(101).encode().unwrap();
        assert!(!out_of_range.contains("progress="));
    }

    #[test]
    fn bad_id_or_app_rejects_report() {
        assert_eq!(Report::new(State::Working).id("bad id").encode(), None);
        assert_eq!(Report::new(State::Working).id("").encode(), None);
        assert_eq!(Report::new(State::Working).app("not valid!").encode(), None);
    }

    #[test]
    fn id_over_spec_limits_rejects_report() {
        // 8 segments of 15 bytes plus separators: 127 bytes, the deepest legal id.
        let deepest = vec!["a".repeat(15); 8].join("/");
        assert!(Report::new(State::Working).id(&deepest).encode().is_some());

        let too_deep = ["a"; 9].join("/");
        assert_eq!(Report::new(State::Working).id(&too_deep).encode(), None);

        let too_long = vec!["a".repeat(32); 4].join("/"); // 131 bytes, 4 segments
        assert_eq!(Report::new(State::Working).id(&too_long).encode(), None);
    }

    #[test]
    fn control_chars_in_text_reject_report() {
        assert_eq!(
            Report::new(State::Working).msg("line\nbreak").encode(),
            None
        );
        assert_eq!(
            Report::new(State::Working).title("tab\there").encode(),
            None
        );
        // C1 controls are controls too, though their UTF-8 bytes are >= 0x80.
        assert_eq!(
            Report::new(State::Working).msg("next\u{85}line").encode(),
            None
        );
        assert_eq!(
            Report::new(State::Working).title("csi\u{9b}").encode(),
            None
        );
        assert!(
            Report::new(State::Working)
                .msg("caf\u{e9} \u{a0}ok")
                .encode()
                .is_some()
        );
    }

    #[test]
    fn text_limits_reject_report() {
        assert_eq!(
            Report::new(State::Working).title(&"x".repeat(193)).encode(),
            None
        );
        assert_eq!(
            Report::new(State::Working).msg(&"x".repeat(2049)).encode(),
            None
        );
    }

    #[test]
    fn clear_helper_targets_record() {
        assert_eq!(
            Report::clear(Some("build")).encode(),
            Some("\x1b]7501;state=clear:id=build\x1b\\".to_string())
        );
        assert_eq!(
            Report::clear(None).encode(),
            Some("\x1b]7501;state=clear\x1b\\".to_string())
        );
    }

    #[test]
    fn query_has_expected_shape() {
        assert_eq!(encode_query(), "\x1b]7501;?\x1b\\");
    }

    #[test]
    fn write_skips_invalid_reports() {
        let mut out = Vec::new();
        write(&mut out, &Report::new(State::Working).id("bad id")).unwrap();
        assert!(out.is_empty());

        write(&mut out, &Report::new(State::Done)).unwrap();
        assert_eq!(out, b"\x1b]7501;state=done\x1b\\");
    }
}
