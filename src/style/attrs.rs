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
    /// A 24-bit truecolor value. A terminal without truecolor support renders
    /// this unpredictably, so the backends map it to the nearest color the
    /// terminal can show — see [`Color::downsample`] and
    /// [`ColorDepth`](crate::term::capabilities::ColorDepth).
    Rgb(u8, u8, u8),
    /// An 8-bit indexed color from the 256-color palette.
    Indexed(u8),
}

/// The xterm defaults for the sixteen ANSI slots, in slot order. A terminal's
/// theme may repaint these, which is why downsampling only *targets* them when
/// it has to: they are the one palette every color terminal has.
const ANSI16_RGB: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

/// The named variants in ANSI slot order, so slot `i` is `ANSI16[i]`.
const ANSI16: [Color; 16] = [
    Color::Black,
    Color::Red,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::Gray,
    Color::DarkGray,
    Color::LightRed,
    Color::LightGreen,
    Color::LightYellow,
    Color::LightBlue,
    Color::LightMagenta,
    Color::LightCyan,
    Color::White,
];

/// The six channel levels of the xterm 6×6×6 cube (slots 16–231).
const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

impl Color {
    /// The nearest color a terminal of `depth` can show.
    ///
    /// [`ColorDepth::TrueColor`] returns `self` unchanged. Below that, an
    /// [`Rgb`](Color::Rgb) becomes the nearest slot of the xterm 256-color
    /// palette (its 6×6×6 cube or 24-step gray ramp — never slots 0–15, whose
    /// values the user's theme decides), and at [`ColorDepth::Ansi16`] both
    /// `Rgb` and [`Indexed`](Color::Indexed) become the nearest of the sixteen
    /// named colors. `Reset` and the named colors are already representable
    /// everywhere and pass through.
    ///
    /// The backends call this on every cell they emit, so a host normally never
    /// does; it is public for a host that writes styled text some other way.
    ///
    /// ```
    /// use tuika::style::Color;
    /// use tuika::term::capabilities::ColorDepth;
    ///
    /// assert_eq!(Color::Rgb(255, 0, 0).downsample(ColorDepth::Ansi256), Color::Indexed(196));
    /// assert_eq!(Color::Rgb(250, 10, 10).downsample(ColorDepth::Ansi16), Color::LightRed);
    /// assert_eq!(Color::Rgb(1, 2, 3).downsample(ColorDepth::TrueColor), Color::Rgb(1, 2, 3));
    /// ```
    ///
    /// [`ColorDepth::TrueColor`]: crate::term::capabilities::ColorDepth::TrueColor
    /// [`ColorDepth::Ansi16`]: crate::term::capabilities::ColorDepth::Ansi16
    #[must_use = "returns the downsampled color"]
    pub fn downsample(self, depth: crate::term::capabilities::ColorDepth) -> Self {
        use crate::term::capabilities::ColorDepth;
        match (depth, self) {
            (ColorDepth::TrueColor, _) => self,
            (ColorDepth::Ansi256, Self::Rgb(r, g, b)) => Self::Indexed(nearest_256(r, g, b)),
            (ColorDepth::Ansi16, Self::Rgb(r, g, b)) => ANSI16[nearest_16(r, g, b)],
            (ColorDepth::Ansi16, Self::Indexed(i)) if i < 16 => ANSI16[usize::from(i)],
            (ColorDepth::Ansi16, Self::Indexed(i)) => {
                let (r, g, b) = indexed_rgb(i);
                ANSI16[nearest_16(r, g, b)]
            }
            _ => self,
        }
    }
}

/// The RGB value xterm assigns to palette slot `i`.
fn indexed_rgb(i: u8) -> (u8, u8, u8) {
    match i {
        0..=15 => ANSI16_RGB[usize::from(i)],
        16..=231 => {
            let i = i - 16;
            (
                CUBE_LEVELS[usize::from(i / 36)],
                CUBE_LEVELS[usize::from(i / 6 % 6)],
                CUBE_LEVELS[usize::from(i % 6)],
            )
        }
        232..=255 => {
            let v = 8 + 10 * (i - 232);
            (v, v, v)
        }
    }
}

/// Perceptual-ish squared distance ("redmean"): plain Euclidean RGB distance
/// over-weights blue and under-weights green, which is what makes a naive
/// nearest-match turn mid greens gray. Cheap, dependency-free, and good enough
/// for picking among at most 256 fixed candidates.
fn distance((r1, g1, b1): (u8, u8, u8), (r2, g2, b2): (u8, u8, u8)) -> u32 {
    let rmean = (u32::from(r1) + u32::from(r2)) / 2;
    let dr = i32::from(r1) - i32::from(r2);
    let dg = i32::from(g1) - i32::from(g2);
    let db = i32::from(b1) - i32::from(b2);
    let (dr2, dg2, db2) = (
        dr.unsigned_abs().pow(2),
        dg.unsigned_abs().pow(2),
        db.unsigned_abs().pow(2),
    );
    (((512 + rmean) * dr2) >> 8) + 4 * dg2 + (((767 - rmean) * db2) >> 8)
}

/// The nearest slot in 16–255: the closest cube color or the closest gray,
/// whichever is nearer. Two candidates rather than a scan of all 240, the way
/// tmux does it, because each axis of the cube can be quantised on its own.
fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    // A channel's nearest cube level: the midpoints between 0, 95, 135, … are
    // 47.5, 115, 155, 195, 235.
    let level = |v: u8| -> u8 {
        match v {
            0..48 => 0,
            48..115 => 1,
            _ => (v - 35) / 40,
        }
    };
    let (qr, qg, qb) = (level(r), level(g), level(b));
    let cube_slot = 16 + 36 * qr + 6 * qg + qb;
    let cube = indexed_rgb(cube_slot);
    if cube == (r, g, b) {
        return cube_slot;
    }
    let avg = (u16::from(r) + u16::from(g) + u16::from(b)) / 3;
    // Grays are 8, 18, …, 238; anything past 238 is nearest the last step
    // (white itself is a cube corner, and wins there on distance).
    let step = if avg > 238 {
        23
    } else {
        (avg.saturating_sub(3) / 10) as u8
    };
    let gray_slot = 232 + step;
    let gray = indexed_rgb(gray_slot);
    if distance(gray, (r, g, b)) < distance(cube, (r, g, b)) {
        gray_slot
    } else {
        cube_slot
    }
}

/// The index (0–15) of the ANSI color that best stands in for `(r, g, b)`.
///
/// Not a nearest-RGB match. Against only sixteen fixed, fully saturated
/// candidates, distance sends every muted color — a copper, a sage, a plum, the
/// palettes real themes are made of — to gray, and a 16-color frame of a warm
/// theme comes out monochrome. So hue and lightness are judged separately:
///
/// - A color with little chroma, or too dark to read as a hue, is a gray, and
///   its lightness picks black, bright black, white, or bright white.
/// - Otherwise each channel that sits in the upper half of the color's own
///   range is "on", which picks one of the six hues, and a light color (or a
///   near-full channel) takes the bright variant.
fn nearest_16(r: u8, g: u8, b: u8) -> usize {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let chroma = max - min;
    let lightness = (u16::from(max) + u16::from(min)) / 2;
    if chroma < 40 || max < 64 {
        // Midpoints between the gray slots' values: 0, 127, 229, 255.
        return match lightness {
            0..64 => 0,
            64..178 => 8,
            178..242 => 7,
            _ => 15,
        };
    }
    let on = |v: u8| usize::from(v - min >= chroma.div_ceil(2));
    let hue = on(r) | on(g) << 1 | on(b) << 2;
    let bright = max >= 230 || lightness >= 150;
    if bright { hue + 8 } else { hue }
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
    use crate::term::capabilities::ColorDepth;

    #[test]
    fn truecolor_depth_is_the_identity() {
        for color in [
            Color::Reset,
            Color::Red,
            Color::Rgb(1, 2, 3),
            Color::Indexed(200),
        ] {
            assert_eq!(color.downsample(ColorDepth::TrueColor), color);
        }
    }

    #[test]
    fn named_and_reset_pass_through_every_depth() {
        for depth in [ColorDepth::Ansi16, ColorDepth::Ansi256] {
            for color in ANSI16.iter().copied().chain([Color::Reset]) {
                assert_eq!(color.downsample(depth), color, "{color:?} at {depth:?}");
            }
        }
    }

    #[test]
    fn rgb_to_256_lands_exactly_on_palette_colors() {
        // Every cube and gray slot round-trips through its own RGB value.
        for slot in 16..=255u8 {
            let (r, g, b) = indexed_rgb(slot);
            let got = Color::Rgb(r, g, b).downsample(ColorDepth::Ansi256);
            assert_eq!(got, Color::Indexed(slot), "slot {slot} = ({r},{g},{b})");
        }
    }

    #[test]
    fn rgb_to_256_picks_the_nearer_of_cube_and_gray() {
        let down = |r, g, b| Color::Rgb(r, g, b).downsample(ColorDepth::Ansi256);
        // A near-gray goes to the finer gray ramp, not the coarse cube.
        assert_eq!(down(100, 100, 102), Color::Indexed(241)); // gray 98
        // A saturated color stays in the cube.
        assert_eq!(down(250, 5, 5), Color::Indexed(196));
        // Solarized base03 lands on 234, the slot Solarized's own 256-color
        // fallback picks for it.
        assert_eq!(down(0x00, 0x2b, 0x36), Color::Indexed(234));
        // Never one of the theme-owned slots 0–15.
        for (r, g, b) in [(0, 0, 0), (255, 255, 255), (205, 0, 0), (127, 127, 127)] {
            let Color::Indexed(slot) = down(r, g, b) else {
                panic!("not indexed")
            };
            assert!(slot >= 16, "({r},{g},{b}) -> {slot}");
        }
    }

    #[test]
    fn rgb_and_indexed_to_16_choose_the_nearest_named_color() {
        let down = |c: Color| c.downsample(ColorDepth::Ansi16);
        assert_eq!(down(Color::Rgb(250, 10, 10)), Color::LightRed);
        assert_eq!(down(Color::Rgb(180, 0, 0)), Color::Red);
        assert_eq!(down(Color::Rgb(0, 0x2b, 0x36)), Color::Black);
        assert_eq!(down(Color::Rgb(250, 250, 250)), Color::White);
        assert_eq!(down(Color::Rgb(235, 235, 235)), Color::Gray);
        assert_eq!(down(Color::Rgb(120, 120, 120)), Color::DarkGray);
        assert_eq!(down(Color::Rgb(30, 200, 40)), Color::Green);
        // Muted theme colors keep their hue instead of collapsing to gray…
        assert_eq!(down(Color::Rgb(175, 91, 59)), Color::Red); // copper
        assert_eq!(down(Color::Rgb(189, 143, 179)), Color::LightMagenta); // plum
        assert_eq!(down(Color::Rgb(128, 155, 177)), Color::LightCyan); // steel blue
        // …while near-neutrals stay gray and dark backgrounds stay black.
        assert_eq!(down(Color::Rgb(222, 204, 199)), Color::Gray);
        assert_eq!(down(Color::Rgb(151, 119, 115)), Color::DarkGray);
        assert_eq!(down(Color::Rgb(22, 18, 21)), Color::Black);
        // Slots 0–15 map to their named color; the rest via their RGB value.
        for (slot, named) in ANSI16.iter().enumerate() {
            assert_eq!(down(Color::Indexed(slot as u8)), *named);
        }
        assert_eq!(down(Color::Indexed(196)), Color::LightRed);
        assert_eq!(down(Color::Indexed(232)), Color::Black);
        assert_eq!(down(Color::Indexed(255)), Color::Gray);
        // Every result is a named color.
        for slot in 0..=255u8 {
            assert!(ANSI16.contains(&down(Color::Indexed(slot))), "{slot}");
        }
    }

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
