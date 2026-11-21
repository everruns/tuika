//! Input events, decoupled from crossterm.
//!
//! Components handle `tuika` events, not raw crossterm events, so the widget
//! layer stays testable without a terminal. The host ([`super::host`])
//! translates crossterm into these. This mirrors Codex's event-stream input
//! model: a small, explicit event enum flowing to focused components.

/// A translated input event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A key press.
    Key(Key),
    /// A mouse action.
    Mouse(Mouse),
    /// Bracketed-paste payload.
    Paste(String),
    /// Terminal resized to the given cell dimensions.
    Resize {
        /// New width in cells.
        width: u16,
        /// New height in cells.
        height: u16,
    },
}

/// A key press with its modifier state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    /// The logical key value that was pressed.
    pub code: KeyCode,
    /// Ctrl held during the press.
    pub ctrl: bool,
    /// Alt held during the press.
    pub alt: bool,
    /// Shift held during the press.
    pub shift: bool,
}

impl Key {
    /// A key with the given code and no modifiers held.
    pub fn new(code: KeyCode) -> Self {
        Self {
            code,
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    /// True when neither Ctrl nor Alt is held (Shift is ignored, as it is
    /// already folded into the character for text input).
    pub fn plain(&self) -> bool {
        !self.ctrl && !self.alt
    }

    /// The same key on the US QWERTY layout, when its character comes from a
    /// known non-Latin layout: Ctrl+`с` on a Ukrainian or Russian keyboard
    /// becomes Ctrl+`c`, the key in the same position.
    ///
    /// Shortcuts are written against Latin letters, but a [`KeyCode::Char`] is
    /// the logical character the active layout produced. With enhanced keyboard
    /// reporting a terminal sends Ctrl+`с` as exactly that, so a `ctrl+c`
    /// binding misses unless the user switches layout first. The [`Keymap`]
    /// and the built-in components therefore retry an unmatched Ctrl or Alt
    /// chord with this key. Plain characters are text and are never translated
    /// by them.
    ///
    /// Covers the Cyrillic layouts (Ukrainian, Russian, Belarusian), Greek, and
    /// Hebrew; returns `None` for any other character or key. Modifiers are
    /// kept, and case follows the character (`С` becomes `C`).
    ///
    /// The Kitty keyboard protocol can report the base-layout key directly, for
    /// every layout, but crossterm discards that field; this table stands in
    /// for it.
    ///
    /// [`Keymap`]: crate::keymap::Keymap
    pub fn base_layout(self) -> Option<Key> {
        let KeyCode::Char(c) = self.code else {
            return None;
        };
        let latin = qwerty_equivalent(c)?;
        Some(Key {
            code: KeyCode::Char(latin),
            ..self
        })
    }

    /// [`base_layout`](Self::base_layout) for a Ctrl or Alt chord, otherwise
    /// the key unchanged — what a component matching fixed Latin shortcuts
    /// compares against.
    pub fn shortcut(self) -> Key {
        if self.plain() {
            self
        } else {
            self.base_layout().unwrap_or(self)
        }
    }
}

/// The US QWERTY character at the physical position that produces `c` on the
/// standard (PC) Cyrillic, Greek, or Hebrew layouts. Shifted letters map to
/// shifted Latin; positions whose layouts disagree (Ukrainian `ґ`) are left out.
fn qwerty_equivalent(c: char) -> Option<char> {
    let latin = match c {
        // Cyrillic, ЙЦУКЕН. Ukrainian і/ї/є and Belarusian і/ў share the keys
        // of Russian ы/ъ/э and щ.
        'й' => 'q',
        'ц' => 'w',
        'у' => 'e',
        'к' => 'r',
        'е' => 't',
        'н' => 'y',
        'г' => 'u',
        'ш' => 'i',
        'щ' | 'ў' => 'o',
        'з' => 'p',
        'х' => '[',
        'ъ' | 'ї' => ']',
        'ф' => 'a',
        'ы' | 'і' => 's',
        'в' => 'd',
        'а' => 'f',
        'п' => 'g',
        'р' => 'h',
        'о' => 'j',
        'л' => 'k',
        'д' => 'l',
        'ж' => ';',
        'э' | 'є' => '\'',
        'я' => 'z',
        'ч' => 'x',
        'с' => 'c',
        'м' => 'v',
        'и' => 'b',
        'т' => 'n',
        'ь' => 'm',
        'б' => ',',
        'ю' => '.',
        'ё' => '`',
        'Й' => 'Q',
        'Ц' => 'W',
        'У' => 'E',
        'К' => 'R',
        'Е' => 'T',
        'Н' => 'Y',
        'Г' => 'U',
        'Ш' => 'I',
        'Щ' | 'Ў' => 'O',
        'З' => 'P',
        'Х' => '{',
        'Ъ' | 'Ї' => '}',
        'Ф' => 'A',
        'Ы' | 'І' => 'S',
        'В' => 'D',
        'А' => 'F',
        'П' => 'G',
        'Р' => 'H',
        'О' => 'J',
        'Л' => 'K',
        'Д' => 'L',
        'Ж' => ':',
        'Э' | 'Є' => '"',
        'Я' => 'Z',
        'Ч' => 'X',
        'С' => 'C',
        'М' => 'V',
        'И' => 'B',
        'Т' => 'N',
        'Ь' => 'M',
        'Б' => '<',
        'Ю' => '>',
        'Ё' => '~',
        // Greek.
        'ς' => 'w',
        'ε' => 'e',
        'ρ' => 'r',
        'τ' => 't',
        'υ' => 'y',
        'θ' => 'u',
        'ι' => 'i',
        'ο' => 'o',
        'π' => 'p',
        'α' => 'a',
        'σ' => 's',
        'δ' => 'd',
        'φ' => 'f',
        'γ' => 'g',
        'η' => 'h',
        'ξ' => 'j',
        'κ' => 'k',
        'λ' => 'l',
        'ζ' => 'z',
        'χ' => 'x',
        'ψ' => 'c',
        'ω' => 'v',
        'β' => 'b',
        'ν' => 'n',
        'μ' => 'm',
        'Ε' => 'E',
        'Ρ' => 'R',
        'Τ' => 'T',
        'Υ' => 'Y',
        'Θ' => 'U',
        'Ι' => 'I',
        'Ο' => 'O',
        'Π' => 'P',
        'Α' => 'A',
        'Σ' => 'S',
        'Δ' => 'D',
        'Φ' => 'F',
        'Γ' => 'G',
        'Η' => 'H',
        'Ξ' => 'J',
        'Κ' => 'K',
        'Λ' => 'L',
        'Ζ' => 'Z',
        'Χ' => 'X',
        'Ψ' => 'C',
        'Ω' => 'V',
        'Β' => 'B',
        'Ν' => 'N',
        'Μ' => 'M',
        // Hebrew (SI-1452). Hebrew has no case, so these are the unshifted keys.
        'ק' => 'e',
        'ר' => 'r',
        'א' => 't',
        'ט' => 'y',
        'ו' => 'u',
        'ן' => 'i',
        'ם' => 'o',
        'פ' => 'p',
        'ש' => 'a',
        'ד' => 's',
        'ג' => 'd',
        'כ' => 'f',
        'ע' => 'g',
        'י' => 'h',
        'ח' => 'j',
        'ל' => 'k',
        'ך' => 'l',
        'ף' => ';',
        'ז' => 'z',
        'ס' => 'x',
        'ב' => 'c',
        'ה' => 'v',
        'נ' => 'b',
        'מ' => 'n',
        'צ' => 'm',
        'ת' => ',',
        'ץ' => '.',
        _ => return None,
    };
    Some(latin)
}

/// A logical key value, independent of the separately reported modifier state.
///
/// Character codes describe the text produced by the active keyboard layout,
/// not a physical key position. This keeps text input and character bindings
/// portable across layouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    /// The logical Unicode character produced by the key press.
    Char(char),
    /// Enter/Return.
    Enter,
    /// Escape.
    Esc,
    /// Backspace (delete before cursor).
    Backspace,
    /// Delete (delete under/after cursor).
    Delete,
    /// Tab.
    Tab,
    /// Back-tab (Shift-Tab).
    BackTab,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Home.
    Home,
    /// End.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
}

/// A mouse event at a terminal cell, with modifier state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mouse {
    /// What the pointer did (button press/release/drag, move, or scroll).
    pub kind: MouseKind,
    /// Cell column (0-based, terminal coordinates).
    pub column: u16,
    /// Cell row (0-based, terminal coordinates).
    pub row: u16,
    /// Shift held during the event.
    pub shift: bool,
    /// Ctrl held during the event.
    pub ctrl: bool,
    /// Alt held during the event.
    pub alt: bool,
    /// Super/Command held during the event.
    pub super_key: bool,
}

impl Mouse {
    /// A bare event at `(column, row)` with the given kind and no modifiers —
    /// convenience for tests and synthetic events.
    pub fn at(kind: MouseKind, column: u16, row: u16) -> Self {
        Self {
            kind,
            column,
            row,
            shift: false,
            ctrl: false,
            alt: false,
            super_key: false,
        }
    }

    /// True when no modifier keys are held. Terminals also use Shift-drag to
    /// bypass application mouse capture for native selection, so a host that
    /// implements its own selection should generally act only on `plain()`
    /// left-drags and leave Shift-drags to the terminal.
    pub fn plain(&self) -> bool {
        !self.shift && !self.ctrl && !self.alt && !self.super_key
    }
}

/// Which mouse button an event refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    /// Left (primary) button.
    Left,
    /// Right (secondary) button.
    Right,
    /// Middle button.
    Middle,
}

/// The kind of mouse interaction an event represents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    /// Button pressed.
    Down(MouseButton),
    /// Button released.
    Up(MouseButton),
    /// Pointer moved with a button held (a drag of that button).
    Drag(MouseButton),
    /// Pointer moved with no button held.
    Moved,
    /// Wheel scrolled up.
    ScrollUp,
    /// Wheel scrolled down.
    ScrollDown,
    /// Wheel scrolled left.
    ScrollLeft,
    /// Wheel scrolled right.
    ScrollRight,
}

/// Whether a component consumed an event or let it bubble.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventFlow {
    /// The event was handled; stop propagating.
    Consumed,
    /// The event was not handled; keep bubbling.
    Ignored,
}

/// Result of routing an input event through host-owned component state.
///
/// The outcome describes both propagation and lifecycle without duplicating
/// values already available from the state (`selected()`, `value()`, `text()`,
/// and so on). Every variant except [`Ignored`](Self::Ignored) consumes the
/// event.
#[must_use = "input outcomes determine whether an event keeps propagating"]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputOutcome {
    /// The state did not recognize the event; keep routing it.
    #[default]
    Ignored,
    /// The event was recognized, but persistent state was already at its bound.
    Consumed,
    /// Persistent component state changed.
    Changed,
    /// The current value was activated or requested for submission.
    Submitted,
    /// The interaction requested cancellation or dismissal.
    Cancelled,
}

impl InputOutcome {
    /// Convert this richer result to the propagation-only [`EventFlow`].
    pub fn flow(self) -> EventFlow {
        if self == Self::Ignored {
            EventFlow::Ignored
        } else {
            EventFlow::Consumed
        }
    }

    /// Whether the event was recognized and should stop propagating.
    pub fn consumed(self) -> bool {
        self.flow().consumed()
    }

    /// Whether persistent component state changed.
    pub fn changed(self) -> bool {
        self == Self::Changed
    }
}

impl EventFlow {
    /// True for [`EventFlow::Consumed`].
    pub fn consumed(self) -> bool {
        matches!(self, EventFlow::Consumed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(c: char, ctrl: bool, alt: bool) -> Key {
        Key {
            code: KeyCode::Char(c),
            ctrl,
            alt,
            shift: false,
        }
    }

    #[test]
    fn base_layout_maps_non_latin_letters_to_their_qwerty_key() {
        for (layout, latin) in [
            ('с', 'c'), // Cyrillic es, Ctrl+C's key
            ('і', 's'), // Ukrainian
            ('ы', 's'), // Russian, same key
            ('ў', 'o'), // Belarusian
            ('ї', ']'),
            ('С', 'C'),
            ('ψ', 'c'), // Greek
            ('Σ', 'S'),
            ('ב', 'c'), // Hebrew
        ] {
            let key = chord(layout, true, false).base_layout();
            assert_eq!(key, Some(chord(latin, true, false)), "{layout}");
        }
        for unmapped in ['c', 'é', 'ґ', '中', '1'] {
            assert_eq!(
                chord(unmapped, true, false).base_layout(),
                None,
                "{unmapped}"
            );
        }
        assert_eq!(Key::new(KeyCode::Enter).base_layout(), None);
    }

    #[test]
    fn shortcut_translates_only_modified_chords() {
        assert_eq!(chord('с', true, false).shortcut(), chord('c', true, false));
        assert_eq!(chord('с', false, true).shortcut(), chord('c', false, true));
        // Plain text stays text.
        assert_eq!(
            chord('с', false, false).shortcut(),
            chord('с', false, false)
        );
        assert_eq!(chord('x', true, false).shortcut(), chord('x', true, false));
    }

    #[test]
    fn every_base_layout_target_is_ascii() {
        for c in ('\u{370}'..='\u{3ff}')
            .chain('\u{400}'..='\u{4ff}')
            .chain('\u{590}'..='\u{5ff}')
        {
            if let Some(latin) = qwerty_equivalent(c) {
                assert!(latin.is_ascii_graphic(), "{c} -> {latin:?}");
            }
        }
    }
}
