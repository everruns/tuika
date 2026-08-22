//! Cell attributes: [`Color`], [`Modifier`], and [`Style`].
//!
//! These are tuika's own types rather than re-exports, so `ratatui-core` does
//! not appear in tuika's public API. The variants and the patch semantics match
//! ratatui's deliberately: the two are converted at the interop boundary
//! ([`Surface::render_ratatui`](crate::Surface::render_ratatui)) under the
//! `ratatui` feature, and a mismatch there would be a silent rendering
//! difference rather than a compile error.

use core::fmt;

/// A terminal color.
///
/// `Reset` returns the cell to the terminal's own default rather than painting
/// a color, which is what makes a scrollback block published under
/// [`ScreenMode::SplitFooter`](crate::ScreenMode) read as part of the
/// surrounding session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Color {
    /// Reset to the terminal's default foreground or background.
    #[default]
    Reset,
    /// ANSI black (fg 30 / bg 40).
    Black,
    /// ANSI red (fg 31 / bg 41).
    Red,
    /// ANSI green (fg 32 / bg 42).
    Green,
    /// ANSI yellow (fg 33 / bg 43).
    Yellow,
    /// ANSI blue (fg 34 / bg 44).
    Blue,
    /// ANSI magenta (fg 35 / bg 45).
    Magenta,
    /// ANSI cyan (fg 36 / bg 46).
    Cyan,
    /// ANSI white (fg 37 / bg 47) — the dim one; [`Color::White`] is the bright.
    Gray,
    /// ANSI bright black (fg 90 / bg 100).
    DarkGray,
    /// ANSI bright red (fg 91 / bg 101).
    LightRed,
    /// ANSI bright green (fg 92 / bg 102).
    LightGreen,
    /// ANSI bright yellow (fg 93 / bg 103).
    LightYellow,
    /// ANSI bright blue (fg 94 / bg 104).
    LightBlue,
    /// ANSI bright magenta (fg 95 / bg 105).
    LightMagenta,
    /// ANSI bright cyan (fg 96 / bg 106).
    LightCyan,
    /// ANSI bright white (fg 97 / bg 107).
    White,
    /// A 24-bit truecolor value. Terminals without truecolor support render
    /// this unpredictably; [`term::capabilities`](crate::term::capabilities)
    /// is how a host detects that before choosing a theme.
    Rgb(u8, u8, u8),
    /// An 8-bit indexed color from the 256-color palette.
    Indexed(u8),
}

/// Text attributes, composable as bitflags.
///
/// A [`Style`] carries two of these — one to add and one to remove — so a style
/// patched over another can turn an attribute *off* as well as on.
///
/// Hand-rolled rather than pulled from `bitflags`: nine flags and the four set
/// operations tuika uses are less code than justifying the dependency, and the
/// `NAMED` table below is what gives [`Debug`] its readable output.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Modifier(u16);

impl Modifier {
    /// Bold.
    pub const BOLD: Self = Self(0b0000_0000_0001);
    /// Dim / faint.
    pub const DIM: Self = Self(0b0000_0000_0010);
    /// Italic.
    pub const ITALIC: Self = Self(0b0000_0000_0100);
    /// Underlined.
    pub const UNDERLINED: Self = Self(0b0000_0000_1000);
    /// Slow blink.
    pub const SLOW_BLINK: Self = Self(0b0000_0001_0000);
    /// Rapid blink.
    pub const RAPID_BLINK: Self = Self(0b0000_0010_0000);
    /// Swap foreground and background.
    pub const REVERSED: Self = Self(0b0000_0100_0000);
    /// Hidden / concealed.
    pub const HIDDEN: Self = Self(0b0000_1000_0000);
    /// Struck through.
    pub const CROSSED_OUT: Self = Self(0b0001_0000_0000);

    /// Every flag paired with its name, for [`Debug`] and for `all()`.
    const NAMED: [(&'static str, Self); 9] = [
        ("BOLD", Self::BOLD),
        ("DIM", Self::DIM),
        ("ITALIC", Self::ITALIC),
        ("UNDERLINED", Self::UNDERLINED),
        ("SLOW_BLINK", Self::SLOW_BLINK),
        ("RAPID_BLINK", Self::RAPID_BLINK),
        ("REVERSED", Self::REVERSED),
        ("HIDDEN", Self::HIDDEN),
        ("CROSSED_OUT", Self::CROSSED_OUT),
    ];

    /// No attributes.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Every attribute.
    pub const fn all() -> Self {
        Self(0b0001_1111_1111)
    }

    /// Whether no attribute is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether every flag in `other` is set here.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The flags set in either.
    #[must_use = "returns a new Modifier"]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The flags set in both.
    #[must_use = "returns a new Modifier"]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// The flags set here but not in `other`.
    #[must_use = "returns a new Modifier"]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// The raw bits, for a backend translating to terminal attributes.
    pub const fn bits(self) -> u16 {
        self.0
    }
}

impl core::ops::BitOr for Modifier {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl core::ops::BitOrAssign for Modifier {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

impl core::ops::BitAnd for Modifier {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        self.intersection(rhs)
    }
}

impl core::ops::Sub for Modifier {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        self.difference(rhs)
    }
}

/// The appearance of a run of cells.
///
/// Every field is optional, which is what makes styles *composable*: a
/// component resolves a [`Role`](crate::style::Role) from the theme and then
/// patches a caller's override on top, and only the fields the override sets
/// are replaced. [`patch`](Self::patch) is that operation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Style {
    /// Foreground color, or `None` to inherit.
    pub fg: Option<Color>,
    /// Background color, or `None` to inherit.
    pub bg: Option<Color>,
    /// Underline color, or `None` to inherit. Honored only by terminals that
    /// implement the extended underline SGR.
    pub underline_color: Option<Color>,
    /// Attributes this style turns on.
    pub add_modifier: Modifier,
    /// Attributes this style turns off. Applied after `add_modifier`, so
    /// removing an attribute wins over adding it in the same style.
    pub sub_modifier: Modifier,
}

impl Style {
    /// A style that changes nothing.
    pub const fn new() -> Self {
        Self {
            fg: None,
            bg: None,
            underline_color: None,
            add_modifier: Modifier::empty(),
            sub_modifier: Modifier::empty(),
        }
    }

    /// A style that resets every attribute to the terminal default.
    ///
    /// Distinct from [`new`](Self::new): `new` inherits what is already there,
    /// `reset` actively clears it.
    pub const fn reset() -> Self {
        Self {
            fg: Some(Color::Reset),
            bg: Some(Color::Reset),
            underline_color: Some(Color::Reset),
            add_modifier: Modifier::empty(),
            sub_modifier: Modifier::all(),
        }
    }

    /// Set the foreground color.
    #[must_use = "returns a new Style"]
    pub const fn fg(mut self, color: Color) -> Self {
        self.fg = Some(color);
        self
    }

    /// Set the background color.
    #[must_use = "returns a new Style"]
    pub const fn bg(mut self, color: Color) -> Self {
        self.bg = Some(color);
        self
    }

    /// Set the underline color.
    #[must_use = "returns a new Style"]
    pub const fn underline_color(mut self, color: Color) -> Self {
        self.underline_color = Some(color);
        self
    }

    /// Turn `modifier` on, and stop turning it off.
    #[must_use = "returns a new Style"]
    pub const fn add_modifier(mut self, modifier: Modifier) -> Self {
        self.sub_modifier = self.sub_modifier.difference(modifier);
        self.add_modifier = self.add_modifier.union(modifier);
        self
    }

    /// Turn `modifier` off, and stop turning it on.
    #[must_use = "returns a new Style"]
    pub const fn remove_modifier(mut self, modifier: Modifier) -> Self {
        self.add_modifier = self.add_modifier.difference(modifier);
        self.sub_modifier = self.sub_modifier.union(modifier);
        self
    }

    /// Whether this style turns `modifier` on.
    pub const fn has_modifier(self, modifier: Modifier) -> bool {
        self.add_modifier.contains(modifier)
    }

    /// Bold. Shorthand for `add_modifier(Modifier::BOLD)`.
    #[must_use = "returns a new Style"]
    pub const fn bold(self) -> Self {
        self.add_modifier(Modifier::BOLD)
    }

    /// Dim. Shorthand for `add_modifier(Modifier::DIM)`.
    #[must_use = "returns a new Style"]
    pub const fn dim(self) -> Self {
        self.add_modifier(Modifier::DIM)
    }

    /// Italic. Shorthand for `add_modifier(Modifier::ITALIC)`.
    #[must_use = "returns a new Style"]
    pub const fn italic(self) -> Self {
        self.add_modifier(Modifier::ITALIC)
    }

    /// Underlined. Shorthand for `add_modifier(Modifier::UNDERLINED)`.
    #[must_use = "returns a new Style"]
    pub const fn underlined(self) -> Self {
        self.add_modifier(Modifier::UNDERLINED)
    }

    /// Reversed. Shorthand for `add_modifier(Modifier::REVERSED)`.
    #[must_use = "returns a new Style"]
    pub const fn reversed(self) -> Self {
        self.add_modifier(Modifier::REVERSED)
    }

    /// Struck through. Shorthand for `add_modifier(Modifier::CROSSED_OUT)`.
    #[must_use = "returns a new Style"]
    pub const fn crossed_out(self) -> Self {
        self.add_modifier(Modifier::CROSSED_OUT)
    }

    /// Layer `other` on top of `self`.
    ///
    /// Fields `other` leaves unset keep `self`'s value; modifiers accumulate,
    /// with `other`'s removals applied last. This is the operation that lets a
    /// theme role and a per-call override compose without either needing to
    /// know about the other.
    #[must_use = "returns a new Style"]
    pub fn patch<S: Into<Self>>(mut self, other: S) -> Self {
        let other = other.into();
        self.fg = other.fg.or(self.fg);
        self.bg = other.bg.or(self.bg);
        self.underline_color = other.underline_color.or(self.underline_color);
        self.add_modifier = self.add_modifier.difference(other.sub_modifier);
        self.add_modifier = self.add_modifier.union(other.add_modifier);
        self.sub_modifier = self.sub_modifier.difference(other.add_modifier);
        self.sub_modifier = self.sub_modifier.union(other.sub_modifier);
        self
    }
}

impl From<Color> for Style {
    /// A color used where a `Style` is expected sets the foreground.
    fn from(color: Color) -> Self {
        Self::new().fg(color)
    }
}

impl From<Modifier> for Style {
    fn from(modifier: Modifier) -> Self {
        Self::new().add_modifier(modifier)
    }
}

impl fmt::Debug for Modifier {
    /// Print `NONE` for the empty set rather than `Modifier(0x0)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("NONE");
        }
        let mut first = true;
        for (name, bit) in Self::NAMED {
            if self.contains(bit) {
                if !first {
                    f.write_str(" | ")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The composition law components rely on: unset fields inherit, set fields
    /// win. Asserted field by field rather than on a whole `Style` so a
    /// regression names the field that broke.
    #[test]
    fn patch_keeps_unset_fields_and_replaces_set_ones() {
        let base = Style::new().fg(Color::Red).bg(Color::Blue);
        let over = Style::new().fg(Color::Green);
        let merged = base.patch(over);
        assert_eq!(merged.fg, Some(Color::Green));
        assert_eq!(merged.bg, Some(Color::Blue));
    }

    #[test]
    fn patch_accumulates_added_modifiers() {
        let merged = Style::new()
            .add_modifier(Modifier::BOLD)
            .patch(Style::new().add_modifier(Modifier::ITALIC));
        assert!(merged.add_modifier.contains(Modifier::BOLD));
        assert!(merged.add_modifier.contains(Modifier::ITALIC));
    }

    /// Removal has to beat an earlier add, or a theme role could never turn off
    /// an attribute a wrapper switched on.
    #[test]
    fn a_later_removal_overrides_an_earlier_add() {
        let merged = Style::new()
            .add_modifier(Modifier::BOLD)
            .patch(Style::new().remove_modifier(Modifier::BOLD));
        assert!(!merged.add_modifier.contains(Modifier::BOLD));
        assert!(merged.sub_modifier.contains(Modifier::BOLD));
    }

    /// Setting a modifier both ways on one style is a caller error; the last
    /// call is what holds, in both directions.
    #[test]
    fn add_and_remove_are_mutually_exclusive_on_one_style() {
        let removed = Style::new()
            .add_modifier(Modifier::DIM)
            .remove_modifier(Modifier::DIM);
        assert!(!removed.add_modifier.contains(Modifier::DIM));
        assert!(removed.sub_modifier.contains(Modifier::DIM));

        let added = Style::new()
            .remove_modifier(Modifier::DIM)
            .add_modifier(Modifier::DIM);
        assert!(added.add_modifier.contains(Modifier::DIM));
        assert!(!added.sub_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn reset_clears_every_attribute() {
        let reset = Style::reset();
        assert_eq!(reset.fg, Some(Color::Reset));
        assert_eq!(reset.bg, Some(Color::Reset));
        assert_eq!(reset.sub_modifier, Modifier::all());
    }

    #[test]
    fn empty_modifier_debugs_as_none() {
        assert_eq!(format!("{:?}", Modifier::empty()), "NONE");
        assert_eq!(format!("{:?}", Modifier::BOLD), "BOLD");
        assert_eq!(
            format!("{:?}", Modifier::BOLD | Modifier::ITALIC),
            "BOLD | ITALIC"
        );
    }

    #[test]
    fn a_color_converts_to_a_foreground_style() {
        assert_eq!(Style::from(Color::Red).fg, Some(Color::Red));
    }
}
