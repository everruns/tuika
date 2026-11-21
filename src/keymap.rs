//! A host-agnostic keymap engine: declarative key bindings resolved to named
//! commands, with layered scoping, mode gating, and multi-stroke sequences.
//!
//! This is `tuika`'s answer to the same problem [OpenTUI's keymap][opentui]
//! solves for the browser/terminal: decouple *what keys do* from the widget
//! that reacts, so an application can declare its shortcuts once, gate them by
//! mode, discover them for help UIs, and dispatch through a single boundary. It
//! follows OpenTUI's **register → dispatch → query** shape, adapted to idiomatic
//! Rust and to `tuika`'s own [`Key`] events (so it needs no terminal and stays
//! unit-testable).
//!
//! [opentui]: https://opentui.com/docs/keymap/overview/
//!
//! # Model
//!
//! - A [`Chord`] is one key press plus its modifiers (`ctrl+r`, `enter`, `?`).
//!   Character tokens are exact logical text after keyboard-layout translation;
//!   Shift is written separately only for non-character keys.
//! - A [`KeySequence`] is one or more chords typed in order (`g g`, `ctrl+x s`).
//! - A [`Binding`] maps a sequence to a command value `C` (usually an app enum).
//! - A [`Layer`] groups bindings under a name and priority, optionally *gated*
//!   on runtime data so it is only active in a given mode.
//! - A [`Keymap`] holds the layers plus a small runtime-data store and the
//!   pending multi-stroke state, and turns incoming [`Key`]s into [`Dispatch`].
//! - A [`Mode`] is a named set of bindings (`"resize/left"`) that apply only
//!   while the mode is on the keymap's mode stack.
//! - [`CommandInfo`] entries form a searchable catalog of commands, and
//!   [`Keymap::diagnostics`] / [`Keymap::dump`] explain the bindings.
//!
//! # Example
//!
//! ```
//! use tuika::event::{Key, KeyCode};
//! use tuika::keymap::{Dispatch, Keymap, Layer};
//!
//! #[derive(Clone, Debug, PartialEq)]
//! enum Action { Search, Quit, Top }
//!
//! let mut keymap = Keymap::new().layer(
//!     Layer::new("global")
//!         .bind("ctrl+r", Action::Search)
//!         .bind("ctrl+d", Action::Quit)
//!         // A two-stroke sequence, vim-style.
//!         .bind("g g", Action::Top),
//! );
//!
//! // A single chord resolves immediately.
//! let ctrl_r = Key { code: KeyCode::Char('r'), ctrl: true, alt: false, shift: false };
//! assert_eq!(keymap.dispatch(ctrl_r), Dispatch::Command(Action::Search));
//!
//! // A sequence stays pending until it completes.
//! let g = Key::new(KeyCode::Char('g'));
//! assert_eq!(keymap.dispatch(g), Dispatch::Pending);
//! assert_eq!(keymap.dispatch(g), Dispatch::Command(Action::Top));
//! ```

use std::collections::HashMap;
use std::fmt;
use std::panic::Location;

use crate::event::{Key, KeyCode};
use crate::width::str_cols;

/// One key press plus its modifier state, normalized for binding lookups.
///
/// Modifier state is explicit, but character codes are exact logical text from
/// the active keyboard layout. For a [`KeyCode::Char`] the Shift flag is
/// *dropped*, because its effect is already folded into the character (`?`
/// arrives as `Char('?')`, not `Shift`+`Char('/')`). Shift remains meaningful
/// for non-character keys, where it is a distinct chord (`Shift`+`Enter`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    /// The key that was pressed.
    pub code: KeyCode,
    /// Ctrl held during the press.
    pub ctrl: bool,
    /// Alt held during the press.
    pub alt: bool,
    /// Shift held during the press (only meaningful for non-character keys).
    pub shift: bool,
}

impl Chord {
    /// A chord for `code` with no modifiers held.
    pub fn new(code: KeyCode) -> Self {
        Self {
            code,
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    /// Build a chord from a translated [`Key`] event, applying the character
    /// Shift-normalization described on [`Chord`].
    pub fn from_key(key: Key) -> Self {
        let shift = if matches!(key.code, KeyCode::Char(_)) {
            false
        } else {
            key.shift
        };
        Self {
            code: key.code,
            ctrl: key.ctrl,
            alt: key.alt,
            shift,
        }
    }

    /// Parse a single chord like `"ctrl+r"`, `"alt+shift+tab"`, `"enter"`, or
    /// `"?"`. Modifiers (`ctrl`/`control`, `alt`/`option`, `shift`) are joined to
    /// the key with `+`; the final segment is the key. A trailing `+` is the
    /// literal plus key (`"ctrl++"`).
    ///
    /// Character tokens are exact logical text. Spell a shifted character as
    /// the character it produces (`"A"`, `"?"`, `"ctrl+R"`), not as
    /// `"shift+a"` or `"shift+/"`: translating a physical key through Shift is
    /// keyboard-layout policy and therefore cannot be parsed portably here.
    pub fn parse(spec: &str) -> Result<Self, KeyParseError> {
        let spec = spec.trim();
        if spec.is_empty() {
            return Err(KeyParseError::new(spec));
        }
        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;

        // Split on '+' while treating a trailing '+' as the literal key. Walk the
        // segments; every segment but the last must be a known modifier.
        let mut key_token: Option<&str> = None;
        let segments = split_plus(spec);
        let last = segments.len().saturating_sub(1);
        for (index, segment) in segments.iter().enumerate() {
            if index == last {
                key_token = Some(segment);
                break;
            }
            match segment.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "alt" | "option" | "opt" | "meta" => alt = true,
                "shift" => shift = true,
                _ => return Err(KeyParseError::new(spec)),
            }
        }

        let key_token = key_token.ok_or_else(|| KeyParseError::new(spec))?;
        let mut code = parse_key_code(key_token).ok_or_else(|| KeyParseError::new(spec))?;
        if matches!(code, KeyCode::Char(_)) && shift {
            return Err(KeyParseError::shifted_character(spec));
        }
        // Terminals report Shift+Tab as a distinct `BackTab` key (as does
        // `from_key`), so fold the spec to match what an event will carry.
        if code == KeyCode::Tab && shift {
            code = KeyCode::BackTab;
            shift = false;
        }
        Ok(Self {
            code,
            ctrl,
            alt,
            shift,
        })
    }

    /// A human-readable label such as `"Ctrl+R"`, `"Shift+Tab"`, or `"Space"`,
    /// suitable for a [`KeyHints`](crate::components::KeyHints) row or a help overlay.
    pub fn display(&self) -> String {
        let mut out = String::new();
        if self.ctrl {
            out.push_str("Ctrl+");
        }
        if self.alt {
            out.push_str("Alt+");
        }
        if self.shift {
            out.push_str("Shift+");
        }
        // Preserve the established keycap-style label for lowercase ASCII
        // bindings while keeping an uppercase logical binding distinguishable.
        if !self.shift && matches!(self.code, KeyCode::Char(c) if c.is_ascii_uppercase()) {
            out.push_str("Shift+");
        }
        out.push_str(&key_code_label(self.code));
        out
    }
}

/// Split a chord spec on `+`, but treat a trailing `+` as a literal segment so
/// `"ctrl++"` parses as `["ctrl", "+"]` rather than `["ctrl", "", ""]`.
fn split_plus(spec: &str) -> Vec<&str> {
    if spec == "+" {
        return vec!["+"];
    }
    if let Some(prefix) = spec.strip_suffix('+') {
        // The literal '+' key, possibly with modifiers before it.
        let mut parts: Vec<&str> = prefix.split('+').filter(|s| !s.is_empty()).collect();
        parts.push("+");
        return parts;
    }
    spec.split('+').collect()
}

/// Map a key-name token (already isolated from its modifiers) to a [`KeyCode`].
fn parse_key_code(token: &str) -> Option<KeyCode> {
    // A single character (including a literal "+") is a `Char`, case-sensitive.
    let mut chars = token.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_control() {
            return None;
        }
        return Some(KeyCode::Char(c));
    }
    Some(match token.to_ascii_lowercase().as_str() {
        "enter" | "return" | "cr" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "backspace" | "bs" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "space" | "spc" => KeyCode::Char(' '),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        _ => return None,
    })
}

/// A display label for a bare key code (no modifiers).
fn key_code_label(code: KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) if c.is_ascii_alphabetic() => c.to_ascii_uppercase().to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::BackTab => "Shift+Tab".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
    }
}

/// A parse failure for a chord or sequence spec, carrying the offending text
/// and, for layout-dependent character input, an explanatory reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyParseError {
    spec: String,
    kind: KeyParseErrorKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyParseErrorKind {
    Invalid,
    ShiftedCharacter,
}

impl KeyParseError {
    fn new(spec: &str) -> Self {
        Self {
            spec: spec.to_string(),
            kind: KeyParseErrorKind::Invalid,
        }
    }

    fn shifted_character(spec: &str) -> Self {
        Self {
            spec: spec.to_string(),
            kind: KeyParseErrorKind::ShiftedCharacter,
        }
    }
}

impl fmt::Display for KeyParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid key spec: {:?}", self.spec)?;
        if self.kind == KeyParseErrorKind::ShiftedCharacter {
            f.write_str(
                ": character keys are logical text; use the resulting character instead of Shift",
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for KeyParseError {}

/// One or more [`Chord`]s typed in order — a single stroke (`ctrl+r`) or a
/// multi-stroke sequence (`g g`, `ctrl+x s`), written space-separated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeySequence(Vec<Chord>);

impl KeySequence {
    /// Parse a whitespace-separated sequence of chords. An empty spec is an
    /// error; a single chord yields a one-element sequence.
    pub fn parse(spec: &str) -> Result<Self, KeyParseError> {
        let chords = spec
            .split_whitespace()
            .map(Chord::parse)
            .collect::<Result<Vec<_>, _>>()?;
        if chords.is_empty() {
            return Err(KeyParseError::new(spec));
        }
        Ok(Self(chords))
    }

    /// The chords making up this sequence.
    pub fn chords(&self) -> &[Chord] {
        &self.0
    }

    /// A human-readable label, chords joined by spaces (`"Ctrl+X S"`).
    pub fn display(&self) -> String {
        self.0
            .iter()
            .map(Chord::display)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The token that stands for the keymap's leader key at the start of a spec
/// (`"<leader> f"`). See [`Keymap::leader`].
pub const LEADER: &str = "<leader>";

/// A binding from a [`KeySequence`] to a command value, with an optional label
/// used for help/hint surfaces and the source location that declared it.
#[derive(Clone, Debug)]
pub struct Binding<C> {
    sequence: KeySequence,
    // Whether the spec began with `<leader>`. The leader chord is resolved at
    // dispatch time rather than at parse time, so layers can be built before
    // (or without) the keymap that will own them, and changing the leader
    // rebinds every leader sequence at once.
    leader: bool,
    // The key mode this binding belongs to, or `None` for the base keymap.
    mode: Option<String>,
    command: C,
    label: Option<String>,
    location: Option<&'static Location<'static>>,
}

/// Split a `mode/` prefix off a spec (`"resize/left"`). A mode name is
/// lowercase ASCII letters, digits, `_`, and `-`, so the slash key itself
/// (`"/"`, `"ctrl+/"`, `"g /"`) is never mistaken for one.
fn split_mode(keys: &str) -> (Option<&str>, &str) {
    let trimmed = keys.trim_start();
    if let Some((name, rest)) = trimmed.split_once('/')
        && is_mode_name(name)
        && !rest.trim().is_empty()
    {
        return (Some(name), rest);
    }
    (None, keys)
}

fn is_mode_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// Split a leading [`LEADER`] token off a sequence spec.
fn split_leader(keys: &str) -> (bool, &str) {
    let trimmed = keys.trim_start();
    match trimmed.strip_prefix(LEADER) {
        Some(rest) if rest.is_empty() || rest.starts_with(char::is_whitespace) => (true, rest),
        _ => (false, keys),
    }
}

/// A named, prioritized group of bindings, optionally gated on runtime data so
/// it is only active in a given application mode.
///
/// Gating mirrors OpenTUI's data-driven layer activation: `when("mode",
/// "search")` makes the layer active only while the keymap's `"mode"` datum
/// equals `"search"`. A layer with no `when` clauses is always active.
#[derive(Clone, Debug)]
pub struct Layer<C> {
    name: String,
    priority: i32,
    when: Vec<(String, String)>,
    mode: Option<String>,
    bindings: Vec<Binding<C>>,
}

impl<C> Layer<C> {
    /// A new, always-active layer with priority `0`.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            priority: 0,
            when: Vec::new(),
            mode: None,
            bindings: Vec::new(),
        }
    }

    /// Put the bindings added after this call in key mode `mode`, as if each
    /// spec were written `"mode/keys"`. A spec's own `mode/` prefix still wins.
    ///
    /// # Panics
    ///
    /// Panics if `mode` is not a valid mode name (see [`Mode::new`]).
    #[track_caller]
    pub fn mode(mut self, mode: &str) -> Self {
        assert!(is_mode_name(mode), "invalid key mode name: {mode:?}");
        self.mode = Some(mode.to_string());
        self
    }

    /// This layer's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Set the layer priority. When two active layers both bind the same
    /// sequence, the higher priority wins.
    pub fn priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Gate this layer on a runtime-data value: it is active only while the
    /// keymap's datum for `key` equals `value`. Multiple `when` clauses must all
    /// match (logical AND).
    pub fn when(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.when.push((key.into(), value.into()));
        self
    }

    /// Add a binding, parsing `keys` as a [`KeySequence`].
    ///
    /// A spec may start with [`LEADER`] (`"<leader> f"`) to bind after the
    /// keymap's [leader key](Keymap::leader), and with a mode name and a slash
    /// (`"resize/left"`) to bind inside a [key mode](Mode). The caller's source location is
    /// recorded for [`Keymap::dump`] and [`Keymap::diagnostics`].
    ///
    /// # Panics
    ///
    /// Panics if `keys` is not a valid sequence spec. Bindings are normally
    /// authored as static literals, so a malformed spec is a programmer error;
    /// use [`Layer::try_bind`] to handle dynamic input fallibly.
    #[track_caller]
    pub fn bind(self, keys: &str, command: C) -> Self {
        self.try_bind(keys, command)
            .unwrap_or_else(|err| panic!("{err}"))
    }

    /// Add a binding with a help label, parsing `keys` as a [`KeySequence`].
    ///
    /// # Panics
    ///
    /// Panics if `keys` is not a valid sequence spec (see [`Layer::bind`]).
    #[track_caller]
    pub fn bind_labeled(self, keys: &str, label: impl Into<String>, command: C) -> Self {
        self.try_bind_labeled(keys, Some(label.into()), command)
            .unwrap_or_else(|err| panic!("{err}"))
    }

    /// Fallible [`Layer::bind`] for dynamically sourced specs.
    #[track_caller]
    pub fn try_bind(self, keys: &str, command: C) -> Result<Self, KeyParseError> {
        self.try_bind_labeled(keys, None, command)
    }

    #[track_caller]
    fn try_bind_labeled(
        mut self,
        keys: &str,
        label: Option<String>,
        command: C,
    ) -> Result<Self, KeyParseError> {
        let (mode, keys_only) = split_mode(keys);
        let mode = mode.map(str::to_string).or_else(|| self.mode.clone());
        let (leader, rest) = split_leader(keys_only);
        // `<leader>` alone would bind the leader chord itself, which then
        // could never start a sequence. Bind the chord directly instead.
        if leader && rest.trim().is_empty() {
            return Err(KeyParseError::new(keys));
        }
        let sequence = KeySequence::parse(rest)?;
        self.bindings.push(Binding {
            sequence,
            leader,
            mode,
            command,
            label,
            location: Some(Location::caller()),
        });
        Ok(self)
    }

    /// Whether this layer is active given the keymap's current runtime data.
    fn is_active(&self, data: &HashMap<String, String>) -> bool {
        self.when
            .iter()
            .all(|(key, value)| data.get(key).map(String::as_str) == Some(value.as_str()))
    }

    /// Whether some runtime data could make both layers active at once: they
    /// never gate the same key on different values.
    fn may_coexist(&self, other: &Self) -> bool {
        self.when.iter().all(|(key, value)| {
            other
                .when
                .iter()
                .all(|(other_key, other_value)| key != other_key || value == other_value)
        })
    }

    /// Whether this layer is active whenever `other` is: every gate of this
    /// layer is also a gate of `other`.
    fn active_whenever(&self, other: &Self) -> bool {
        self.when.iter().all(|gate| other.when.contains(gate))
    }

    fn gate_label(&self) -> String {
        if self.when.is_empty() {
            return "always active".to_string();
        }
        let gates = self
            .when
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(" ");
        format!("when {gates}")
    }
}

/// The outcome of feeding one key to a [`Keymap`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dispatch<C> {
    /// A binding matched fully; the host should run this command.
    Command(C),
    /// The keys typed so far are a prefix of one or more longer sequences; the
    /// keymap is waiting for the next stroke and has consumed this key.
    Pending,
    /// No active binding matches; the host should handle the key itself.
    Unmatched,
    /// The keymap used the key itself and the host should ignore it: Escape
    /// left a [key mode](Mode), or the key was blocked or unbound in an
    /// exclusive mode.
    Consumed,
}

/// A discoverable binding for a help surface: its key label, optional
/// description, and the command it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint<C> {
    /// The key sequence label, e.g. `"Ctrl+R"`.
    pub keys: String,
    /// The binding's help label, if one was given.
    pub label: Option<String>,
    /// The command the binding runs.
    pub command: C,
    /// Name of the active layer that declared this binding.
    pub layer: String,
    /// Layer priority, suitable for responsive hint fitting.
    pub priority: i32,
}

/// The keymap engine: a set of [`Layer`]s, a runtime-data store used to gate
/// them, and the pending multi-stroke state.
///
/// See the [module documentation](self) for the model and an example.
#[derive(Clone, Debug, Default)]
pub struct Keymap<C> {
    layers: Vec<Layer<C>>,
    data: HashMap<String, String>,
    pending: Vec<Chord>,
    leader: Option<Chord>,
    commands: Vec<CommandInfo<C>>,
    modes: Vec<Mode>,
    stack: Vec<ActiveMode>,
}

/// Options for a key mode: a named set of bindings that apply only while the
/// mode is entered (vim's normal mode, tmux's prefix table, a resize mode).
///
/// A mode exists as soon as a binding names it (`"resize/left"`), with the
/// defaults below; register a `Mode` with [`Keymap::mode`] only to change
/// them. Modes stack: [`Keymap::enter_mode`] pushes, the most recently entered
/// mode is checked first, and a key it does not bind falls through to the
/// modes below it and then the base keymap — unless the mode is
/// [`exclusive`](Self::exclusive).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mode {
    name: String,
    exclusive: bool,
    escape_exits: bool,
    blocked: Vec<Chord>,
}

impl Mode {
    /// Options for mode `name`: not exclusive, left with Escape, nothing
    /// blocked.
    ///
    /// # Panics
    ///
    /// Panics unless `name` is lowercase ASCII letters, digits, `_`, and `-`
    /// — the characters that keep `"name/key"` specs unambiguous.
    #[track_caller]
    pub fn new(name: &str) -> Self {
        assert!(is_mode_name(name), "invalid key mode name: {name:?}");
        Self {
            name: name.to_string(),
            exclusive: false,
            escape_exits: true,
            blocked: Vec::new(),
        }
    }

    /// The mode's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Only this mode's bindings work while it is on top: every other key
    /// returns [`Dispatch::Consumed`] instead of reaching lower modes, the
    /// base keymap, or the host.
    pub fn exclusive(mut self) -> Self {
        self.exclusive = true;
        self
    }

    /// Whether an Escape the mode does not bind leaves it (the default), or
    /// falls through like any other key.
    pub fn escape_exits(mut self, exits: bool) -> Self {
        self.escape_exits = exits;
        self
    }

    /// Block a key inside this mode: it does nothing there
    /// ([`Dispatch::Consumed`]), even though a lower mode or the base keymap
    /// binds it. Blocking applies to the first stroke of a sequence.
    ///
    /// # Panics
    ///
    /// Panics if `key` is not a valid single chord, like [`Layer::bind`].
    #[track_caller]
    pub fn block(mut self, key: &str) -> Self {
        let chord = Chord::parse(key).unwrap_or_else(|err| panic!("{err}"));
        self.blocked.push(chord);
        self
    }
}

/// An entry on the mode stack.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveMode {
    name: String,
    once: bool,
}

/// What resolving a candidate against the mode stack found.
enum Resolution<C> {
    Exact(C),
    Prefix,
    /// Nothing matched and the top mode swallows the key.
    Swallow,
    /// Nothing matched; the key belongs to the host.
    None,
}

/// How one binding relates to a candidate stroke sequence.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Match {
    /// The binding is exactly the candidate.
    Exact,
    /// The binding is strictly longer and starts with the candidate.
    Extends,
    /// Neither.
    No,
}

impl<C: Clone> Keymap<C> {
    /// An empty keymap.
    pub fn new() -> Self {
        Self {
            layers: Vec::new(),
            data: HashMap::new(),
            pending: Vec::new(),
            leader: None,
            commands: Vec::new(),
            modes: Vec::new(),
            stack: Vec::new(),
        }
    }

    /// Register options for a key mode (builder style). See [`Mode`].
    /// Registering a name twice replaces the earlier options.
    pub fn mode(mut self, mode: Mode) -> Self {
        self.add_mode(mode);
        self
    }

    /// Register options for a key mode in place.
    pub fn add_mode(&mut self, mode: Mode) {
        match self.modes.iter_mut().find(|known| known.name == mode.name) {
            Some(known) => *known = mode,
            None => self.modes.push(mode),
        }
    }

    /// Enter key mode `name`, on top of any mode already active. It stays
    /// until [`exit_mode`](Self::exit_mode), Escape, or
    /// [`exit_all_modes`](Self::exit_all_modes).
    ///
    /// Entering never fails: a mode with no bindings and no options is simply
    /// a mode in which only lower modes and the base keymap answer.
    pub fn enter_mode(&mut self, name: impl Into<String>) {
        self.push_mode(name.into(), false);
    }

    /// Enter key mode `name` for one key: the mode is left after the next key
    /// that does not leave a sequence pending, whatever that key did — the
    /// tmux prefix-key pattern.
    pub fn enter_mode_once(&mut self, name: impl Into<String>) {
        self.push_mode(name.into(), true);
    }

    fn push_mode(&mut self, name: String, once: bool) {
        self.stack.push(ActiveMode { name, once });
        self.pending.clear();
    }

    /// Leave the most recently entered mode, returning its name.
    pub fn exit_mode(&mut self) -> Option<String> {
        self.pending.clear();
        self.stack.pop().map(|mode| mode.name)
    }

    /// Leave every mode.
    pub fn exit_all_modes(&mut self) {
        self.stack.clear();
        self.pending.clear();
    }

    /// The most recently entered mode, for a mode indicator.
    pub fn current_mode(&self) -> Option<&str> {
        self.stack.last().map(|mode| mode.name.as_str())
    }

    /// Every active mode, oldest first.
    pub fn active_modes(&self) -> impl Iterator<Item = &str> {
        self.stack.iter().map(|mode| mode.name.as_str())
    }

    /// The options for mode `name`: the registered ones, or the defaults.
    fn mode_options(&self, name: &str) -> Option<&Mode> {
        self.modes.iter().find(|mode| mode.name == name)
    }

    /// The binding levels checked for a key, top first: each active mode from
    /// the most recent down, stopping after an exclusive one, then the base
    /// keymap (`None`) when no exclusive mode cut the walk short.
    fn levels(&self) -> Vec<Option<&str>> {
        let mut levels = Vec::with_capacity(self.stack.len() + 1);
        for active in self.stack.iter().rev() {
            levels.push(Some(active.name.as_str()));
            if self
                .mode_options(&active.name)
                .is_some_and(|mode| mode.exclusive)
            {
                return levels;
            }
        }
        levels.push(None);
        levels
    }

    /// Set the leader key (builder style), parsing `spec` as one chord.
    ///
    /// Bindings written `"<leader> f"` resolve to this chord followed by `f`.
    /// Until a leader is set those bindings never fire, and
    /// [`diagnostics`](Self::diagnostics) reports them.
    ///
    /// # Panics
    ///
    /// Panics if `spec` is not a valid single chord, like [`Layer::bind`].
    #[track_caller]
    pub fn leader(mut self, spec: &str) -> Self {
        let chord = Chord::parse(spec).unwrap_or_else(|err| panic!("{err}"));
        self.set_leader(Some(chord));
        self
    }

    /// Set or clear the leader key in place. Clears any pending sequence,
    /// since it may have started with the old leader.
    pub fn set_leader(&mut self, leader: Option<Chord>) {
        self.leader = leader;
        self.pending.clear();
    }

    /// The current leader key, if one is set.
    pub fn leader_key(&self) -> Option<Chord> {
        self.leader
    }

    /// Register a command in the catalog (builder style). See
    /// [`add_command`](Self::add_command).
    pub fn command(mut self, info: CommandInfo<C>) -> Self {
        self.add_command(info);
        self
    }

    /// Register a command in the catalog in place.
    ///
    /// The catalog is the searchable list of everything an application can
    /// do, bound to a key or not — what a command palette or a help screen
    /// reads. A command whose id is already registered replaces the earlier
    /// entry, so a host can override a default.
    pub fn add_command(&mut self, info: CommandInfo<C>) {
        match self.commands.iter_mut().find(|known| known.id == info.id) {
            Some(known) => *known = info,
            None => self.commands.push(info),
        }
    }

    /// Every registered command, in registration order.
    pub fn commands(&self) -> &[CommandInfo<C>] {
        &self.commands
    }

    /// The registered command with this id.
    pub fn find_command(&self, id: &str) -> Option<&CommandInfo<C>> {
        self.commands.iter().find(|info| info.id == id)
    }

    /// Add a layer (builder style).
    pub fn layer(mut self, layer: Layer<C>) -> Self {
        self.layers.push(layer);
        self
    }

    /// Add a layer in place.
    pub fn add_layer(&mut self, layer: Layer<C>) {
        self.layers.push(layer);
    }

    /// Set a runtime-data value, used to gate layers via [`Layer::when`].
    /// Changing data clears any pending multi-stroke sequence, since the
    /// bindings that could complete it may no longer be active.
    pub fn set_data(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.data.insert(key.into(), value.into());
        self.pending.clear();
    }

    /// Read a runtime-data value.
    pub fn data(&self, key: &str) -> Option<&str> {
        self.data.get(key).map(String::as_str)
    }

    /// The chords typed so far toward an unfinished multi-stroke sequence.
    pub fn pending(&self) -> &[Chord] {
        &self.pending
    }

    /// Abandon any pending multi-stroke sequence (e.g. on a focus change or a
    /// sequence-timeout tick).
    pub fn reset(&mut self) {
        self.pending.clear();
    }

    /// Feed one translated [`Key`] to the keymap and resolve it.
    ///
    /// Single-stroke bindings resolve immediately. For multi-stroke sequences
    /// the keymap accumulates strokes and returns [`Dispatch::Pending`] until a
    /// binding completes ([`Dispatch::Command`]) or the strokes dead-end
    /// ([`Dispatch::Unmatched`], after which the pending state is cleared and the
    /// final stroke is retried on its own so it can begin a fresh sequence).
    ///
    /// An exact match wins immediately even when it is also the prefix of a
    /// longer binding (so a bound `g` fires without waiting to see if `g g`
    /// follows). Author overlapping bindings accordingly.
    pub fn dispatch(&mut self, key: Key) -> Dispatch<C> {
        let chord = Chord::from_key(key);
        // A Ctrl or Alt chord typed on a non-Latin layout (Ctrl+`с`) is retried
        // as its QWERTY key (Ctrl+`c`) wherever the logical chord resolves to
        // nothing, so shortcuts work without switching layout. The logical
        // chord always wins, so an explicit `ctrl+с` binding still applies.
        let base = (!key.plain())
            .then(|| key.base_layout())
            .flatten()
            .map(Chord::from_key);
        let top = self.stack.len();
        let result = self.dispatch_chord(chord, base);
        // A once-mode lasts for one complete key. Leave it unless the key
        // left a sequence pending, or the dispatch already left it (Escape).
        if !matches!(result, Dispatch::Pending)
            && self.stack.len() == top
            && self.stack.last().is_some_and(|mode| mode.once)
        {
            self.stack.pop();
        }
        result
    }

    fn dispatch_chord(&mut self, chord: Chord, base: Option<Chord>) -> Dispatch<C> {
        if self.escape_leaves_mode(chord) {
            self.stack.pop();
            return Dispatch::Consumed;
        }
        let mut swallowed = false;
        let mut candidate = self.pending.clone();
        for stroke in std::iter::once(chord).chain(base) {
            candidate.push(stroke);
            match self.resolve_levels(&candidate) {
                Resolution::Exact(command) => {
                    self.pending.clear();
                    return Dispatch::Command(command);
                }
                Resolution::Prefix => {
                    self.pending = candidate;
                    return Dispatch::Pending;
                }
                Resolution::Swallow => swallowed = true,
                Resolution::None => {}
            }
            candidate.pop();
        }
        // The accumulated strokes dead-ended. Drop them and give this final
        // stroke a fresh chance to start its own sequence.
        let retry = !self.pending.is_empty();
        self.pending.clear();
        if retry {
            swallowed = false;
            for stroke in std::iter::once(chord).chain(base) {
                match self.resolve_levels(&[stroke]) {
                    Resolution::Exact(command) => return Dispatch::Command(command),
                    Resolution::Prefix => {
                        self.pending.push(stroke);
                        return Dispatch::Pending;
                    }
                    Resolution::Swallow => swallowed = true,
                    Resolution::None => {}
                }
            }
        }
        if swallowed {
            Dispatch::Consumed
        } else {
            Dispatch::Unmatched
        }
    }

    /// Whether this chord is a bare Escape that leaves the top mode: no
    /// sequence is pending, the mode keeps the default, and the mode does not
    /// bind Escape itself.
    fn escape_leaves_mode(&self, chord: Chord) -> bool {
        let Some(top) = self.stack.last() else {
            return false;
        };
        chord == Chord::new(KeyCode::Esc)
            && self.pending.is_empty()
            && self
                .mode_options(&top.name)
                .is_none_or(|mode| mode.escape_exits)
            && matches!(self.resolve(&[chord], Some(&top.name)), (None, false))
    }

    /// Walk the levels top first. The first level with an exact match or a
    /// live prefix answers; a blocked first stroke or an exclusive top mode
    /// swallows the key; otherwise it falls through to the next level.
    fn resolve_levels(&self, candidate: &[Chord]) -> Resolution<C> {
        for level in self.levels() {
            let (exact, prefix) = self.resolve(candidate, level);
            if let Some(command) = exact {
                return Resolution::Exact(command);
            }
            if prefix {
                return Resolution::Prefix;
            }
            let options = level.and_then(|name| self.mode_options(name));
            if let Some(mode) = options {
                if candidate.len() == 1 && mode.blocked.contains(&candidate[0]) {
                    return Resolution::Swallow;
                }
                if mode.exclusive {
                    return Resolution::Swallow;
                }
            }
        }
        Resolution::None
    }

    /// Resolve a candidate stroke sequence against the active layers' bindings
    /// at one level (a mode, or `None` for the base keymap), returning
    /// the winning exact-match command (highest priority) and whether any active
    /// binding is a strict extension of the candidate (a live prefix).
    fn resolve(&self, candidate: &[Chord], level: Option<&str>) -> (Option<C>, bool) {
        let mut best: Option<(i32, &C)> = None;
        let mut prefix = false;
        for layer in &self.layers {
            if !layer.is_active(&self.data) {
                continue;
            }
            for binding in &layer.bindings {
                if binding.mode.as_deref() != level {
                    continue;
                }
                match self.match_binding(binding, candidate) {
                    Match::Exact => {
                        if best.is_none_or(|(p, _)| layer.priority > p) {
                            best = Some((layer.priority, &binding.command));
                        }
                    }
                    Match::Extends => prefix = true,
                    Match::No => {}
                }
            }
        }
        (best.map(|(_, command)| command.clone()), prefix)
    }

    /// Compare a binding's effective strokes (leader resolved) with a
    /// candidate, without allocating: dispatch runs this for every binding on
    /// every key.
    fn match_binding(&self, binding: &Binding<C>, candidate: &[Chord]) -> Match {
        let rest = binding.sequence.chords();
        let head = match (binding.leader, self.leader) {
            (false, _) => None,
            (true, Some(leader)) => Some(leader),
            (true, None) => return Match::No,
        };
        let len = rest.len() + usize::from(head.is_some());
        if candidate.len() > len {
            return Match::No;
        }
        let stroke = |index: usize| match head {
            Some(leader) if index == 0 => leader,
            Some(_) => rest[index - 1],
            None => rest[index],
        };
        if !candidate
            .iter()
            .enumerate()
            .all(|(index, chord)| stroke(index) == *chord)
        {
            return Match::No;
        }
        if candidate.len() == len {
            Match::Exact
        } else {
            Match::Extends
        }
    }

    /// A binding's effective strokes, or `None` for a leader binding while no
    /// leader is set.
    fn effective(&self, binding: &Binding<C>) -> Option<Vec<Chord>> {
        let mut strokes = Vec::with_capacity(binding.sequence.chords().len() + 1);
        if binding.leader {
            strokes.push(self.leader?);
        }
        strokes.extend_from_slice(binding.sequence.chords());
        Some(strokes)
    }

    /// A binding's key label: the resolved leader when there is one, the
    /// literal `<leader>` token when there is not.
    fn keys_label(&self, binding: &Binding<C>) -> String {
        let rest = binding.sequence.display();
        match (binding.leader, self.leader) {
            (false, _) => rest,
            (true, Some(leader)) => format!("{} {rest}", leader.display()),
            (true, None) => format!("{LEADER} {rest}"),
        }
    }

    /// A binding's key label with its mode prefix (`"resize/Left"`), for
    /// surfaces that list every mode at once.
    fn qualified_keys(&self, binding: &Binding<C>) -> String {
        match &binding.mode {
            Some(mode) => format!("{mode}/{}", self.keys_label(binding)),
            None => self.keys_label(binding),
        }
    }

    fn site(&self, layer: &Layer<C>, binding: &Binding<C>) -> BindingSite {
        BindingSite {
            layer: layer.name.clone(),
            keys: self.qualified_keys(binding),
            location: binding.location,
        }
    }

    /// Every declared binding, active or not, in declaration order: the raw
    /// material for [`dump`](Self::dump) and for a host's own keymap viewer.
    pub fn bindings(&self) -> Vec<BindingInfo<C>> {
        let levels = self.levels();
        self.layers
            .iter()
            .flat_map(|layer| {
                let gate_open = layer.is_active(&self.data);
                let levels = &levels;
                layer.bindings.iter().map(move |binding| BindingInfo {
                    keys: self.keys_label(binding),
                    label: binding.label.clone(),
                    command: binding.command.clone(),
                    layer: layer.name.clone(),
                    priority: layer.priority,
                    when: layer.when.clone(),
                    mode: binding.mode.clone(),
                    active: gate_open && levels.contains(&binding.mode.as_deref()),
                    location: binding.location,
                })
            })
            .collect()
    }

    /// Bindings that can never fire, or whose winner depends only on
    /// declaration order — the mistakes that otherwise surface as a key that
    /// silently does nothing.
    ///
    /// The checks are static: they consider every runtime-data value, not
    /// just the current one, so they belong in a test or a debug-build
    /// startup check. A higher-priority gated layer overriding an always-on
    /// binding is intentional and not reported; the override is only a
    /// problem when the overriding layer is active whenever the other is.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let mut entries: Vec<(&Layer<C>, &Binding<C>, Vec<Chord>)> = Vec::new();
        for layer in &self.layers {
            for binding in &layer.bindings {
                match self.effective(binding) {
                    Some(strokes) => entries.push((layer, binding, strokes)),
                    None => out.push(Diagnostic {
                        kind: DiagnosticKind::MissingLeader,
                        binding: self.site(layer, binding),
                        by: None,
                    }),
                }
            }
        }
        for (index, (a_layer, a_binding, a_strokes)) in entries.iter().enumerate() {
            for (b_layer, b_binding, b_strokes) in &entries[index + 1..] {
                // Bindings in different modes never compete: a mode is
                // checked before the levels below it by design.
                if a_binding.mode != b_binding.mode || !a_layer.may_coexist(b_layer) {
                    continue;
                }
                let same_layer = std::ptr::eq(*a_layer, *b_layer);
                let found = if a_strokes == b_strokes {
                    if same_layer {
                        Some((
                            DiagnosticKind::Duplicate,
                            (b_layer, b_binding),
                            (a_layer, a_binding),
                        ))
                    } else {
                        // `resolve` keeps the first binding at the highest
                        // priority, so ties go to the earlier declaration.
                        let (winner, loser) = if b_layer.priority > a_layer.priority {
                            ((b_layer, b_binding), (a_layer, a_binding))
                        } else {
                            ((a_layer, a_binding), (b_layer, b_binding))
                        };
                        if winner.0.active_whenever(loser.0) {
                            Some((DiagnosticKind::Shadowed, loser, winner))
                        } else if a_layer.priority == b_layer.priority {
                            Some((DiagnosticKind::Ambiguous, loser, winner))
                        } else {
                            None
                        }
                    }
                } else {
                    // An exact match fires at once, so a binding that is a
                    // strict prefix of another starves it whenever both are
                    // active.
                    let (short, long) = if a_strokes.len() < b_strokes.len() {
                        (
                            (a_layer, a_binding, a_strokes),
                            (b_layer, b_binding, b_strokes),
                        )
                    } else {
                        (
                            (b_layer, b_binding, b_strokes),
                            (a_layer, a_binding, a_strokes),
                        )
                    };
                    (long.2.starts_with(short.2) && short.0.active_whenever(long.0)).then_some((
                        DiagnosticKind::Unreachable,
                        (long.0, long.1),
                        (short.0, short.1),
                    ))
                };
                if let Some((kind, (layer, binding), (by_layer, by_binding))) = found {
                    out.push(Diagnostic {
                        kind,
                        binding: self.site(layer, binding),
                        by: Some(self.site(by_layer, by_binding)),
                    });
                }
            }
        }
        out
    }

    /// The bindings currently active (their layer's gate is satisfied),
    /// suitable for building a [`KeyHints`](crate::components::KeyHints) row or a help
    /// overlay. Higher-priority layers come first.
    pub fn hints(&self) -> Vec<Hint<C>> {
        let mut layers: Vec<&Layer<C>> = self
            .layers
            .iter()
            .filter(|l| l.is_active(&self.data))
            .collect();
        layers.sort_by(|a, b| b.priority.cmp(&a.priority));
        let mut hints = Vec::new();
        // Keys blocked by a mode above hide the bindings below it.
        let mut blocked: Vec<Chord> = Vec::new();
        // The top mode's bindings first, so a help surface leads with what
        // the current mode adds.
        for level in self.levels() {
            for layer in &layers {
                let bindings = layer
                    .bindings
                    .iter()
                    .filter(|binding| binding.mode.as_deref() == level)
                    // A leader binding with no leader set cannot be typed, so
                    // advertising it would be a lie.
                    .filter(|binding| !binding.leader || self.leader.is_some())
                    .filter(|binding| {
                        self.effective(binding)
                            .and_then(|strokes| strokes.first().copied())
                            .is_none_or(|first| !blocked.contains(&first))
                    });
                hints.extend(bindings.map(|binding| Hint {
                    keys: self.keys_label(binding),
                    label: binding.label.clone(),
                    command: binding.command.clone(),
                    layer: layer.name.clone(),
                    priority: layer.priority,
                }));
            }
            if let Some(mode) = level.and_then(|name| self.mode_options(name)) {
                blocked.extend_from_slice(&mode.blocked);
            }
        }
        hints
    }
}

impl<C: Clone + fmt::Debug> Keymap<C> {
    /// Every binding, grouped by layer, with its label, command, and the
    /// source line that declared it, followed by the
    /// [diagnostics](Self::diagnostics) — a plain-text answer to "what does
    /// this key do, and where was that decided?". Print it from a debug flag
    /// or attach it to a bug report.
    ///
    /// The format is for people and may change between releases; read
    /// [`bindings`](Self::bindings) and [`diagnostics`](Self::diagnostics) to
    /// process it.
    pub fn dump(&self) -> String {
        use fmt::Write as _;
        let mut out = String::new();
        match self.leader {
            Some(leader) => {
                let _ = writeln!(out, "leader: {}", leader.display());
            }
            None => out.push_str("leader: (none)\n"),
        }
        let stack: Vec<&str> = self.active_modes().collect();
        if !stack.is_empty() {
            let _ = writeln!(out, "modes: {}", stack.join(" > "));
        }
        for layer in &self.layers {
            let state = if layer.is_active(&self.data) {
                ""
            } else {
                "  (inactive)"
            };
            let _ = writeln!(
                out,
                "\nlayer {:?}  priority {}  {}{state}",
                layer.name,
                layer.priority,
                layer.gate_label()
            );
            let rows: Vec<[String; 4]> = layer
                .bindings
                .iter()
                .map(|binding| {
                    [
                        self.qualified_keys(binding),
                        binding.label.clone().unwrap_or_else(|| "-".to_string()),
                        format!("{:?}", binding.command),
                        binding
                            .location
                            .map_or_else(String::new, |location| location.to_string()),
                    ]
                })
                .collect();
            let mut widths = [0usize; 3];
            for row in &rows {
                for (width, cell) in widths.iter_mut().zip(row) {
                    *width = (*width).max(usize::from(str_cols(cell)));
                }
            }
            for row in &rows {
                out.push(' ');
                for (cell, width) in row.iter().zip(widths) {
                    out.push(' ');
                    out.push_str(cell);
                    out.push_str(
                        &" ".repeat(width.saturating_sub(usize::from(str_cols(cell))) + 1),
                    );
                }
                out.push(' ');
                out.push_str(&row[3]);
                let trimmed = out.trim_end_matches(' ').len();
                out.truncate(trimmed);
                out.push('\n');
            }
        }
        if !self.modes.is_empty() {
            out.push_str("\nmode options:\n");
            for mode in &self.modes {
                let mut traits = Vec::new();
                if mode.exclusive {
                    traits.push("exclusive".to_string());
                }
                if !mode.escape_exits {
                    traits.push("escape does not exit".to_string());
                }
                if !mode.blocked.is_empty() {
                    let blocked: Vec<String> = mode.blocked.iter().map(Chord::display).collect();
                    traits.push(format!("blocked: {}", blocked.join(" ")));
                }
                if traits.is_empty() {
                    traits.push("defaults".to_string());
                }
                let _ = writeln!(out, "  {}  {}", mode.name, traits.join(", "));
            }
        }
        let diagnostics = self.diagnostics();
        if !diagnostics.is_empty() {
            let _ = writeln!(out, "\n{} warning(s):", diagnostics.len());
            for diagnostic in diagnostics {
                let _ = writeln!(out, "  {diagnostic}");
            }
        }
        out
    }
}

impl<C: Clone + PartialEq> Keymap<C> {
    /// Key labels of the active bindings that run `command`, highest-priority
    /// layer first — the shortcut column of a palette row.
    pub fn keys_for(&self, command: &C) -> Vec<String> {
        self.hints()
            .into_iter()
            .filter(|hint| hint.command == *command)
            .map(|hint| hint.keys)
            .collect()
    }

    /// Search the catalog with the same fuzzy ranking as
    /// [`CompletionState`](crate::components::CompletionState): a match on the
    /// title ranks first, then the id, keywords, category, and description.
    /// Ties keep registration order, and an empty query returns every
    /// command. Each match carries the keys that currently run it.
    pub fn search_commands(&self, query: &str) -> Vec<CommandMatch<'_, C>> {
        let mut ranked: Vec<(usize, usize)> = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(index, info)| info.score(query).map(|score| (score, index)))
            .collect();
        ranked.sort_unstable();
        ranked
            .into_iter()
            .map(|(score, index)| {
                let info = &self.commands[index];
                CommandMatch {
                    info,
                    keys: self.keys_for(&info.command),
                    score,
                }
            })
            .collect()
    }
}

/// A command in a keymap's catalog: a stable id, a title people search for,
/// and the command value it runs. Register it with [`Keymap::command`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CommandInfo<C> {
    /// Stable identifier, unique in a keymap (`"file.save"`).
    pub id: String,
    /// What a person searches for and reads (`"Save file"`).
    pub title: String,
    /// The command value dispatch would return for it.
    pub command: C,
    /// A longer explanation for a palette's detail line or a help screen.
    pub description: Option<String>,
    /// A group name for help screens (`"File"`).
    pub category: Option<String>,
    /// Extra search terms that are not in the title (`"write"`).
    pub keywords: Vec<String>,
}

impl<C> CommandInfo<C> {
    /// A catalog entry with no description, category, or keywords.
    pub fn new(id: impl Into<String>, title: impl Into<String>, command: C) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            command,
            description: None,
            category: None,
            keywords: Vec::new(),
        }
    }

    /// Set the description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the category.
    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Add a search keyword.
    pub fn keyword(mut self, keyword: impl Into<String>) -> Self {
        self.keywords.push(keyword.into());
        self
    }

    fn score(&self, query: &str) -> Option<usize> {
        let fields = [(Some(self.title.as_str()), 0), (Some(self.id.as_str()), 50)]
            .into_iter()
            .chain(
                self.keywords
                    .iter()
                    .map(|keyword| (Some(keyword.as_str()), 100)),
            )
            .chain([
                (self.category.as_deref(), 150),
                (self.description.as_deref(), 200),
            ]);
        fields
            .filter_map(|(field, penalty)| {
                crate::fuzzy::score(field?, query).map(|score| score.saturating_add(penalty))
            })
            .min()
    }
}

/// One result of [`Keymap::search_commands`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CommandMatch<'a, C> {
    /// The catalog entry.
    pub info: &'a CommandInfo<C>,
    /// Labels of the active bindings that run it, highest priority first.
    pub keys: Vec<String>,
    /// Rank, lower is better; only meaningful relative to the same query.
    pub score: usize,
}

/// Every declared binding as [`Keymap::bindings`] reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BindingInfo<C> {
    /// The key label, with the leader resolved (`"Space F"`).
    pub keys: String,
    /// The binding's help label, if one was given.
    pub label: Option<String>,
    /// The command it runs.
    pub command: C,
    /// The declaring layer's name.
    pub layer: String,
    /// The declaring layer's priority.
    pub priority: i32,
    /// The declaring layer's gates, as `(key, value)` pairs.
    pub when: Vec<(String, String)>,
    /// The key mode the binding belongs to, or `None` for the base keymap.
    pub mode: Option<String>,
    /// Whether the binding can fire now: its layer's gates hold and its mode
    /// (if any) is reachable on the mode stack.
    pub active: bool,
    /// The source line that declared the binding.
    pub location: Option<&'static Location<'static>>,
}

/// Where a binding was declared, for a [`Diagnostic`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BindingSite {
    /// The declaring layer's name.
    pub layer: String,
    /// The key label.
    pub keys: String,
    /// The source line that declared the binding.
    pub location: Option<&'static Location<'static>>,
}

impl fmt::Display for BindingSite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} in layer {:?}", self.keys, self.layer)?;
        if let Some(location) = self.location {
            write!(f, " ({location})")?;
        }
        Ok(())
    }
}

/// What a [`Diagnostic`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiagnosticKind {
    /// The same keys are bound twice in one layer; the later binding never
    /// fires.
    Duplicate,
    /// Another layer binds the same keys, wins on priority (or on
    /// declaration order at equal priority), and is active whenever this
    /// binding's layer is, so this binding never fires.
    Shadowed,
    /// Two layers that can be active together bind the same keys at the same
    /// priority, so the winner is decided only by declaration order.
    Ambiguous,
    /// A shorter binding is a prefix of this one and is active whenever this
    /// one is. Exact matches fire at once, so this sequence never completes.
    Unreachable,
    /// The binding starts with `<leader>` but the keymap has no leader key.
    MissingLeader,
}

/// A problem with a keymap's bindings, from [`Keymap::diagnostics`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Diagnostic {
    /// What was found.
    pub kind: DiagnosticKind,
    /// The binding that loses.
    pub binding: BindingSite,
    /// The binding that beats it, when another binding is involved.
    pub by: Option<BindingSite>,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let binding = &self.binding;
        match (&self.kind, &self.by) {
            (DiagnosticKind::Duplicate, Some(by)) => {
                write!(f, "{binding} never fires: {by} binds the same keys first")
            }
            (DiagnosticKind::Shadowed, Some(by)) => write!(
                f,
                "{binding} never fires: shadowed by {by}, which is active whenever it is"
            ),
            (DiagnosticKind::Ambiguous, Some(by)) => write!(
                f,
                "{binding} conflicts with {by} at the same priority; declaration order picks {by}"
            ),
            (DiagnosticKind::Unreachable, Some(by)) => {
                write!(f, "{binding} never completes: {by} fires on its prefix")
            }
            (DiagnosticKind::MissingLeader, _) => {
                write!(f, "{binding} never fires: the keymap has no leader key")
            }
            (kind, _) => write!(f, "{binding}: {kind:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Key, KeyCode};

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Action {
        Search,
        Quit,
        Top,
        Bottom,
        Close,
    }

    fn key(code: KeyCode) -> Key {
        Key::new(code)
    }

    fn ctrl(c: char) -> Key {
        Key {
            code: KeyCode::Char(c),
            ctrl: true,
            alt: false,
            shift: false,
        }
    }

    #[test]
    fn parses_modifier_chords() {
        assert_eq!(
            Chord::parse("ctrl+r").unwrap(),
            Chord {
                code: KeyCode::Char('r'),
                ctrl: true,
                alt: false,
                shift: false,
            }
        );
        // Shift+Tab folds to BackTab, the key terminals actually report.
        assert_eq!(
            Chord::parse("alt+shift+tab").unwrap(),
            Chord {
                code: KeyCode::BackTab,
                ctrl: false,
                alt: true,
                shift: false,
            }
        );
    }

    #[test]
    fn parses_named_and_literal_keys() {
        assert_eq!(Chord::parse("enter").unwrap().code, KeyCode::Enter);
        assert_eq!(Chord::parse("space").unwrap().code, KeyCode::Char(' '));
        assert_eq!(Chord::parse("?").unwrap().code, KeyCode::Char('?'));
        // A trailing '+' is the literal plus key, modifiers and all.
        assert_eq!(
            Chord::parse("ctrl++").unwrap(),
            Chord {
                code: KeyCode::Char('+'),
                ctrl: true,
                alt: false,
                shift: false,
            }
        );
    }

    #[test]
    fn character_chords_are_exact_logical_text() {
        let error = Chord::parse("shift+/").unwrap_err();
        assert!(error.to_string().contains("logical text"));
        assert!(Chord::parse("shift+a").is_err());
        assert!(Chord::parse("shift+space").is_err());

        assert_eq!(Chord::parse("?").unwrap().code, KeyCode::Char('?'));
        assert_eq!(Chord::parse("ж").unwrap().code, KeyCode::Char('ж'));
        assert_ne!(Chord::parse("r").unwrap(), Chord::parse("R").unwrap());

        // Translated events already contain the resulting character; an
        // independently reported Shift flag is not part of a text chord.
        let from = Chord::from_key(Key {
            code: KeyCode::Char('R'),
            ctrl: false,
            alt: false,
            shift: true,
        });
        assert_eq!(from.code, KeyCode::Char('R'));
        assert!(!from.shift);
    }

    #[test]
    fn rejects_garbage_specs() {
        assert!(Chord::parse("").is_err());
        assert!(Chord::parse("bogus+r").is_err());
        assert!(Chord::parse("\u{1b}").is_err());
        assert!(Chord::parse("\u{7}").is_err());
        assert!(KeySequence::parse("   ").is_err());
    }

    #[test]
    fn display_is_human_readable() {
        assert_eq!(Chord::parse("ctrl+r").unwrap().display(), "Ctrl+R");
        assert_eq!(Chord::parse("ctrl+R").unwrap().display(), "Ctrl+Shift+R");
        assert_eq!(Chord::parse("space").unwrap().display(), "Space");
        assert_eq!(Chord::parse("enter").unwrap().display(), "Enter");
        assert_eq!(
            KeySequence::parse("ctrl+x s").unwrap().display(),
            "Ctrl+X S"
        );
    }

    #[test]
    fn dispatches_single_chords() {
        let mut keymap = Keymap::new().layer(
            Layer::new("global")
                .bind("ctrl+r", Action::Search)
                .bind("ctrl+d", Action::Quit),
        );
        assert_eq!(
            keymap.dispatch(ctrl('r')),
            Dispatch::Command(Action::Search)
        );
        assert_eq!(keymap.dispatch(ctrl('d')), Dispatch::Command(Action::Quit));
        assert_eq!(keymap.dispatch(ctrl('z')), Dispatch::Unmatched);
    }

    #[test]
    fn dispatch_distinguishes_logical_character_case() {
        let mut keymap = Keymap::new().layer(
            Layer::new("global")
                .bind("ctrl+r", Action::Search)
                .bind("ctrl+R", Action::Close),
        );
        assert_eq!(
            keymap.dispatch(Key {
                code: KeyCode::Char('R'),
                ctrl: true,
                alt: false,
                shift: true,
            }),
            Dispatch::Command(Action::Close)
        );
        assert_eq!(
            keymap.dispatch(ctrl('r')),
            Dispatch::Command(Action::Search)
        );
    }

    #[test]
    fn resolves_multi_stroke_sequences() {
        let mut keymap = Keymap::new().layer(
            Layer::new("nav")
                .bind("g g", Action::Top)
                .bind("g e", Action::Bottom),
        );
        assert_eq!(keymap.dispatch(key(KeyCode::Char('g'))), Dispatch::Pending);
        assert_eq!(keymap.pending().len(), 1);
        assert_eq!(
            keymap.dispatch(key(KeyCode::Char('g'))),
            Dispatch::Command(Action::Top)
        );
        // Pending is cleared after a completed sequence.
        assert!(keymap.pending().is_empty());

        assert_eq!(keymap.dispatch(key(KeyCode::Char('g'))), Dispatch::Pending);
        assert_eq!(
            keymap.dispatch(key(KeyCode::Char('e'))),
            Dispatch::Command(Action::Bottom)
        );
    }

    #[test]
    fn dead_end_sequence_retries_final_stroke() {
        let mut keymap = Keymap::new().layer(
            Layer::new("nav")
                .bind("g g", Action::Top)
                .bind("ctrl+r", Action::Search),
        );
        // `g` starts a pending sequence; a following ctrl+r dead-ends `g g`, but
        // ctrl+r is itself a live binding and should fire.
        assert_eq!(keymap.dispatch(key(KeyCode::Char('g'))), Dispatch::Pending);
        assert_eq!(
            keymap.dispatch(ctrl('r')),
            Dispatch::Command(Action::Search)
        );
        assert!(keymap.pending().is_empty());
    }

    #[test]
    fn exact_match_wins_over_prefix() {
        // `g` is both a complete binding and the prefix of `g g`; the exact
        // match fires immediately without waiting.
        let mut keymap = Keymap::new().layer(
            Layer::new("nav")
                .bind("g", Action::Bottom)
                .bind("g g", Action::Top),
        );
        assert_eq!(
            keymap.dispatch(key(KeyCode::Char('g'))),
            Dispatch::Command(Action::Bottom)
        );
    }

    #[test]
    fn mode_gated_layers_activate_on_data() {
        let mut keymap = Keymap::new()
            .layer(Layer::new("global").bind("ctrl+d", Action::Quit))
            .layer(
                Layer::new("panel")
                    .when("mode", "panel")
                    .bind("q", Action::Close),
            );
        // The panel layer is dormant until the mode says so.
        assert_eq!(
            keymap.dispatch(key(KeyCode::Char('q'))),
            Dispatch::Unmatched
        );
        keymap.set_data("mode", "panel");
        assert_eq!(
            keymap.dispatch(key(KeyCode::Char('q'))),
            Dispatch::Command(Action::Close)
        );
        // The global layer stays active in every mode.
        assert_eq!(keymap.dispatch(ctrl('d')), Dispatch::Command(Action::Quit));
    }

    #[test]
    fn higher_priority_layer_wins_conflicts() {
        let mut keymap = Keymap::new()
            .layer(Layer::new("base").priority(0).bind("x", Action::Quit))
            .layer(Layer::new("over").priority(10).bind("x", Action::Close));
        assert_eq!(
            keymap.dispatch(key(KeyCode::Char('x'))),
            Dispatch::Command(Action::Close)
        );
    }

    #[test]
    fn changing_mode_clears_pending() {
        let mut keymap = Keymap::new().layer(Layer::new("nav").bind("g g", Action::Top));
        assert_eq!(keymap.dispatch(key(KeyCode::Char('g'))), Dispatch::Pending);
        keymap.set_data("mode", "other");
        assert!(keymap.pending().is_empty());
    }

    #[test]
    fn hints_list_active_bindings_by_priority() {
        let keymap = Keymap::new()
            .layer(Layer::new("global").priority(0).bind_labeled(
                "ctrl+r",
                "search",
                Action::Search,
            ))
            .layer(
                Layer::new("panel")
                    .priority(10)
                    .when("mode", "panel")
                    .bind_labeled("q", "close", Action::Close),
            );
        // Only the always-on layer contributes while no mode is set.
        let hints = keymap.hints();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].keys, "Ctrl+R");
        assert_eq!(hints[0].label.as_deref(), Some("search"));
        assert_eq!(hints[0].command, Action::Search);

        let mut keymap = keymap;
        keymap.set_data("mode", "panel");
        let hints = keymap.hints();
        // Higher-priority panel layer first.
        assert_eq!(hints.len(), 2);
        assert_eq!(hints[0].command, Action::Close);
        assert_eq!(hints[1].command, Action::Search);
    }

    #[test]
    fn leader_bindings_resolve_through_the_current_leader() {
        let mut keymap = Keymap::new().layer(
            Layer::new("global")
                .bind_labeled("<leader> f", "find", Action::Search)
                .bind("ctrl+d", Action::Quit),
        );
        // No leader yet: the binding is dormant and not advertised.
        assert_eq!(
            keymap.dispatch(key(KeyCode::Char(' '))),
            Dispatch::Unmatched
        );
        assert_eq!(keymap.hints().len(), 1);

        keymap.set_leader(Some(Chord::parse("space").unwrap()));
        assert_eq!(keymap.dispatch(key(KeyCode::Char(' '))), Dispatch::Pending);
        assert_eq!(
            keymap.dispatch(key(KeyCode::Char('f'))),
            Dispatch::Command(Action::Search)
        );
        assert_eq!(keymap.hints()[0].keys, "Space F");

        // Changing the leader rebinds every leader sequence at once.
        let mut keymap = keymap.leader("ctrl+x");
        assert_eq!(keymap.dispatch(ctrl('x')), Dispatch::Pending);
        assert_eq!(
            keymap.dispatch(key(KeyCode::Char('f'))),
            Dispatch::Command(Action::Search)
        );
        assert_eq!(keymap.leader_key(), Some(Chord::parse("ctrl+x").unwrap()));
    }

    #[test]
    fn leader_token_needs_a_following_key() {
        assert!(Layer::new("l").try_bind("<leader>", Action::Top).is_err());
        assert!(
            Layer::new("l")
                .try_bind("  <leader>  ", Action::Top)
                .is_err()
        );
        // Only a whole leading token counts.
        assert!(Layer::new("l").try_bind("<leader>f", Action::Top).is_err());
        assert!(Layer::new("l").try_bind("g <leader>", Action::Top).is_err());
    }

    #[test]
    fn bindings_record_their_source_line() {
        let line = line!() + 1;
        let keymap = Keymap::new().layer(Layer::new("global").bind("ctrl+r", Action::Search));
        let info = &keymap.bindings()[0];
        let location = info.location.expect("track_caller location");
        assert_eq!(location.file(), file!());
        assert_eq!(location.line(), line);
        assert_eq!(info.layer, "global");
        assert!(info.active);
    }

    #[test]
    fn bindings_list_inactive_layers_too() {
        let keymap = Keymap::new()
            .layer(Layer::new("global").bind("ctrl+d", Action::Quit))
            .layer(
                Layer::new("panel")
                    .priority(5)
                    .when("mode", "panel")
                    .bind_labeled("q", "close", Action::Close),
            );
        let all = keymap.bindings();
        assert_eq!(all.len(), 2);
        assert!(!all[1].active);
        assert_eq!(all[1].when, vec![("mode".to_string(), "panel".to_string())]);
        assert_eq!(all[1].priority, 5);
        assert_eq!(all[1].label.as_deref(), Some("close"));
    }

    fn kinds(keymap: &Keymap<Action>) -> Vec<(DiagnosticKind, String, Option<String>)> {
        keymap
            .diagnostics()
            .into_iter()
            .map(|d| (d.kind, d.binding.layer, d.by.map(|by| by.layer)))
            .collect()
    }

    #[test]
    fn diagnostics_find_duplicates_in_one_layer() {
        let keymap = Keymap::new().layer(
            Layer::new("global")
                .bind("x", Action::Quit)
                .bind("x", Action::Close),
        );
        let found = keymap.diagnostics();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, DiagnosticKind::Duplicate);
        // The later declaration is the dead one.
        let loser = found[0].binding.location.unwrap().line();
        let winner = found[0].by.as_ref().unwrap().location.unwrap().line();
        assert!(loser > winner);
    }

    #[test]
    fn diagnostics_separate_dead_shadows_from_intentional_overrides() {
        // An always-on overlay at higher priority kills the base binding.
        let dead = Keymap::new()
            .layer(Layer::new("base").bind("x", Action::Quit))
            .layer(Layer::new("over").priority(10).bind("x", Action::Close));
        assert_eq!(
            kinds(&dead),
            vec![(
                DiagnosticKind::Shadowed,
                "base".to_string(),
                Some("over".to_string())
            )]
        );

        // A gated overlay overriding a global key is the point of priorities.
        let intended = Keymap::new()
            .layer(Layer::new("base").bind("x", Action::Quit))
            .layer(
                Layer::new("over")
                    .priority(10)
                    .when("mode", "panel")
                    .bind("x", Action::Close),
            );
        assert!(intended.diagnostics().is_empty());

        // Layers gated on different values of one key are never co-active.
        let exclusive = Keymap::new()
            .layer(Layer::new("a").when("mode", "a").bind("x", Action::Quit))
            .layer(Layer::new("b").when("mode", "b").bind("x", Action::Close));
        assert!(exclusive.diagnostics().is_empty());
    }

    #[test]
    fn diagnostics_flag_equal_priority_conflicts_as_ambiguous() {
        let keymap = Keymap::new()
            .layer(
                Layer::new("a")
                    .when("focus", "list")
                    .bind("x", Action::Quit),
            )
            .layer(
                Layer::new("b")
                    .when("mode", "panel")
                    .bind("x", Action::Close),
            );
        assert_eq!(
            kinds(&keymap),
            vec![(
                DiagnosticKind::Ambiguous,
                "b".to_string(),
                Some("a".to_string())
            )]
        );
        // Equal priority and the winner always active is a dead binding.
        let keymap = Keymap::new()
            .layer(Layer::new("a").bind("x", Action::Quit))
            .layer(
                Layer::new("b")
                    .when("mode", "panel")
                    .bind("x", Action::Close),
            );
        assert_eq!(kinds(&keymap)[0].0, DiagnosticKind::Shadowed);
    }

    #[test]
    fn diagnostics_find_sequences_starved_by_a_prefix() {
        let keymap = Keymap::new().layer(
            Layer::new("nav")
                .bind("g g", Action::Top)
                .bind("g", Action::Bottom),
        );
        let found = keymap.diagnostics();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, DiagnosticKind::Unreachable);
        assert_eq!(found[0].binding.keys, "G G");
        assert_eq!(found[0].by.as_ref().unwrap().keys, "G");

        // The leader chord bound on its own starves every leader sequence.
        let keymap = Keymap::new()
            .leader("space")
            .layer(Layer::new("global").bind("<leader> f", Action::Search))
            .layer(Layer::new("other").bind("space", Action::Top));
        assert_eq!(kinds(&keymap)[0].0, DiagnosticKind::Unreachable);
    }

    #[test]
    fn diagnostics_report_leader_bindings_without_a_leader() {
        let keymap = Keymap::new().layer(Layer::new("global").bind("<leader> f", Action::Search));
        let found = keymap.diagnostics();
        assert_eq!(found[0].kind, DiagnosticKind::MissingLeader);
        assert_eq!(found[0].binding.keys, "<leader> F");
        assert!(found[0].to_string().contains("no leader key"));
    }

    #[test]
    fn clean_keymap_has_no_diagnostics() {
        let keymap = Keymap::new()
            .leader("space")
            .layer(
                Layer::new("global")
                    .bind("ctrl+r", Action::Search)
                    .bind("<leader> q", Action::Quit)
                    .bind("g g", Action::Top)
                    .bind("g e", Action::Bottom),
            )
            .layer(
                Layer::new("panel")
                    .priority(5)
                    .when("mode", "panel")
                    .bind("ctrl+r", Action::Close),
            );
        assert_eq!(keymap.diagnostics(), Vec::new());
    }

    #[test]
    fn dump_lists_layers_bindings_sources_and_warnings() {
        let keymap = Keymap::new()
            .leader("space")
            .layer(
                Layer::new("global")
                    .bind_labeled("ctrl+r", "search", Action::Search)
                    .bind("<leader> q", Action::Quit),
            )
            .layer(
                Layer::new("panel")
                    .priority(10)
                    .when("mode", "panel")
                    .bind("q", Action::Close)
                    .bind("q", Action::Top),
            );
        let dump = keymap.dump();
        let lines: Vec<&str> = dump.lines().collect();
        assert_eq!(lines[0], "leader: Space");
        assert_eq!(lines[2], "layer \"global\"  priority 0  always active");
        // `file!()` rather than a literal path: Windows spells it with a backslash.
        let here = file!();
        assert!(lines[3].starts_with(&format!("  Ctrl+R   search  Search  {here}:")));
        assert!(lines[4].starts_with(&format!("  Space Q  -       Quit    {here}:")));
        assert_eq!(
            lines[6],
            "layer \"panel\"  priority 10  when mode=panel  (inactive)"
        );
        assert!(dump.contains("1 warning(s):"));
        assert!(dump.contains(&format!("\"Q\" in layer \"panel\" ({here}:")));
        assert!(lines.iter().all(|line| !line.ends_with(' ')));
    }

    fn catalog() -> Keymap<Action> {
        Keymap::new()
            .layer(
                Layer::new("global")
                    .bind("ctrl+r", Action::Search)
                    .bind("/", Action::Search)
                    .bind("ctrl+d", Action::Quit),
            )
            .layer(
                Layer::new("panel")
                    .priority(5)
                    .when("mode", "panel")
                    .bind("q", Action::Close),
            )
            .command(
                CommandInfo::new("app.search", "Search messages", Action::Search)
                    .category("Navigation")
                    .keyword("find"),
            )
            .command(CommandInfo::new("app.quit", "Quit", Action::Quit).description("Exit the app"))
            .command(CommandInfo::new(
                "panel.close",
                "Close panel",
                Action::Close,
            ))
            .command(CommandInfo::new("nav.top", "Go to top", Action::Top))
    }

    #[test]
    fn catalog_registers_finds_and_replaces_commands() {
        let mut keymap = catalog();
        assert_eq!(keymap.commands().len(), 4);
        assert_eq!(keymap.find_command("app.quit").unwrap().title, "Quit");
        keymap.add_command(CommandInfo::new("app.quit", "Exit", Action::Quit));
        assert_eq!(keymap.commands().len(), 4);
        assert_eq!(keymap.find_command("app.quit").unwrap().title, "Exit");
        assert!(keymap.find_command("missing").is_none());
    }

    #[test]
    fn keys_for_lists_active_bindings_only() {
        let mut keymap = catalog();
        assert_eq!(keymap.keys_for(&Action::Search), vec!["Ctrl+R", "/"]);
        assert!(keymap.keys_for(&Action::Close).is_empty());
        keymap.set_data("mode", "panel");
        assert_eq!(keymap.keys_for(&Action::Close), vec!["Q"]);
        assert!(keymap.keys_for(&Action::Top).is_empty());
    }

    #[test]
    fn search_ranks_title_before_other_fields() {
        let keymap = catalog();
        let all = keymap.search_commands("");
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].info.id, "app.search");

        let ids = |query: &str| -> Vec<String> {
            keymap
                .search_commands(query)
                .into_iter()
                .map(|m| m.info.id.clone())
                .collect()
        };
        assert_eq!(ids("close"), vec!["panel.close"]);
        // Keyword, category, and description all count, below the title.
        assert_eq!(ids("find"), vec!["app.search"]);
        assert_eq!(ids("navigation"), vec!["app.search"]);
        assert_eq!(ids("exit"), vec!["app.quit"]);
        // A title prefix beats a description substring.
        let ranked = ids("to");
        assert_eq!(ranked[0], "nav.top");
        assert!(ids("zzz").is_empty());

        let search = &keymap.search_commands("search")[0];
        assert_eq!(search.keys, vec!["Ctrl+R", "/"]);
    }

    #[test]
    fn non_latin_layout_chords_fall_back_to_the_qwerty_key() {
        let mut keymap = Keymap::new().layer(
            Layer::new("global")
                .bind("ctrl+r", Action::Search)
                .bind("q", Action::Quit)
                .bind("ctrl+x ctrl+b", Action::Bottom),
        );
        // Ctrl+к is Ctrl+R's key on a Ukrainian/Russian layout.
        assert_eq!(
            keymap.dispatch(ctrl('к')),
            Dispatch::Command(Action::Search)
        );
        // Sequences fall back stroke by stroke.
        assert_eq!(keymap.dispatch(ctrl('ч')), Dispatch::Pending);
        assert_eq!(
            keymap.dispatch(ctrl('и')),
            Dispatch::Command(Action::Bottom)
        );
        // Plain characters are text: `й` must not quit through `q`.
        assert_eq!(
            keymap.dispatch(key(KeyCode::Char('й'))),
            Dispatch::Unmatched
        );
        // Greek too.
        assert_eq!(
            keymap.dispatch(ctrl('ρ')),
            Dispatch::Command(Action::Search)
        );
    }

    #[test]
    fn an_explicit_non_latin_binding_wins_over_the_fallback() {
        let mut keymap = Keymap::new().layer(
            Layer::new("global")
                .bind("ctrl+r", Action::Search)
                .bind("ctrl+к", Action::Close),
        );
        assert_eq!(keymap.dispatch(ctrl('к')), Dispatch::Command(Action::Close));
        assert_eq!(
            keymap.dispatch(ctrl('r')),
            Dispatch::Command(Action::Search)
        );
    }

    #[test]
    fn a_dead_end_sequence_retries_the_final_stroke_on_its_base_key() {
        let mut keymap = Keymap::new().layer(
            Layer::new("global")
                .bind("g g", Action::Top)
                .bind("ctrl+r", Action::Search),
        );
        assert_eq!(keymap.dispatch(key(KeyCode::Char('g'))), Dispatch::Pending);
        assert_eq!(
            keymap.dispatch(ctrl('к')),
            Dispatch::Command(Action::Search)
        );
        assert!(keymap.pending().is_empty());
    }

    fn left() -> Key {
        key(KeyCode::Left)
    }

    fn modal() -> Keymap<Action> {
        Keymap::new().layer(
            Layer::new("global")
                .bind("ctrl+r", Action::Search)
                .bind("q", Action::Quit)
                .bind("resize/left", Action::Top)
                .bind("resize/enter", Action::Close),
        )
    }

    #[test]
    fn mode_bindings_only_apply_inside_the_mode() {
        let mut keymap = modal();
        assert_eq!(keymap.dispatch(left()), Dispatch::Unmatched);
        keymap.enter_mode("resize");
        assert_eq!(keymap.current_mode(), Some("resize"));
        assert_eq!(keymap.dispatch(left()), Dispatch::Command(Action::Top));
        // Unbound keys fall through to the base keymap by default, and the
        // mode stays until it is left.
        assert_eq!(
            keymap.dispatch(ctrl('r')),
            Dispatch::Command(Action::Search)
        );
        assert_eq!(keymap.dispatch(left()), Dispatch::Command(Action::Top));
        assert_eq!(keymap.exit_mode().as_deref(), Some("resize"));
        assert_eq!(keymap.dispatch(left()), Dispatch::Unmatched);
    }

    #[test]
    fn mode_prefix_is_not_confused_with_the_slash_key() {
        let keymap = Keymap::new().layer(
            Layer::new("global")
                .bind("/", Action::Search)
                .bind("ctrl+/", Action::Quit)
                .bind("g /", Action::Top),
        );
        assert!(keymap.bindings().iter().all(|b| b.mode.is_none()));
        assert!(
            Layer::new("l")
                .try_bind("Resize/left", Action::Top)
                .is_err()
        );
        assert!(Layer::new("l").try_bind("resize/", Action::Top).is_err());
        let layer = Layer::new("l")
            .mode("nav")
            .bind("j", Action::Top)
            .bind("other/k", Action::Close);
        let keymap = Keymap::new().layer(layer);
        let modes: Vec<_> = keymap.bindings().into_iter().map(|b| b.mode).collect();
        assert_eq!(
            modes,
            vec![Some("nav".to_string()), Some("other".to_string())]
        );
    }

    #[test]
    fn escape_leaves_the_mode_unless_bound_or_disabled() {
        let mut keymap = modal();
        keymap.enter_mode("resize");
        assert_eq!(keymap.dispatch(key(KeyCode::Esc)), Dispatch::Consumed);
        assert_eq!(keymap.current_mode(), None);
        // Outside any mode Escape is the host's.
        assert_eq!(keymap.dispatch(key(KeyCode::Esc)), Dispatch::Unmatched);

        let mut keymap = modal().layer(Layer::new("m").bind("resize/esc", Action::Bottom));
        keymap.enter_mode("resize");
        assert_eq!(
            keymap.dispatch(key(KeyCode::Esc)),
            Dispatch::Command(Action::Bottom)
        );
        assert_eq!(keymap.current_mode(), Some("resize"));

        let mut keymap = modal().mode(Mode::new("resize").escape_exits(false));
        keymap.enter_mode("resize");
        assert_eq!(keymap.dispatch(key(KeyCode::Esc)), Dispatch::Unmatched);
        assert_eq!(keymap.current_mode(), Some("resize"));
    }

    #[test]
    fn exclusive_modes_swallow_unbound_keys() {
        let mut keymap = modal().mode(Mode::new("resize").exclusive());
        keymap.enter_mode("resize");
        assert_eq!(keymap.dispatch(ctrl('r')), Dispatch::Consumed);
        assert_eq!(keymap.dispatch(key(KeyCode::Char('x'))), Dispatch::Consumed);
        assert_eq!(keymap.dispatch(left()), Dispatch::Command(Action::Top));
        assert_eq!(keymap.dispatch(key(KeyCode::Esc)), Dispatch::Consumed);
        assert_eq!(
            keymap.dispatch(ctrl('r')),
            Dispatch::Command(Action::Search)
        );
    }

    #[test]
    fn blocked_keys_do_nothing_inside_the_mode() {
        let mut keymap = modal().mode(Mode::new("resize").block("q"));
        keymap.enter_mode("resize");
        assert_eq!(keymap.dispatch(key(KeyCode::Char('q'))), Dispatch::Consumed);
        assert_eq!(
            keymap.dispatch(ctrl('r')),
            Dispatch::Command(Action::Search)
        );
        assert!(
            keymap
                .hints()
                .iter()
                .all(|hint| hint.command != Action::Quit)
        );
        keymap.exit_mode();
        assert_eq!(
            keymap.dispatch(key(KeyCode::Char('q'))),
            Dispatch::Command(Action::Quit)
        );
    }

    #[test]
    fn once_modes_leave_after_one_complete_key() {
        let mut keymap = modal().layer(Layer::new("seq").bind("resize/g g", Action::Bottom));
        keymap.enter_mode_once("resize");
        assert_eq!(keymap.dispatch(left()), Dispatch::Command(Action::Top));
        assert_eq!(keymap.current_mode(), None);

        // A pending sequence keeps the mode until the sequence completes.
        keymap.enter_mode_once("resize");
        assert_eq!(keymap.dispatch(key(KeyCode::Char('g'))), Dispatch::Pending);
        assert_eq!(keymap.current_mode(), Some("resize"));
        assert_eq!(
            keymap.dispatch(key(KeyCode::Char('g'))),
            Dispatch::Command(Action::Bottom)
        );
        assert_eq!(keymap.current_mode(), None);

        // An unbound key ends it too, and still reaches the host.
        keymap.enter_mode_once("resize");
        assert_eq!(
            keymap.dispatch(key(KeyCode::Char('x'))),
            Dispatch::Unmatched
        );
        assert_eq!(keymap.current_mode(), None);

        // Escape leaves only the once-mode, not the one below it.
        keymap.enter_mode("other");
        keymap.enter_mode_once("resize");
        assert_eq!(keymap.dispatch(key(KeyCode::Esc)), Dispatch::Consumed);
        assert_eq!(keymap.current_mode(), Some("other"));
    }

    #[test]
    fn nested_modes_check_the_newest_first_and_fall_through() {
        let mut keymap = Keymap::new().layer(
            Layer::new("global")
                .bind("a/x", Action::Top)
                .bind("a/y", Action::Bottom)
                .bind("b/x", Action::Close)
                .bind("z", Action::Quit),
        );
        keymap.enter_mode("a");
        keymap.enter_mode("b");
        assert_eq!(keymap.active_modes().collect::<Vec<_>>(), vec!["a", "b"]);
        assert_eq!(
            keymap.dispatch(key(KeyCode::Char('x'))),
            Dispatch::Command(Action::Close)
        );
        assert_eq!(
            keymap.dispatch(key(KeyCode::Char('y'))),
            Dispatch::Command(Action::Bottom)
        );
        assert_eq!(
            keymap.dispatch(key(KeyCode::Char('z'))),
            Dispatch::Command(Action::Quit)
        );
        // Hints lead with the newest mode.
        let hints: Vec<Action> = keymap.hints().into_iter().map(|h| h.command).collect();
        assert_eq!(
            hints,
            vec![Action::Close, Action::Top, Action::Bottom, Action::Quit]
        );

        // An exclusive mode in the middle stops the walk there.
        let mut keymap = keymap.mode(Mode::new("a").exclusive());
        keymap.enter_mode("a");
        keymap.enter_mode("b");
        assert_eq!(
            keymap.dispatch(key(KeyCode::Char('y'))),
            Dispatch::Command(Action::Bottom)
        );
        assert_eq!(keymap.dispatch(key(KeyCode::Char('z'))), Dispatch::Consumed);
        keymap.exit_all_modes();
        assert_eq!(keymap.current_mode(), None);
    }

    #[test]
    fn entering_or_leaving_a_mode_clears_pending() {
        let mut keymap = Keymap::new().layer(Layer::new("nav").bind("g g", Action::Top));
        assert_eq!(keymap.dispatch(key(KeyCode::Char('g'))), Dispatch::Pending);
        keymap.enter_mode("x");
        assert!(keymap.pending().is_empty());
    }

    #[test]
    fn diagnostics_and_dump_understand_modes() {
        let keymap = Keymap::new()
            .layer(
                Layer::new("global")
                    .bind("x", Action::Quit)
                    .bind("resize/x", Action::Top)
                    .bind("resize/x", Action::Close),
            )
            .mode(Mode::new("resize").exclusive().block("q"));
        // Same keys in different modes are not a conflict; a duplicate inside
        // one mode is.
        let found = keymap.diagnostics();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, DiagnosticKind::Duplicate);
        assert_eq!(found[0].binding.keys, "resize/X");

        let infos = keymap.bindings();
        assert!(infos[0].active);
        assert!(!infos[1].active);

        let dump = keymap.dump();
        assert!(dump.contains("  resize/X  -  Top"));
        assert!(dump.contains("mode options:\n  resize  exclusive, blocked: Q\n"));
        let mut keymap = keymap;
        keymap.enter_mode("resize");
        assert!(keymap.dump().contains("modes: resize\n"));
        assert!(keymap.bindings()[1].active);
        assert!(!keymap.bindings()[0].active);
    }

    #[test]
    fn non_latin_fallback_works_inside_modes() {
        let mut keymap = modal()
            .layer(Layer::new("m").bind("resize/ctrl+r", Action::Bottom))
            .mode(Mode::new("resize").exclusive());
        keymap.enter_mode("resize");
        // Ctrl+к reaches the mode's Ctrl+R before the exclusive mode swallows it.
        assert_eq!(
            keymap.dispatch(ctrl('к')),
            Dispatch::Command(Action::Bottom)
        );
        assert_eq!(keymap.dispatch(ctrl('й')), Dispatch::Consumed);
    }
}
