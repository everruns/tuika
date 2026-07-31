//! Styling and theming.
//!
//! `tuika` reuses ratatui's [`Style`], [`Color`], and [`Modifier`] rather than
//! reinventing cell attributes, but components never hard-code colors: they
//! pull from a [`Theme`] passed through the render context. Swapping the theme
//! restyles every component at once, and downstream libraries can ship their
//! own palette without touching component code.

use ratatui_core::style::{Color, Modifier, Style};

use crate::geometry::Padding;

/// Line-drawing style for bordered components.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BorderStyle {
    /// Rounded corners (`╭╮╰╯`) with light edges.
    #[default]
    Rounded,
    /// Square corners (`┌┐└┘`) with light edges.
    Plain,
    /// Heavy corners and edges (`┏┓┗┛━┃`).
    Thick,
    /// No border; drawn as blank spaces.
    None,
}

impl BorderStyle {
    /// The six line-drawing glyphs (corners, horizontal, vertical) for this style.
    pub fn glyphs(self) -> BorderGlyphs {
        match self {
            BorderStyle::Rounded => BorderGlyphs::new('╭', '╮', '╰', '╯', '─', '│'),
            BorderStyle::Plain => BorderGlyphs::new('┌', '┐', '└', '┘', '─', '│'),
            BorderStyle::Thick => BorderGlyphs::new('┏', '┓', '┗', '┛', '━', '┃'),
            BorderStyle::None => BorderGlyphs::new(' ', ' ', ' ', ' ', ' ', ' '),
        }
    }
}

/// The resolved corner and edge glyphs for a [`BorderStyle`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BorderGlyphs {
    /// Top-left corner glyph.
    pub top_left: char,
    /// Top-right corner glyph.
    pub top_right: char,
    /// Bottom-left corner glyph.
    pub bottom_left: char,
    /// Bottom-right corner glyph.
    pub bottom_right: char,
    /// Horizontal edge glyph (top and bottom sides).
    pub horizontal: char,
    /// Vertical edge glyph (left and right sides).
    pub vertical: char,
}

impl BorderGlyphs {
    const fn new(
        top_left: char,
        top_right: char,
        bottom_left: char,
        bottom_right: char,
        horizontal: char,
        vertical: char,
    ) -> Self {
        Self {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
            horizontal,
            vertical,
        }
    }
}

/// A named palette every component styles itself from.
///
/// The default mirrors yolop's existing inline palette so the full-screen
/// renderer looks like the same product. A different host can construct its
/// own `Theme` and the whole component tree follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// Base fill behind the whole screen.
    pub background: Color,
    /// Raised fill for panels and overlays that sit above the background.
    pub surface: Color,
    /// Primary foreground for body text.
    pub text: Color,
    /// De-emphasized foreground for secondary text, hints, and scrollbar thumbs.
    pub muted: Color,
    /// Faintest foreground, dimmer than `muted`, for inactive tracks and rules.
    pub dim: Color,
    /// Primary highlight for active/emphasized elements (progress fill, spinner).
    pub accent: Color,
    /// Secondary accent for complementary highlights distinct from `accent`.
    pub accent_alt: Color,
    /// Border color for unfocused bordered components.
    pub border: Color,
    /// Border color for the focused component.
    pub border_focused: Color,
    /// Background of the selected item in lists and menus.
    pub selection_bg: Color,
    /// Foreground of the selected item in lists and menus.
    pub selection_fg: Color,
    /// Markdown and code-block role colors, consumed by [`Markdown`] and
    /// [`CodeBlock`] (and any [`Highlighter`] the host plugs in).
    ///
    /// [`Markdown`]: crate::components::Markdown
    /// [`CodeBlock`]: crate::components::CodeBlock
    /// [`Highlighter`]: crate::highlight::Highlighter
    pub code: CodeTheme,
}

/// The palette [`Markdown`](crate::components::Markdown) and
/// [`CodeBlock`](crate::components::CodeBlock) style themselves from.
///
/// It carries two related things: a few markdown-prose roles (`heading`,
/// `link`, the inline/`block` code background) and the syntax roles a
/// [`Highlighter`](crate::highlight::Highlighter) maps token classes onto so
/// highlighted code follows the host theme instead of a hard-coded palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeTheme {
    /// Heading text (`# …`).
    pub heading: Color,
    /// Link text and bare URLs.
    pub link: Color,
    /// Background behind inline code and fenced blocks.
    pub background: Color,
    /// Default code text: unclassified tokens and the plain fallback.
    pub text: Color,
    /// Language-tag label above a fenced block.
    pub label: Color,
    // Syntax roles a highlighter maps token classes onto.
    /// Keywords and language reserved words.
    pub keyword: Color,
    /// Function and method names.
    pub function: Color,
    /// Type names, traits, and other type-level identifiers.
    pub type_name: Color,
    /// Constants, literals, and enum-like values.
    pub constant: Color,
    /// String and character literals.
    pub string: Color,
    /// Comments.
    pub comment: Color,
    /// Punctuation, operators, and delimiters.
    pub punctuation: Color,
}

impl Default for Theme {
    fn default() -> Self {
        // tuika's own toolkit identity — a warm red-on-dark palette. This is a
        // neutral default for any app built on tuika; it is deliberately NOT any
        // one host's brand. A host with its own look builds its own `Theme` (see
        // yolop's `fullscreen::yolop_theme`) rather than relying on this.
        Theme {
            background: Color::Rgb(20, 18, 20),
            surface: Color::Rgb(34, 28, 30),
            text: Color::Rgb(235, 230, 230),
            muted: Color::Rgb(150, 140, 142),
            dim: Color::Rgb(90, 74, 78),
            accent: Color::Rgb(200, 60, 70),
            accent_alt: Color::Rgb(230, 140, 90),
            border: Color::Rgb(90, 74, 78),
            border_focused: Color::Rgb(200, 60, 70),
            selection_bg: Color::Rgb(120, 30, 40),
            selection_fg: Color::Rgb(240, 235, 235),
            code: CodeTheme::default(),
        }
    }
}

impl Default for CodeTheme {
    fn default() -> Self {
        // A warm, low-contrast syntax palette that sits alongside tuika's
        // default red-on-dark identity. Hosts with their own look build their
        // own `CodeTheme` (see yolop's `yolop_theme`).
        CodeTheme {
            heading: Color::Rgb(235, 230, 230),
            link: Color::Rgb(120, 160, 210),
            background: Color::Rgb(28, 24, 26),
            text: Color::Rgb(210, 205, 205),
            label: Color::Rgb(150, 140, 142),
            keyword: Color::Rgb(210, 120, 90),
            function: Color::Rgb(120, 160, 210),
            type_name: Color::Rgb(126, 170, 176),
            constant: Color::Rgb(200, 160, 120),
            string: Color::Rgb(150, 180, 140),
            comment: Color::Rgb(120, 110, 112),
            punctuation: Color::Rgb(150, 140, 142),
        }
    }
}

impl Theme {
    /// Style for primary body text (`text` foreground).
    pub fn text_style(&self) -> Style {
        Style::default().fg(self.text)
    }

    /// Style for de-emphasized text (`muted` foreground).
    pub fn muted_style(&self) -> Style {
        Style::default().fg(self.muted)
    }

    /// Bold style in the primary `accent` color.
    pub fn accent_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Border color for a component, `border_focused` when `focused` else `border`.
    pub fn border_color(&self, focused: bool) -> Color {
        if focused {
            self.border_focused
        } else {
            self.border
        }
    }

    /// Bold style for a selected item (`selection_fg` on `selection_bg`).
    pub fn selection_style(&self) -> Style {
        Style::default()
            .bg(self.selection_bg)
            .fg(self.selection_fg)
            .add_modifier(Modifier::BOLD)
    }

    /// Resolve a generic status role without requiring additional public
    /// fields in [`Theme`].
    ///
    /// The mapping reuses the syntax palette's established semantic colors:
    /// strings for success, constants for warning, keywords for danger, and
    /// links for info. This keeps existing downstream `Theme { .. }` literals
    /// source-compatible while giving every custom and bundled theme sensible
    /// status colors.
    pub fn semantic_color(&self, role: SemanticRole) -> Color {
        match role {
            SemanticRole::Success => self.code.string,
            SemanticRole::Warning => self.code.constant,
            SemanticRole::Danger => self.code.keyword,
            SemanticRole::Info => self.code.link,
        }
    }

    /// Bold foreground style for a generic status role.
    pub fn semantic_style(&self, role: SemanticRole) -> Style {
        Style::default()
            .fg(self.semantic_color(role))
            .add_modifier(Modifier::BOLD)
    }

    /// Bold success style.
    pub fn success_style(&self) -> Style {
        self.semantic_style(SemanticRole::Success)
    }

    /// Bold warning style.
    pub fn warning_style(&self) -> Style {
        self.semantic_style(SemanticRole::Warning)
    }

    /// Bold danger/error style.
    pub fn danger_style(&self) -> Style {
        self.semantic_style(SemanticRole::Danger)
    }

    /// Bold informational style.
    pub fn info_style(&self) -> Style {
        self.semantic_style(SemanticRole::Info)
    }
}

/// Generic semantic status roles shared by notifications, validation, and
/// application-defined views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticRole {
    /// Successful or completed state.
    Success,
    /// Warning or caution state.
    Warning,
    /// Error, destructive, or dangerous state.
    Danger,
    /// Informational or neutral status.
    Info,
}

/// A resolved, partial style for one semantic [`Role`].
///
/// This is the "declaration block" of tuika's stylesheet model: the `Theme` is
/// the token layer (named *colors*), and a `StyleBundle` is what a role maps
/// those tokens onto — a foreground/background plus text modifiers, and, for
/// container roles, a border glyph style and padding. Every field is optional so
/// a bundle can *contribute* (add a modifier, override just the foreground)
/// rather than replace a whole [`Style`]: [`apply`](Self::apply) overlays only
/// the fields it sets onto a base style, and adds its modifiers.
///
/// A component resolves layout-affecting fields from the same [`RenderCtx`] in
/// both [`measure`] and render. Border *presence* remains instance-level;
/// container padding may come from a role, with an explicit component value
/// taking precedence. Color, background, modifiers, and border glyph choice are
/// paint attributes.
///
/// [`measure`]: crate::View::measure
/// [`RenderCtx`]: crate::RenderCtx
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StyleBundle {
    /// Foreground color, when the role sets one.
    pub fg: Option<Color>,
    /// Background color, when the role sets one.
    pub bg: Option<Color>,
    /// Text modifiers the role adds (bold, italic, underline, …).
    pub add_modifier: Modifier,
    /// Border glyph style for container roles (see the note above: this must not
    /// change a component's footprint, so it selects the glyph set, not whether
    /// a border exists).
    pub border: Option<BorderStyle>,
    /// Padding for container roles, when the role sets one. Components that
    /// consume it resolve it during both measurement and rendering.
    pub padding: Option<Padding>,
}

impl StyleBundle {
    /// An empty bundle that overrides nothing and adds no modifiers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the foreground color.
    pub fn fg(mut self, color: Color) -> Self {
        self.fg = Some(color);
        self
    }

    /// Set the background color.
    pub fn bg(mut self, color: Color) -> Self {
        self.bg = Some(color);
        self
    }

    /// Add text modifiers (accumulates with any already set).
    pub fn modifier(mut self, m: Modifier) -> Self {
        self.add_modifier |= m;
        self
    }

    /// Add the bold modifier.
    pub fn bold(self) -> Self {
        self.modifier(Modifier::BOLD)
    }

    /// Add the italic modifier.
    pub fn italic(self) -> Self {
        self.modifier(Modifier::ITALIC)
    }

    /// Add the underline modifier.
    pub fn underlined(self) -> Self {
        self.modifier(Modifier::UNDERLINED)
    }

    /// Add the crossed-out (strikethrough) modifier.
    pub fn crossed_out(self) -> Self {
        self.modifier(Modifier::CROSSED_OUT)
    }

    /// Set the border glyph style (container roles).
    pub fn border(mut self, border: BorderStyle) -> Self {
        self.border = Some(border);
        self
    }

    /// Set the padding (container roles).
    pub fn padding(mut self, padding: Padding) -> Self {
        self.padding = Some(padding);
        self
    }

    /// Overlay this bundle onto `base`: `fg`/`bg` replace the base's when set,
    /// and this bundle's modifiers are added. Unset fields leave `base` as-is, so
    /// a modifier-only bundle (e.g. emphasis) keeps the inherited color.
    pub fn apply(&self, base: Style) -> Style {
        let mut style = base;
        if let Some(fg) = self.fg {
            style = style.fg(fg);
        }
        if let Some(bg) = self.bg {
            style = style.bg(bg);
        }
        style.add_modifier(self.add_modifier)
    }

    /// Resolve this bundle to a standalone [`Style`] over the default.
    pub fn to_style(&self) -> Style {
        self.apply(Style::default())
    }

    /// Overlay `override_bundle` onto this bundle.
    ///
    /// Set fields replace the base, unset fields inherit it, and modifiers are
    /// combined. This is how a host style resolver can change one attribute of
    /// a built-in component role without restating its complete default.
    pub fn overlay(self, override_bundle: StyleBundle) -> StyleBundle {
        StyleBundle {
            fg: override_bundle.fg.or(self.fg),
            bg: override_bundle.bg.or(self.bg),
            add_modifier: self.add_modifier | override_bundle.add_modifier,
            border: override_bundle.border.or(self.border),
            padding: override_bundle.padding.or(self.padding),
        }
    }
}

/// A semantic element a component styles itself as. The stylesheet maps each
/// role to a [`StyleBundle`]; this is the "selector" half of the model, kept a
/// closed enum (rather than open string classes) so a typo is a compile error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// A bordered panel / framed container ([`Boxed`](crate::components::Boxed)).
    Panel,
    /// A markdown heading (`# …`).
    Heading,
    /// A hyperlink or bare URL.
    Link,
    /// Inline `code` spans.
    InlineCode,
    /// Emphasized (`*italic*`) text.
    Emphasis,
    /// Strong (`**bold**`) text.
    Strong,
    /// Struck-through (`~~text~~`) text.
    Strikethrough,
    /// A list item's bullet or number marker.
    ListMarker,
    /// A task-list checkbox marker.
    TaskMarker,
    /// A thematic break / horizontal rule.
    Rule,
    /// The glyph marking an inline image placeholder.
    ImageMarker,
}

/// An open semantic style key used by components and host-defined views.
///
/// Tuika publishes constants for its built-in component roles. Applications
/// and companion crates may define their own namespaced constants with
/// [`StyleRole::new`] and resolve them through a [`StyleResolver`] installed on
/// [`RenderCtx`](crate::RenderCtx). The static string is an identifier, never
/// terminal output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StyleRole(&'static str);

impl StyleRole {
    /// Define a stable, namespaced semantic role.
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// The role's stable identifier.
    pub const fn name(self) -> &'static str {
        self.0
    }

    /// Base text and fill of a toast row.
    pub const TOAST: Self = Self::new("tuika.toast");
    /// Informational toast accent.
    pub const TOAST_INFO: Self = Self::new("tuika.toast.info");
    /// Successful toast accent.
    pub const TOAST_SUCCESS: Self = Self::new("tuika.toast.success");
    /// Warning toast accent.
    pub const TOAST_WARNING: Self = Self::new("tuika.toast.warning");
    /// Error toast accent.
    pub const TOAST_ERROR: Self = Self::new("tuika.toast.error");
    /// Diff block background.
    pub const DIFF: Self = Self::new("tuika.diff");
    /// Inserted diff row.
    pub const DIFF_ADDED: Self = Self::new("tuika.diff.added");
    /// Removed diff row.
    pub const DIFF_REMOVED: Self = Self::new("tuika.diff.removed");
    /// Unchanged diff row.
    pub const DIFF_CONTEXT: Self = Self::new("tuika.diff.context");
    /// Diff line-number gutter.
    pub const DIFF_GUTTER: Self = Self::new("tuika.diff.gutter");
    /// Divider between side-by-side diff columns.
    pub const DIFF_DIVIDER: Self = Self::new("tuika.diff.divider");
    /// Key-cap portion of an action hint.
    pub const KEY_HINT_KEY: Self = Self::new("tuika.key-hint.key");
    /// Label portion of an action hint.
    pub const KEY_HINT_LABEL: Self = Self::new("tuika.key-hint.label");
}

/// Optional host policy for built-in or application-defined [`StyleRole`]s.
///
/// Returned bundles overlay the active [`StyleSheet`] defaults. Resolvers are
/// expected to be cheap and side-effect free. A resolver with interior mutable
/// policy should increment [`revision`](Self::revision) whenever its answers
/// change so measurement caches can invalidate.
pub trait StyleResolver {
    /// Resolve `role`, or return `None` to keep the stylesheet/default result.
    fn resolve(&self, role: StyleRole) -> Option<StyleBundle>;

    /// Monotonic policy revision for cache invalidation.
    fn revision(&self) -> u64 {
        0
    }
}

/// The rule layer of tuika's styling model: a mapping from every [`Role`] to the
/// [`StyleBundle`] a component resolves for it.
///
/// Where [`Theme`] centralizes the *colors* a whole component tree draws from,
/// `StyleSheet` centralizes the *choice of style per role* — so a host can, in
/// one place, make links green-and-bold, panels thick-bordered, or every panel
/// share a surface fill, without touching component code or the raw colors used
/// elsewhere. Like [`Theme`] it is a flat `Copy` struct of named slots, built
/// once (usually with [`from_theme`](Self::from_theme)) and threaded through
/// [`RenderCtx`](crate::RenderCtx).
///
/// Override a rule with struct-update syntax:
///
/// ```
/// use tuika::style::{StyleBundle, StyleSheet, Theme};
/// use ratatui_core::style::Color;
///
/// let theme = Theme::default();
/// let sheet = StyleSheet {
///     link: StyleBundle::new().fg(Color::Green).bold().underlined(),
///     ..StyleSheet::from_theme(&theme)
/// };
/// assert_eq!(sheet.link.fg, Some(Color::Green));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StyleSheet {
    /// Border color/background for a [`Boxed`](crate::components::Boxed) panel.
    pub panel: StyleBundle,
    /// Markdown heading text.
    pub heading: StyleBundle,
    /// Links and bare URLs.
    pub link: StyleBundle,
    /// Inline code spans.
    pub inline_code: StyleBundle,
    /// Emphasized text (modifier-only, keeps the inherited color).
    pub emphasis: StyleBundle,
    /// Strong text (modifier-only).
    pub strong: StyleBundle,
    /// Struck-through text (modifier-only).
    pub strikethrough: StyleBundle,
    /// List bullet / number markers.
    pub list_marker: StyleBundle,
    /// Task-list checkbox markers.
    pub task_marker: StyleBundle,
    /// Thematic-break rules.
    pub rule: StyleBundle,
    /// Inline-image placeholder marker glyph.
    pub image_marker: StyleBundle,
    /// Base text and fill of toast rows.
    pub toast: StyleBundle,
    /// Informational toast accent.
    pub toast_info: StyleBundle,
    /// Successful toast accent.
    pub toast_success: StyleBundle,
    /// Warning toast accent.
    pub toast_warning: StyleBundle,
    /// Error toast accent.
    pub toast_error: StyleBundle,
    /// Diff block background.
    pub diff: StyleBundle,
    /// Inserted diff rows.
    pub diff_added: StyleBundle,
    /// Removed diff rows.
    pub diff_removed: StyleBundle,
    /// Unchanged diff rows.
    pub diff_context: StyleBundle,
    /// Diff line-number gutter.
    pub diff_gutter: StyleBundle,
    /// Side-by-side diff divider.
    pub diff_divider: StyleBundle,
    /// Key-cap portion of action hints.
    pub key_hint_key: StyleBundle,
    /// Label portion of action hints.
    pub key_hint_label: StyleBundle,
}

impl StyleSheet {
    /// The default stylesheet for `theme`: every built-in role mapped to a
    /// theme-derived style. Start from this and override the roles you want to
    /// restyle.
    pub fn from_theme(theme: &Theme) -> Self {
        let code = &theme.code;
        StyleSheet {
            // Panel background/border color default to unset so a plain `Boxed`
            // keeps its no-fill, theme-bordered look; a host opts panels into a
            // shared fill by setting `panel.bg`.
            panel: StyleBundle::new(),
            heading: StyleBundle::new().fg(code.heading).bold(),
            link: StyleBundle::new().fg(code.link).underlined(),
            inline_code: StyleBundle::new().fg(code.text).bg(code.background),
            emphasis: StyleBundle::new().italic(),
            strong: StyleBundle::new().bold(),
            strikethrough: StyleBundle::new().crossed_out(),
            list_marker: StyleBundle::new().fg(theme.accent_alt),
            task_marker: StyleBundle::new().fg(theme.accent),
            rule: StyleBundle::new().fg(theme.dim),
            image_marker: StyleBundle::new().fg(theme.accent),
            toast: StyleBundle::new().fg(theme.text).bg(theme.surface),
            toast_info: StyleBundle::new()
                .fg(theme.semantic_color(SemanticRole::Info))
                .bg(theme.surface),
            toast_success: StyleBundle::new()
                .fg(theme.semantic_color(SemanticRole::Success))
                .bg(theme.surface),
            toast_warning: StyleBundle::new()
                .fg(theme.semantic_color(SemanticRole::Warning))
                .bg(theme.surface),
            toast_error: StyleBundle::new()
                .fg(theme.semantic_color(SemanticRole::Danger))
                .bg(theme.surface),
            diff: StyleBundle::new().bg(code.background),
            diff_added: StyleBundle::new().fg(theme.semantic_color(SemanticRole::Success)),
            diff_removed: StyleBundle::new().fg(theme.semantic_color(SemanticRole::Danger)),
            diff_context: StyleBundle::new().fg(theme.muted),
            diff_gutter: StyleBundle::new().fg(theme.muted),
            diff_divider: StyleBundle::new().fg(theme.dim),
            key_hint_key: StyleBundle::new().fg(Color::Black).bg(theme.accent),
            key_hint_label: StyleBundle::new().fg(theme.muted),
        }
    }

    /// The [`StyleBundle`] mapped to `role`.
    pub fn resolve(&self, role: Role) -> StyleBundle {
        match role {
            Role::Panel => self.panel,
            Role::Heading => self.heading,
            Role::Link => self.link,
            Role::InlineCode => self.inline_code,
            Role::Emphasis => self.emphasis,
            Role::Strong => self.strong,
            Role::Strikethrough => self.strikethrough,
            Role::ListMarker => self.list_marker,
            Role::TaskMarker => self.task_marker,
            Role::Rule => self.rule,
            Role::ImageMarker => self.image_marker,
        }
    }

    /// Resolve a built-in open [`StyleRole`]. Unknown application roles return
    /// `None` for a host [`StyleResolver`] to own.
    pub fn resolve_style(&self, role: StyleRole) -> Option<StyleBundle> {
        Some(match role {
            StyleRole::TOAST => self.toast,
            StyleRole::TOAST_INFO => self.toast_info,
            StyleRole::TOAST_SUCCESS => self.toast_success,
            StyleRole::TOAST_WARNING => self.toast_warning,
            StyleRole::TOAST_ERROR => self.toast_error,
            StyleRole::DIFF => self.diff,
            StyleRole::DIFF_ADDED => self.diff_added,
            StyleRole::DIFF_REMOVED => self.diff_removed,
            StyleRole::DIFF_CONTEXT => self.diff_context,
            StyleRole::DIFF_GUTTER => self.diff_gutter,
            StyleRole::DIFF_DIVIDER => self.diff_divider,
            StyleRole::KEY_HINT_KEY => self.key_hint_key,
            StyleRole::KEY_HINT_LABEL => self.key_hint_label,
            _ => return None,
        })
    }
}

impl Default for StyleSheet {
    /// The stylesheet for the default [`Theme`].
    fn default() -> Self {
        Self::from_theme(&Theme::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::support::rainbow_theme;
    use ratatui_core::style::Modifier;

    #[test]
    fn theme_helper_styles_map_to_slots() {
        let t = rainbow_theme();
        assert_eq!(t.text_style().fg, Some(t.text));
        assert_eq!(t.muted_style().fg, Some(t.muted));
        assert_eq!(t.accent_style().fg, Some(t.accent));
        assert!(t.accent_style().add_modifier.contains(Modifier::BOLD));
        assert_eq!(t.border_color(false), t.border);
        assert_eq!(t.border_color(true), t.border_focused);
        let sel = t.selection_style();
        assert_eq!(sel.bg, Some(t.selection_bg));
        assert_eq!(sel.fg, Some(t.selection_fg));
        assert!(sel.add_modifier.contains(Modifier::BOLD));
        assert_eq!(
            t.success_style().fg,
            Some(t.semantic_color(SemanticRole::Success))
        );
        assert_eq!(
            t.warning_style().fg,
            Some(t.semantic_color(SemanticRole::Warning))
        );
        assert_eq!(
            t.danger_style().fg,
            Some(t.semantic_color(SemanticRole::Danger))
        );
        assert_eq!(
            t.info_style().fg,
            Some(t.semantic_color(SemanticRole::Info))
        );
    }

    #[test]
    fn default_theme_is_the_toolkit_identity_not_a_host_brand() {
        use ratatui_core::style::Color;
        // tuika's own look is warm red-on-dark. It is deliberately a neutral toolkit
        // identity, not any host's brand — a host with its own palette builds its own
        // `Theme` (e.g. yolop's `fullscreen::yolop_theme`) instead of inheriting this.
        let t = Theme::default();
        assert_eq!(t.accent, Color::Rgb(200, 60, 70));
        assert_eq!(t.selection_bg, Color::Rgb(120, 30, 40));
        // Not yolop's accent blue.
        assert_ne!(t.accent, Color::Rgb(45, 91, 158));
    }

    #[test]
    fn bundle_apply_overrides_color_but_only_adds_modifiers() {
        use ratatui_core::style::Color;
        // A modifier-only bundle keeps the base's color and adds its modifier.
        let base = Style::default().fg(Color::Red);
        let emphasized = StyleBundle::new().italic().apply(base);
        assert_eq!(emphasized.fg, Some(Color::Red), "color inherited");
        assert!(emphasized.add_modifier.contains(Modifier::ITALIC));

        // A bundle that sets fg replaces the base's fg.
        let recolored = StyleBundle::new().fg(Color::Green).bold().apply(base);
        assert_eq!(recolored.fg, Some(Color::Green));
        assert!(recolored.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn bundle_overlay_inherits_unset_attributes() {
        let base = StyleBundle::new().fg(Color::Red).bg(Color::Black).bold();
        let merged = base.overlay(StyleBundle::new().fg(Color::Green).italic());
        assert_eq!(merged.fg, Some(Color::Green));
        assert_eq!(merged.bg, Some(Color::Black));
        assert!(merged.add_modifier.contains(Modifier::BOLD));
        assert!(merged.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn from_theme_reproduces_the_pre_stylesheet_styles() {
        let t = rainbow_theme();
        let s = StyleSheet::from_theme(&t);
        // Heading = code.heading + bold; link = code.link + underline; these are
        // exactly what the components hard-coded before the stylesheet existed.
        assert_eq!(s.heading.fg, Some(t.code.heading));
        assert!(s.heading.add_modifier.contains(Modifier::BOLD));
        assert_eq!(s.link.fg, Some(t.code.link));
        assert!(s.link.add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(s.inline_code.fg, Some(t.code.text));
        assert_eq!(s.inline_code.bg, Some(t.code.background));
        // Emphasis is modifier-only so it keeps the surrounding prose color.
        assert_eq!(s.emphasis.fg, None);
        assert!(s.emphasis.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn resolve_matches_named_fields() {
        let s = StyleSheet::from_theme(&rainbow_theme());
        assert_eq!(s.resolve(Role::Heading), s.heading);
        assert_eq!(s.resolve(Role::Link), s.link);
        assert_eq!(s.resolve(Role::Panel), s.panel);
        assert_eq!(s.resolve_style(StyleRole::TOAST_ERROR), Some(s.toast_error));
        assert_eq!(s.resolve_style(StyleRole::DIFF_ADDED), Some(s.diff_added));
        assert_eq!(
            s.resolve_style(StyleRole::KEY_HINT_LABEL),
            Some(s.key_hint_label)
        );
        assert_eq!(s.resolve_style(StyleRole::new("app.unknown")), None);
    }

    #[test]
    fn overriding_one_role_leaves_the_rest_at_theme_defaults() {
        use ratatui_core::style::Color;
        let t = rainbow_theme();
        let sheet = StyleSheet {
            link: StyleBundle::new().fg(Color::Green).bold(),
            ..StyleSheet::from_theme(&t)
        };
        assert_eq!(sheet.link.fg, Some(Color::Green));
        assert!(sheet.link.add_modifier.contains(Modifier::BOLD));
        // Everything else still tracks the theme.
        assert_eq!(sheet.heading.fg, Some(t.code.heading));
    }
}
