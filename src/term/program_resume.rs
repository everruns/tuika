//! OSC 7577 program resume reports.
//!
//! A program tells the terminal which command would bring it back — the same
//! editor session, the same log view, the same project — so a terminal that is
//! restarted, restored, or reattached can relaunch it in place of a bare
//! shell. The terminal never answers; it only records. Restoring is entirely
//! the terminal's decision.
//!
//! Unlike the other out-of-band escapes, every report is **authenticated**. The
//! terminal hands the first process on each pty a random secret in
//! [`TOKEN_ENV`] (`OSC7577_TOKEN`), and each report carries
//! `mac = sha256(body ":" token)`. A program that cannot see the token — a
//! `cat` of a hostile file, an ssh session, a sandboxed child — cannot forge a
//! command the terminal would later run. That is why the token is also the
//! feature test: **no token, no report**, and nothing here writes one without
//! it.
//!
//! ```rust
//! use tuika::term::program_resume::{Report, Token};
//!
//! // Normally `Token::from_env()`; a fixed token keeps the doctest hermetic.
//! let token = Token::new("dummy-token-replace-me").expect("valid token");
//! let report = Report::resume(["lazygit"])
//!     .cwd("/home/user/src/project")
//!     .pid(74877)
//!     .time(1795543200);
//! assert_eq!(
//!     report.encode(&token).unwrap(),
//!     "\x1b]7577;pid=74877:time=1795543200:argv=bGF6eWdpdA==\
//!      :cwd=L2hvbWUvdXNlci9zcmMvcHJvamVjdA==\
//!      :mac=35cb909805c380336efd3689a125acc774a9dec8422a1d21492ebec06ca758e1\x1b\\"
//! );
//! ```
//!
//! Most hosts want [`Reporter`], which owns the token, stamps fresh `pid` and
//! `time` on every send, only re-sends when the command actually changes, and
//! knows the spec's fallbacks (an unencodable command clears the old one
//! rather than leaving it to be resumed). [`verify`] is the terminal's half of
//! the protocol — strict parsing, MAC and time checks, the ignored-versus-
//! rejected split — so a host's own tests can assert that what it emits would
//! be accepted, not merely that it looks plausible.
//!
//! The token is a credential. [`Token`]'s `Debug` redacts it, and a host that
//! starts other programs must strip it from their environment with
//! [`scrub_env`]: a child that inherits it could report a command in the
//! host's name.
//!
//! Spec: <https://www.superlogical.com/rex/docs/build/program-resume>
//!
//! ![the program_resume example after a resume](https://raw.githubusercontent.com/everruns/tuika/main/examples/program_resume.png)

use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The environment variable the terminal puts the token in.
pub const TOKEN_ENV: &str = "OSC7577_TOKEN";

/// Longest whole sequence (`ESC ] 7577 ;` through the terminator) every
/// conforming terminal must accept. Programs should stay within it; a
/// terminal may accept more, but one that cannot hold a report ignores it.
pub const MAX_SEQUENCE_LEN: usize = 2048;

/// Most `argv` elements every conforming terminal must accept.
pub const MAX_ARGV: usize = 64;

/// How far a report's `time` may sit from the terminal's clock, in seconds.
/// The spec recommends 60; [`verify`] uses it.
pub const TIME_WINDOW_SECS: u64 = 60;

const PREFIX: &str = "\x1b]7577;";
const ST: &str = "\x1b\\";

// ---------------------------------------------------------------------------
// Token

/// The per-pty secret a report is signed with.
///
/// 22–64 characters of `[A-Za-z0-9_-]`. `Debug` prints `Token(<redacted>)`
/// and there is no `Display`, so it cannot end up in a log by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// A token, or `None` when it does not fit the spec's grammar. A
    /// malformed token is treated as absent: signing with it could only
    /// produce reports the terminal ignores.
    pub fn new(token: impl Into<String>) -> Option<Self> {
        let token = token.into();
        let ok = (22..=64).contains(&token.len())
            && token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        ok.then_some(Token(token))
    }

    /// The token the terminal passed this process, if any. Its absence means
    /// the terminal does not support the protocol (or chose not to offer it
    /// here), and the program must not report.
    pub fn from_env() -> Option<Self> {
        std::env::var(TOKEN_ENV).ok().and_then(Token::new)
    }

    /// `mac` for a body: lowercase hex SHA-256 of `body ":" token`. The token
    /// goes last so that one valid MAC does not let anyone extend its body.
    fn mac(&self, body: &str) -> String {
        let mut hasher = sha256::Sha256::new();
        hasher.update(body.as_bytes());
        hasher.update(b":");
        hasher.update(self.0.as_bytes());
        hex(&hasher.finish())
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// Remove [`TOKEN_ENV`] from a child's environment.
///
/// The spec requires it for every program a reporter starts: a child holding
/// the token could replace the host's resume command with its own. Call it on
/// every [`std::process::Command`] the host spawns.
pub fn scrub_env(command: &mut std::process::Command) -> &mut std::process::Command {
    command.env_remove(TOKEN_ENV)
}

// ---------------------------------------------------------------------------
// Reports

/// What a report asks the terminal to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Remember this command (`argv`, optionally `cwd`) as the way to resume
    /// the owner.
    Resume {
        /// Element 0 is the program: a bare name looked up on `PATH` at
        /// resume time, or an absolute path. It is also the program's own
        /// `argv[0]`.
        argv: Vec<String>,
        /// Absolute working directory. When it no longer exists at restore
        /// time, the terminal does not resume.
        cwd: Option<PathBuf>,
    },
    /// Forget the owner's command (`clear=1`).
    Clear,
}

/// One OSC 7577 report, before signing.
///
/// `pid` defaults to this process and `time` to now, both read when the
/// report is encoded — a report is only valid for [`TIME_WINDOW_SECS`], so
/// it is built fresh rather than saved and replayed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    action: Action,
    pid: Option<u32>,
    time: Option<u64>,
}

impl Report {
    /// Resume with this command line. See [`Action::Resume`] for what
    /// element 0 may be.
    pub fn resume<I, S>(argv: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Report {
            action: Action::Resume {
                argv: argv.into_iter().map(Into::into).collect(),
                cwd: None,
            },
            pid: None,
            time: None,
        }
    }

    /// Forget the owner's resume command. Send it when the *user* ends the
    /// program — not on `SIGHUP`, `SIGTERM`, or a lost terminal, which are
    /// exactly the cases resuming is for.
    pub fn clear() -> Self {
        Report {
            action: Action::Clear,
            pid: None,
            time: None,
        }
    }

    /// Working directory to resume in. Absolute; ignored on a clear.
    pub fn cwd(mut self, path: impl Into<PathBuf>) -> Self {
        if let Action::Resume { cwd, .. } = &mut self.action {
            *cwd = Some(path.into());
        }
        self
    }

    /// Owning process. Defaults to this one; a wrapper that `exec`s its
    /// target reports with its own pid, which the target inherits.
    pub fn pid(mut self, pid: u32) -> Self {
        self.pid = Some(pid);
        self
    }

    /// Unix time in seconds. Defaults to now; set it only for tests.
    pub fn time(mut self, secs: u64) -> Self {
        self.time = Some(secs);
        self
    }

    /// What the report asks for.
    pub fn action(&self) -> &Action {
        &self.action
    }

    /// The signed body: every pair before `:mac=`, in the fixed order
    /// `pid`, `time`, then `argv` and `cwd`, or `clear`.
    fn body(&self) -> Result<String, ReportError> {
        let pid = self.pid.unwrap_or_else(std::process::id);
        let time = self.time.unwrap_or_else(now);
        if pid == 0 {
            return Err(ReportError::InvalidPid);
        }
        if time == 0 {
            return Err(ReportError::InvalidTime);
        }
        let mut body = format!("pid={pid}:time={time}");
        match &self.action {
            Action::Clear => body.push_str(":clear=1"),
            Action::Resume { argv, cwd } => {
                check_argv(argv.iter().map(String::as_str))?;
                body.push_str(":argv=");
                for (i, arg) in argv.iter().enumerate() {
                    if i > 0 {
                        body.push(',');
                    }
                    body.push_str(&base64_encode(arg.as_bytes()));
                }
                if let Some(cwd) = cwd {
                    let text = cwd.to_str().ok_or(ReportError::NotUtf8)?;
                    check_cwd(text)?;
                    body.push_str(":cwd=");
                    body.push_str(&base64_encode(text.as_bytes()));
                }
            }
        }
        Ok(body)
    }

    /// Sign and frame the report as `OSC 7577 ; … ST`.
    ///
    /// Fails, rather than emitting something the terminal would discard, on
    /// anything [`ReportError`] lists — including a sequence over
    /// [`MAX_SEQUENCE_LEN`].
    pub fn encode(&self, token: &Token) -> Result<String, ReportError> {
        let body = self.body()?;
        let mac = token.mac(&body);
        let out = format!("{PREFIX}{body}:mac={mac}{ST}");
        if out.len() > MAX_SEQUENCE_LEN {
            return Err(ReportError::TooLong);
        }
        Ok(out)
    }
}

/// Why a [`Report`] cannot be encoded.
///
/// Each one is a report a conforming terminal would ignore or reject, so
/// tuika refuses to send it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReportError {
    /// A resume command needs at least the program.
    EmptyArgv,
    /// More than [`MAX_ARGV`] elements.
    TooManyArgs,
    /// Element 0 is empty, starts with `-`, or is a relative path. Only a
    /// bare name (found on `PATH` at resume time) or an absolute path is
    /// allowed.
    InvalidProgram,
    /// `cwd` is not an absolute path.
    RelativeCwd,
    /// `cwd` is not valid UTF-8.
    NotUtf8,
    /// An argument or `cwd` holds a control character (C0, DEL, or C1). A
    /// newline in an argument is the common case; the spec's answer is to
    /// clear instead, which [`Reporter`] does.
    ControlCharacter,
    /// `pid` is zero.
    InvalidPid,
    /// `time` is zero.
    InvalidTime,
    /// The sequence is longer than [`MAX_SEQUENCE_LEN`].
    TooLong,
}

impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ReportError::EmptyArgv => "resume command has no program",
            ReportError::TooManyArgs => "resume command has more than 64 arguments",
            ReportError::InvalidProgram => {
                "program must be a bare name or an absolute path, not starting with '-'"
            }
            ReportError::RelativeCwd => "cwd must be an absolute path",
            ReportError::NotUtf8 => "cwd is not valid UTF-8",
            ReportError::ControlCharacter => "argument or cwd contains a control character",
            ReportError::InvalidPid => "pid must be non-zero",
            ReportError::InvalidTime => "time must be non-zero",
            ReportError::TooLong => "report is longer than 2048 bytes",
        })
    }
}

impl std::error::Error for ReportError {}

/// Write one signed report. Nothing is written when it cannot be encoded.
pub fn write(out: &mut impl Write, report: &Report, token: &Token) -> Result<(), SendError> {
    let bytes = report.encode(token).map_err(SendError::Invalid)?;
    out.write_all(bytes.as_bytes())?;
    out.flush()?;
    Ok(())
}

/// Why a send failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum SendError {
    /// The report could not be encoded. Nothing was written for it.
    Invalid(ReportError),
    /// Writing to the terminal failed.
    Io(io::Error),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::Invalid(err) => write!(f, "invalid resume report: {err}"),
            SendError::Io(err) => write!(f, "writing resume report: {err}"),
        }
    }
}

impl std::error::Error for SendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SendError::Invalid(err) => Some(err),
            SendError::Io(err) => Some(err),
        }
    }
}

impl From<io::Error> for SendError {
    fn from(err: io::Error) -> Self {
        SendError::Io(err)
    }
}

// ---------------------------------------------------------------------------
// Reporter

/// Owns the token and the current resume command for one program.
///
/// Built only when the terminal offered a token, so holding one *is* the
/// capability check:
///
/// ```rust,no_run
/// use tuika::term::program_resume::{Report, Reporter};
///
/// let mut resume = Reporter::from_env(); // None outside a supporting terminal
/// if let Some(resume) = &mut resume {
///     let _ = resume.update(&mut std::io::stdout(), &Report::resume(["myapp", "--open", "notes.md"]));
/// }
/// // ... and when the user quits:
/// if let Some(resume) = &mut resume {
///     let _ = resume.clear(&mut std::io::stdout());
/// }
/// ```
///
/// Reports are written to whatever the host passes — in a tuika app, the
/// same stdout the [`Terminal`](crate::term::terminal::Terminal) draws on,
/// between frames. OSC 7577 never moves the cursor, so it does not disturb
/// the frame diff.
#[derive(Debug, Clone)]
pub struct Reporter {
    token: Token,
    current: Option<Report>,
}

impl Reporter {
    /// A reporter signing with `token`.
    pub fn new(token: Token) -> Self {
        Reporter {
            token,
            current: None,
        }
    }

    /// A reporter for the token in [`TOKEN_ENV`], or `None` when the
    /// terminal did not provide one — in which case the program must not
    /// report at all.
    pub fn from_env() -> Option<Self> {
        Token::from_env().map(Reporter::new)
    }

    /// The command last sent, if any (cleared by [`Reporter::clear`]).
    pub fn current(&self) -> Option<&Report> {
        self.current.as_ref()
    }

    /// Send `report` now, stamped with this process and the current time
    /// unless the report sets its own.
    ///
    /// When a resume command cannot be encoded the old one must not
    /// survive — the terminal would bring back a state the program has left
    /// — so a `clear` goes out instead and the error is returned.
    pub fn send(&mut self, out: &mut impl Write, report: &Report) -> Result<(), SendError> {
        match report.encode(&self.token) {
            Ok(bytes) => {
                out.write_all(bytes.as_bytes())?;
                out.flush()?;
                self.current = match report.action {
                    Action::Clear => None,
                    Action::Resume { .. } => Some(report.clone()),
                };
                Ok(())
            }
            Err(err) => {
                if self.current.is_some() {
                    self.clear(out)?;
                }
                Err(SendError::Invalid(err))
            }
        }
    }

    /// Send `report` only if its command differs from the last one sent.
    /// Cheap enough to call on every state change, or every frame.
    pub fn update(&mut self, out: &mut impl Write, report: &Report) -> Result<(), SendError> {
        let unchanged = match (&self.current, &report.action) {
            (Some(current), Action::Resume { .. }) => current.action == report.action,
            (None, Action::Clear) => true,
            _ => false,
        };
        if unchanged {
            return Ok(());
        }
        self.send(out, report)
    }

    /// Re-send the current command with a fresh `time`.
    ///
    /// The spec asks for this whenever the program gets the terminal back —
    /// after `SIGCONT`, or after a child it ran in the foreground exits —
    /// because something else may have reported in the meantime.
    pub fn refresh(&mut self, out: &mut impl Write) -> Result<(), SendError> {
        match self.current.clone() {
            Some(mut report) => {
                report.time = None;
                self.send(out, &report)
            }
            None => Ok(()),
        }
    }

    /// Forget the resume command. Send it when the user deliberately ends
    /// the program; see [`Report::clear`].
    pub fn clear(&mut self, out: &mut impl Write) -> Result<(), SendError> {
        let pid = self.current.as_ref().and_then(|r| r.pid);
        let mut report = Report::clear();
        report.pid = pid;
        let bytes = report.encode(&self.token).map_err(SendError::Invalid)?;
        out.write_all(bytes.as_bytes())?;
        out.flush()?;
        self.current = None;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Verification (the terminal's half)

/// What a conforming terminal does with a report. See [`verify`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Accepted: the terminal stores (or, for a clear, removes) the record.
    Accepted(Record),
    /// Ignored: no effect at all. The terminal cannot tell who sent it, so it
    /// must not touch any existing record.
    Ignored(Ignored),
    /// Authentic but invalid: the terminal must remove every record for
    /// `pid` (when the pid itself was readable).
    Rejected {
        /// The owner whose records are dropped.
        pid: Option<u32>,
        /// What was wrong.
        reason: Rejected,
    },
}

/// An accepted report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Owning process.
    pub pid: u32,
    /// Unix time the report was built.
    pub time: u64,
    /// Resume command, or clear.
    pub action: Action,
}

/// Why a report was ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Ignored {
    /// Not an `OSC 7577 ; … ST` sequence, or truncated.
    NotAReport,
    /// The reserved `?` body.
    Query,
    /// Longer than [`MAX_SEQUENCE_LEN`].
    TooLong,
    /// No `mac`, or it is not the last key, or not 64 lowercase hex digits.
    MissingMac,
    /// The MAC does not match the token.
    BadMac,
    /// `time` is missing, malformed, or repeated.
    BadTime,
    /// `time` is more than [`TIME_WINDOW_SECS`] from now.
    Stale,
}

/// Why an authentic report was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Rejected {
    /// A pair is not `key=value` with a lowercase key and an allowed value.
    MalformedPair,
    /// A key appears twice.
    RepeatedKey,
    /// `pid` is missing or not a positive integer.
    BadPid,
    /// Neither, or both, of `argv` and `clear`.
    BadAction,
    /// `clear` has a value other than `1`.
    BadClear,
    /// A base64 element does not decode, or is not UTF-8.
    BadEncoding,
    /// Decoded text holds a control character.
    ControlCharacter,
    /// `argv=` with no value.
    EmptyArgv,
    /// More than [`MAX_ARGV`] elements.
    TooManyArgs,
    /// Element 0 is not a bare name or an absolute path.
    InvalidProgram,
    /// `cwd` is not absolute.
    RelativeCwd,
    /// `need` names a key this implementation does not support.
    UnsupportedNeed,
}

/// Check one report the way a conforming terminal does.
///
/// `sequence` is the whole escape, `ESC ] 7577 ;` through `ESC \` or `BEL`;
/// `now` is the terminal's clock in Unix seconds. The order follows the spec:
/// the MAC first, then `time`, and only then everything else — so a report
/// that fails authentication never reaches the parser, and a forged one can
/// never cause a record to be dropped.
///
/// Stateless: the per-pid "not older than the last accepted report" rule,
/// record storage, and restoring stay with the caller.
pub fn verify(sequence: &[u8], token: &Token, now: u64) -> Verdict {
    if sequence.len() > MAX_SEQUENCE_LEN {
        return Verdict::Ignored(Ignored::TooLong);
    }
    let Some(rest) = sequence.strip_prefix(PREFIX.as_bytes()) else {
        return Verdict::Ignored(Ignored::NotAReport);
    };
    let body = if let Some(body) = rest.strip_suffix(ST.as_bytes()) {
        body
    } else if let Some(body) = rest.strip_suffix(b"\x07") {
        body
    } else {
        return Verdict::Ignored(Ignored::NotAReport);
    };
    // Every allowed byte is ASCII; anything else cannot carry a valid MAC.
    let Ok(body) = std::str::from_utf8(body) else {
        return Verdict::Ignored(Ignored::MissingMac);
    };
    if body == "?" {
        return Verdict::Ignored(Ignored::Query);
    }

    // 1. MAC: the final pair, over exactly the bytes before it.
    let Some((signed, mac)) = body.rsplit_once(":mac=") else {
        return Verdict::Ignored(Ignored::MissingMac);
    };
    if mac.len() != 64 || !mac.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Verdict::Ignored(Ignored::MissingMac);
    }
    if !constant_time_eq(token.mac(signed).as_bytes(), mac.as_bytes()) {
        return Verdict::Ignored(Ignored::BadMac);
    }

    // 2. time: exactly one well-formed `time=` within the window.
    let mut times = signed
        .split(':')
        .filter_map(|pair| pair.strip_prefix("time="));
    let time = match (times.next(), times.next()) {
        (Some(value), None) => parse_positive(value),
        _ => None,
    };
    let Some(time) = time else {
        return Verdict::Ignored(Ignored::BadTime);
    };
    if time.abs_diff(now) > TIME_WINDOW_SECS {
        return Verdict::Ignored(Ignored::Stale);
    }

    // 3. Everything else. From here the report is authentic, so a failure
    //    drops the owner's records.
    let mut pid_value: Option<&str> = None;
    let mut argv_value: Option<&str> = None;
    let mut cwd_value: Option<&str> = None;
    let mut clear_value: Option<&str> = None;
    let mut need_value: Option<&str> = None;
    let mut seen: Vec<&str> = Vec::new();
    let mut malformed = None;
    for pair in signed.split(':') {
        let Some((key, value)) = pair.split_once('=') else {
            malformed = Some(Rejected::MalformedPair);
            break;
        };
        let key_ok = !key.is_empty() && key.bytes().all(|b| b.is_ascii_lowercase());
        let value_ok = value.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b',' | b'+' | b'/' | b'=' | b'-')
        });
        if !key_ok || !value_ok {
            malformed = Some(Rejected::MalformedPair);
            break;
        }
        if seen.contains(&key) {
            malformed = Some(Rejected::RepeatedKey);
            break;
        }
        seen.push(key);
        match key {
            "pid" => pid_value = Some(value),
            "argv" => argv_value = Some(value),
            "cwd" => cwd_value = Some(value),
            "clear" => clear_value = Some(value),
            "need" => need_value = Some(value),
            _ => {} // unknown keys are ignored unless named in `need`
        }
    }
    let pid = pid_value
        .and_then(parse_positive)
        .and_then(|p| u32::try_from(p).ok());
    let reject = |reason| Verdict::Rejected { pid, reason };
    if let Some(reason) = malformed {
        return reject(reason);
    }
    let Some(pid) = pid else {
        return reject(Rejected::BadPid);
    };
    if let Some(need) = need_value {
        const SUPPORTED: [&str; 7] = ["pid", "time", "mac", "argv", "cwd", "clear", "need"];
        if need.is_empty() || need.split(',').any(|k| !SUPPORTED.contains(&k)) {
            return reject(Rejected::UnsupportedNeed);
        }
    }

    let action = match (argv_value, clear_value) {
        (Some(_), Some(_)) | (None, None) => return reject(Rejected::BadAction),
        (None, Some(clear)) => {
            if clear != "1" {
                return reject(Rejected::BadClear);
            }
            Action::Clear
        }
        (Some(argv), None) => {
            if argv.is_empty() {
                return reject(Rejected::EmptyArgv);
            }
            let elements: Vec<&str> = argv.split(',').collect();
            if elements.len() > MAX_ARGV {
                return reject(Rejected::TooManyArgs);
            }
            let mut decoded = Vec::with_capacity(elements.len());
            for element in elements {
                match decode_text(element) {
                    Ok(text) => decoded.push(text),
                    Err(reason) => return reject(reason),
                }
            }
            if !is_valid_program(&decoded[0]) {
                return reject(Rejected::InvalidProgram);
            }
            let cwd = match cwd_value {
                None => None,
                Some(value) => {
                    let text = match decode_text(value) {
                        Ok(text) => text,
                        Err(reason) => return reject(reason),
                    };
                    if !is_absolute(&text) {
                        return reject(Rejected::RelativeCwd);
                    }
                    Some(PathBuf::from(text))
                }
            };
            Action::Resume { argv: decoded, cwd }
        }
    };
    Verdict::Accepted(Record { pid, time, action })
}

// ---------------------------------------------------------------------------
// Validation shared by both halves

fn check_argv<'a>(argv: impl ExactSizeIterator<Item = &'a str>) -> Result<(), ReportError> {
    let len = argv.len();
    if len == 0 {
        return Err(ReportError::EmptyArgv);
    }
    if len > MAX_ARGV {
        return Err(ReportError::TooManyArgs);
    }
    for (i, arg) in argv.enumerate() {
        if has_control(arg) {
            return Err(ReportError::ControlCharacter);
        }
        if i == 0 && !is_valid_program(arg) {
            return Err(ReportError::InvalidProgram);
        }
    }
    Ok(())
}

fn check_cwd(cwd: &str) -> Result<(), ReportError> {
    if has_control(cwd) {
        return Err(ReportError::ControlCharacter);
    }
    if !is_absolute(cwd) {
        return Err(ReportError::RelativeCwd);
    }
    Ok(())
}

/// The spec's control set: C0, DEL, and C1. A byte-level check would miss
/// C1, whose UTF-8 encoding starts at 0xC2.
fn has_control(text: &str) -> bool {
    text.chars()
        .any(|c| c <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&c))
}

/// Element 0: an absolute path, or a bare name — no separator, not `-…`,
/// not `.`/`..`. A relative path would resolve against whatever directory
/// the terminal restores into, which is not what the program meant.
fn is_valid_program(program: &str) -> bool {
    if program.is_empty() || program.starts_with('-') {
        return false;
    }
    if is_absolute(program) {
        return true;
    }
    !program.contains('/')
        && !program.contains(std::path::MAIN_SEPARATOR)
        && program != "."
        && program != ".."
}

fn is_absolute(path: &str) -> bool {
    Path::new(path).is_absolute()
}

fn parse_positive(value: &str) -> Option<u64> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes[0] == b'0' || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    value.parse().ok()
}

fn decode_text(element: &str) -> Result<String, Rejected> {
    let bytes = base64_decode(element).ok_or(Rejected::BadEncoding)?;
    let text = String::from_utf8(bytes).map_err(|_| Rejected::BadEncoding)?;
    if has_control(&text) {
        return Err(Rejected::ControlCharacter);
    }
    Ok(text)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 15) as usize] as char);
    }
    out
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(BASE64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Standard base64. Padding is optional, but must be exactly right when
/// present (the spec's rule): `QQ`, `QQ==` decode; `QQ=`, `QQ===` do not.
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let bytes = input.as_bytes();
    let data = match bytes.iter().position(|&b| b == b'=') {
        None => bytes,
        Some(pad_at) => {
            if !bytes.len().is_multiple_of(4)
                || bytes.len() - pad_at > 2
                || !bytes[pad_at..].iter().all(|&b| b == b'=')
            {
                return None;
            }
            &bytes[..pad_at]
        }
    };
    if data.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(data.len() * 3 / 4);
    for chunk in data.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let v = BASE64.iter().position(|&a| a == c)? as u32;
            n |= v << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
}

/// FIPS 180-4 SHA-256, the one digest the protocol needs. Kept in-crate
/// rather than as a dependency: it is ~60 lines, has fixed test vectors, and
/// tuika keeps its dependency set small.
mod sha256 {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    pub(super) struct Sha256 {
        state: [u32; 8],
        block: [u8; 64],
        filled: usize,
        total: u64,
    }

    impl Sha256 {
        pub(super) fn new() -> Self {
            Sha256 {
                state: [
                    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c,
                    0x1f83d9ab, 0x5be0cd19,
                ],
                block: [0; 64],
                filled: 0,
                total: 0,
            }
        }

        pub(super) fn update(&mut self, mut data: &[u8]) {
            self.total = self.total.wrapping_add(data.len() as u64);
            while !data.is_empty() {
                let take = (64 - self.filled).min(data.len());
                self.block[self.filled..self.filled + take].copy_from_slice(&data[..take]);
                self.filled += take;
                data = &data[take..];
                if self.filled == 64 {
                    let block = self.block;
                    self.compress(&block);
                    self.filled = 0;
                }
            }
        }

        pub(super) fn finish(mut self) -> [u8; 32] {
            let bits = self.total.wrapping_mul(8);
            self.update(&[0x80]);
            while self.filled != 56 {
                self.update(&[0]);
            }
            self.update(&bits.to_be_bytes());
            let mut out = [0u8; 32];
            for (chunk, word) in out.chunks_exact_mut(4).zip(self.state) {
                chunk.copy_from_slice(&word.to_be_bytes());
            }
            out
        }

        fn compress(&mut self, block: &[u8; 64]) {
            let mut w = [0u32; 64];
            for (i, chunk) in block.chunks_exact(4).enumerate() {
                w[i] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }
            let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ (!e & g);
                let t1 = h
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                h = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }
            for (s, v) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
                *s = s.wrapping_add(v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "dummy-token-replace-me";
    const T: u64 = 1795543200;

    fn token() -> Token {
        Token::new(TOKEN).unwrap()
    }

    fn sha_hex(data: &[u8]) -> String {
        let mut h = sha256::Sha256::new();
        h.update(data);
        hex(&h.finish())
    }

    /// Sign an arbitrary body, for building reports the encoder refuses to.
    fn signed(body: &str) -> Vec<u8> {
        format!("{PREFIX}{body}:mac={}{ST}", token().mac(body)).into_bytes()
    }

    #[test]
    fn sha256_matches_fips_vectors() {
        assert_eq!(
            sha_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // One million 'a's, fed in uneven pieces to exercise block carry.
        let mut h = sha256::Sha256::new();
        let chunk = [b'a'; 997];
        let mut left = 1_000_000;
        while left > 0 {
            let n = left.min(chunk.len());
            h.update(&chunk[..n]);
            left -= n;
        }
        assert_eq!(
            hex(&h.finish()),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    // The four valid examples from the spec, with their published MACs.

    #[test]
    fn spec_example_neovim_session() {
        let report = Report::resume([
            "nvim",
            "-S",
            "/home/user/.local/state/nvim/sessions/8f2c1d.vim",
        ])
        .pid(74877)
        .time(T);
        let seq = report.encode(&token()).unwrap();
        assert!(seq.ends_with(
            ":mac=9dbb99bfb856749a87563839d7ce9062818d78cf25909db93f309e0cd3b1909b\x1b\\"
        ));
        assert!(seq.starts_with("\x1b]7577;pid=74877:time=1795543200:argv=bnZpbQ==,LVM=,"));
    }

    #[test]
    fn spec_example_clear() {
        assert_eq!(
            Report::clear().pid(74877).time(T).encode(&token()).unwrap(),
            "\x1b]7577;pid=74877:time=1795543200:clear=1\
             :mac=c30603369f98fef581a4098be889175b0c74f0641e49f8b25f4326cb38f97c93\x1b\\"
        );
    }

    #[test]
    fn spec_example_wrapper() {
        assert_eq!(
            Report::resume(["resume-wrap", "ssh", "devbox"])
                .pid(90412)
                .time(T)
                .encode(&token())
                .unwrap(),
            "\x1b]7577;pid=90412:time=1795543200:argv=cmVzdW1lLXdyYXA=,c3No,ZGV2Ym94\
             :mac=e76149d0ba54a83570ffab0583867dacdfa23928ba9d18604f0badea88b62b65\x1b\\"
        );
    }

    #[test]
    fn spec_examples_verify_as_accepted() {
        let seq = Report::resume(["lazygit"])
            .cwd("/home/user/src/project")
            .pid(74877)
            .time(T)
            .encode(&token())
            .unwrap();
        assert_eq!(
            verify(seq.as_bytes(), &token(), T + 30),
            Verdict::Accepted(Record {
                pid: 74877,
                time: T,
                action: Action::Resume {
                    argv: vec!["lazygit".into()],
                    cwd: Some("/home/user/src/project".into()),
                },
            })
        );
    }

    #[test]
    fn round_trips_awkward_arguments_as_single_arguments() {
        let argv = vec![
            "/usr/bin/env".to_string(),
            String::new(),
            "$(reboot)".into(),
            "a; b".into(),
            "two words".into(),
            "--flag=x:y;z".into(),
            "ünïcødé 漢字 \u{202e}bidi".into(),
        ];
        let seq = Report::resume(argv.clone())
            .time(T)
            .encode(&token())
            .unwrap();
        match verify(seq.as_bytes(), &token(), T) {
            Verdict::Accepted(Record {
                action: Action::Resume { argv: got, cwd },
                ..
            }) => {
                assert_eq!(got, argv);
                assert_eq!(cwd, None);
            }
            other => panic!("expected accept, got {other:?}"),
        }
    }

    #[test]
    fn bel_terminator_is_accepted() {
        let body = format!("pid=1:time={T}:clear=1");
        let seq = format!("{PREFIX}{body}:mac={}\x07", token().mac(&body));
        assert!(matches!(
            verify(seq.as_bytes(), &token(), T),
            Verdict::Accepted(_)
        ));
    }

    // The spec's invalid-report table.

    #[test]
    fn forged_or_unauthenticated_reports_are_ignored() {
        let good = signed(&format!("pid=1:time={T}:clear=1"));
        let text = String::from_utf8(good.clone()).unwrap();

        let wrong_mac = format!("{}{}{ST}", &text[..text.len() - 66], "0".repeat(64));
        assert_eq!(
            verify(wrong_mac.as_bytes(), &token(), T),
            Verdict::Ignored(Ignored::BadMac)
        );

        let other = Token::new("another-token-of-good-length").unwrap();
        assert_eq!(verify(&good, &other, T), Verdict::Ignored(Ignored::BadMac));

        let no_mac = format!("{PREFIX}pid=1:time={T}:clear=1{ST}");
        assert_eq!(
            verify(no_mac.as_bytes(), &token(), T),
            Verdict::Ignored(Ignored::MissingMac)
        );

        // mac not last: the trailing pair makes the "mac" non-hex.
        let body = format!("pid=1:time={T}");
        let mac = token().mac(&body);
        let not_last = format!("{PREFIX}{body}:mac={mac}:clear=1{ST}");
        assert_eq!(
            verify(not_last.as_bytes(), &token(), T),
            Verdict::Ignored(Ignored::MissingMac)
        );

        assert_eq!(
            verify(format!("{PREFIX}?{ST}").as_bytes(), &token(), T),
            Verdict::Ignored(Ignored::Query)
        );
        assert_eq!(
            verify(&good[..good.len() - 2], &token(), T),
            Verdict::Ignored(Ignored::NotAReport)
        );
    }

    #[test]
    fn bad_or_stale_time_is_ignored() {
        assert_eq!(
            verify(&signed("pid=1:time=1000000000:clear=1"), &token(), T),
            Verdict::Ignored(Ignored::Stale)
        );
        assert_eq!(
            verify(&signed("pid=1:clear=1"), &token(), T),
            Verdict::Ignored(Ignored::BadTime)
        );
        assert_eq!(
            verify(&signed(&format!("pid=1:time=0{T}:clear=1")), &token(), T),
            Verdict::Ignored(Ignored::BadTime)
        );
        assert_eq!(
            verify(
                &signed("pid=1:time=99999999999999999999999:clear=1"),
                &token(),
                T
            ),
            Verdict::Ignored(Ignored::BadTime)
        );
        // Inside the window either side of now.
        for now in [T - 60, T + 60] {
            assert!(matches!(
                verify(&signed(&format!("pid=1:time={T}:clear=1")), &token(), now),
                Verdict::Accepted(_)
            ));
        }
        for now in [T - 61, T + 61] {
            assert_eq!(
                verify(&signed(&format!("pid=1:time={T}:clear=1")), &token(), now),
                Verdict::Ignored(Ignored::Stale)
            );
        }
    }

    #[test]
    fn authentic_but_invalid_reports_are_rejected() {
        let cases = [
            ("argv=Li9ydW4uc2g=", Rejected::InvalidProgram), // ./run.sh
            ("argv=bnZpbQ==:clear=1", Rejected::BadAction),
            ("argv=bnZpbQ==:cwd=cHJvamVjdA==", Rejected::RelativeCwd), // project
            ("argv=bnZpbQ==:need=later", Rejected::UnsupportedNeed),
            ("argv=", Rejected::EmptyArgv),
            ("argv=LWk=", Rejected::InvalidProgram), // -i
            ("argv=bnZpbQ=", Rejected::BadEncoding), // bad padding
            ("argv=bnZp*Q==", Rejected::MalformedPair),
            ("argv=bnZpbQ==:argv=bnZpbQ==", Rejected::RepeatedKey),
            ("argv=bnZpbQ==,YQpi", Rejected::ControlCharacter), // "a\nb"
            ("argv=bnZpbQ==,woU=", Rejected::ControlCharacter), // U+0085
            ("argv=/w==", Rejected::BadEncoding),               // not UTF-8
            ("clear=2", Rejected::BadClear),
            ("", Rejected::BadAction),
            ("Argv=bnZpbQ==", Rejected::MalformedPair),
            ("junk", Rejected::MalformedPair),
        ];
        for (tail, reason) in cases {
            let body = if tail.is_empty() {
                format!("pid=7:time={T}")
            } else {
                format!("pid=7:time={T}:{tail}")
            };
            assert_eq!(
                verify(&signed(&body), &token(), T),
                Verdict::Rejected {
                    pid: Some(7),
                    reason
                },
                "{tail}"
            );
        }
        assert_eq!(
            verify(&signed(&format!("time={T}:clear=1")), &token(), T),
            Verdict::Rejected {
                pid: None,
                reason: Rejected::BadPid
            }
        );
        let too_many = vec!["YQ=="; MAX_ARGV + 1].join(",");
        assert_eq!(
            verify(
                &signed(&format!("pid=7:time={T}:argv={too_many}")),
                &token(),
                T
            ),
            Verdict::Rejected {
                pid: Some(7),
                reason: Rejected::TooManyArgs
            }
        );
    }

    #[test]
    fn unknown_keys_are_ignored_and_known_needs_pass() {
        assert!(matches!(
            verify(
                &signed(&format!(
                    "pid=7:time={T}:argv=bnZpbQ==:later=x:need=cwd,argv"
                )),
                &token(),
                T
            ),
            Verdict::Accepted(_)
        ));
    }

    #[test]
    fn padding_is_optional_but_must_be_right() {
        assert_eq!(base64_decode("bnZpbQ").unwrap(), b"nvim");
        assert_eq!(base64_decode("bnZpbQ==").unwrap(), b"nvim");
        assert_eq!(base64_decode("").unwrap(), b"");
        assert!(base64_decode("bnZpbQ=").is_none());
        assert!(base64_decode("bnZpbQ===").is_none());
        assert!(base64_decode("b").is_none());
        assert!(base64_decode("bn=Z").is_none());
        for len in 0..40 {
            let data: Vec<u8> = (0..len as u8).map(|b| b.wrapping_mul(37)).collect();
            assert_eq!(base64_decode(&base64_encode(&data)).unwrap(), data);
        }
    }

    #[test]
    fn encoder_refuses_what_a_terminal_would_reject() {
        let t = token();
        let enc = |r: Report| r.time(T).encode(&t);
        assert_eq!(
            enc(Report::resume(Vec::<String>::new())),
            Err(ReportError::EmptyArgv)
        );
        assert_eq!(
            enc(Report::resume(["./run.sh"])),
            Err(ReportError::InvalidProgram)
        );
        assert_eq!(
            enc(Report::resume(["bin/run"])),
            Err(ReportError::InvalidProgram)
        );
        assert_eq!(
            enc(Report::resume(["-x"])),
            Err(ReportError::InvalidProgram)
        );
        assert_eq!(enc(Report::resume([""])), Err(ReportError::InvalidProgram));
        assert_eq!(
            enc(Report::resume(["vi", "a\nb"])),
            Err(ReportError::ControlCharacter)
        );
        assert_eq!(
            enc(Report::resume(["vi"]).cwd("project")),
            Err(ReportError::RelativeCwd)
        );
        assert_eq!(
            enc(Report::resume(vec!["a"; MAX_ARGV + 1])),
            Err(ReportError::TooManyArgs)
        );
        assert_eq!(
            enc(Report::resume(["vi", &"x".repeat(1600)])),
            Err(ReportError::TooLong)
        );
        assert_eq!(enc(Report::clear().pid(0)), Err(ReportError::InvalidPid));
        assert!(enc(Report::resume(vec!["a"; MAX_ARGV])).is_ok());
    }

    #[test]
    fn token_grammar_and_redaction() {
        assert!(Token::new("a".repeat(21)).is_none());
        assert!(Token::new("a".repeat(22)).is_some());
        assert!(Token::new("a".repeat(64)).is_some());
        assert!(Token::new("a".repeat(65)).is_none());
        assert!(Token::new("has space in the middle!!").is_none());
        assert_eq!(format!("{:?}", token()), "Token(<redacted>)");
        assert!(!format!("{:?}", Reporter::new(token())).contains(TOKEN));
    }

    #[test]
    fn reporter_sends_on_change_only_and_clears() {
        let mut r = Reporter::new(token());
        let mut out = Vec::new();
        let open = Report::resume(["app", "--open", "a.md"]).pid(9);
        r.update(&mut out, &open).unwrap();
        r.update(&mut out, &open).unwrap();
        assert_eq!(count(&out), 1, "unchanged command is not re-sent");

        r.update(&mut out, &Report::resume(["app", "--open", "b.md"]).pid(9))
            .unwrap();
        assert_eq!(count(&out), 2);

        r.refresh(&mut out).unwrap();
        assert_eq!(count(&out), 3, "refresh re-sends");

        r.clear(&mut out).unwrap();
        assert!(r.current().is_none());
        let reports = split(&out);
        assert_eq!(reports.len(), 4);
        let now = now();
        for seq in &reports[..3] {
            assert!(matches!(verify(seq, &token(), now), Verdict::Accepted(_)));
        }
        assert!(matches!(
            verify(reports[3], &token(), now),
            Verdict::Accepted(Record {
                pid: 9,
                action: Action::Clear,
                ..
            })
        ));

        // Clear when nothing is current is a no-op through `update`.
        let before = out.len();
        r.update(&mut out, &Report::clear()).unwrap();
        assert_eq!(out.len(), before);
    }

    #[test]
    fn reporter_clears_when_the_new_command_cannot_be_sent() {
        let mut r = Reporter::new(token());
        let mut out = Vec::new();
        r.send(&mut out, &Report::resume(["app", "a.md"])).unwrap();
        let err = r
            .send(&mut out, &Report::resume(["app", "line\nbreak"]))
            .unwrap_err();
        assert!(matches!(
            err,
            SendError::Invalid(ReportError::ControlCharacter)
        ));
        let reports = split(&out);
        assert_eq!(reports.len(), 2);
        assert!(matches!(
            verify(reports[1], &token(), now()),
            Verdict::Accepted(Record {
                action: Action::Clear,
                ..
            })
        ));
        assert!(r.current().is_none());
    }

    fn split(stream: &[u8]) -> Vec<&[u8]> {
        let mut out = Vec::new();
        let mut rest = stream;
        while let Some(start) = find(rest, PREFIX.as_bytes()) {
            let tail = &rest[start..];
            let end = find(tail, ST.as_bytes()).expect("terminated") + ST.len();
            out.push(&tail[..end]);
            rest = &tail[end..];
        }
        out
    }

    fn count(stream: &[u8]) -> usize {
        split(stream).len()
    }

    fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
        hay.windows(needle.len()).position(|w| w == needle)
    }
}
