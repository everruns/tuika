//! [`Diff`] — a line-oriented text diff, unified or side-by-side.
//!
//! The component computes an LCS line diff of an `old` and `new` text and
//! renders it two ways: **unified** (one column, `+`/`-`/` ` markers) or
//! **side-by-side** (old on the left, new on the right, a divider between). Both
//! carry an optional line-number gutter. Colors resolve from the active semantic
//! stylesheet and host resolver; [`DiffStyle`] remains the per-instance override.
//!
//! The diff itself ([`rows`]) is a pure function, so classification is
//! testable without rendering.

use crate::geometry::Rect;
use crate::style::{Color, Modifier, Style};

use crate::geometry::Size;
use crate::style::StyleRole;
use crate::surface::Surface;
use crate::view::{RenderCtx, View};

/// How a diff line relates the two texts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffTag {
    /// Present unchanged in both texts.
    Equal,
    /// Present only in the old text (a removal).
    Delete,
    /// Present only in the new text (an addition).
    Insert,
}

/// One classified line of a diff: its [`DiffTag`], the 1-based line numbers on
/// each side (`None` on the side it does not appear), and the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffRow {
    /// How this line relates the two texts.
    pub tag: DiffTag,
    /// 1-based line number in the old text, or `None` for an insertion.
    pub old_line: Option<usize>,
    /// 1-based line number in the new text, or `None` for a deletion.
    pub new_line: Option<usize>,
    /// The line's text (without the trailing newline).
    pub text: String,
}

/// Above this many `old.len() * new.len()` cells the quadratic LCS table is
/// skipped for a trivial "all removed, then all added" diff, bounding memory on
/// pathological inputs.
const LCS_CELL_CAP: usize = 4_000_000;

/// Classify every line of `old` vs `new` into an ordered list of [`DiffRow`]s
/// via a longest-common-subsequence line diff.
///
/// Ties (a line could be equally well explained as a delete or an insert) resolve
/// toward deletes-before-inserts, matching the usual unified-diff ordering. For
/// very large inputs it falls back to emitting all deletions followed by all
/// insertions rather than allocate a table whose cell count is unbounded.
pub fn rows(old: &str, new: &str) -> Vec<DiffRow> {
    let old: Vec<&str> = if old.is_empty() {
        Vec::new()
    } else {
        old.split('\n').collect()
    };
    let new: Vec<&str> = if new.is_empty() {
        Vec::new()
    } else {
        new.split('\n').collect()
    };
    let (n, m) = (old.len(), new.len());

    let mut rows = Vec::new();

    if n.saturating_mul(m) > LCS_CELL_CAP {
        for (i, line) in old.iter().enumerate() {
            rows.push(DiffRow {
                tag: DiffTag::Delete,
                old_line: Some(i + 1),
                new_line: None,
                text: (*line).to_string(),
            });
        }
        for (j, line) in new.iter().enumerate() {
            rows.push(DiffRow {
                tag: DiffTag::Insert,
                old_line: None,
                new_line: Some(j + 1),
                text: (*line).to_string(),
            });
        }
        return rows;
    }

    // dp[i][j] = LCS length of old[i..] and new[j..].
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if old[i] == new[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if old[i] == new[j] {
            rows.push(DiffRow {
                tag: DiffTag::Equal,
                old_line: Some(i + 1),
                new_line: Some(j + 1),
                text: old[i].to_string(),
            });
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            rows.push(DiffRow {
                tag: DiffTag::Delete,
                old_line: Some(i + 1),
                new_line: None,
                text: old[i].to_string(),
            });
            i += 1;
        } else {
            rows.push(DiffRow {
                tag: DiffTag::Insert,
                old_line: None,
                new_line: Some(j + 1),
                text: new[j].to_string(),
            });
            j += 1;
        }
    }
    while i < n {
        rows.push(DiffRow {
            tag: DiffTag::Delete,
            old_line: Some(i + 1),
            new_line: None,
            text: old[i].to_string(),
        });
        i += 1;
    }
    while j < m {
        rows.push(DiffRow {
            tag: DiffTag::Insert,
            old_line: None,
            new_line: Some(j + 1),
            text: new[j].to_string(),
        });
        j += 1;
    }
    rows
}

/// Layout of a [`Diff`]: one unified column or two side-by-side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiffMode {
    /// One column, changes marked with `+`/`-`/` ` (the default).
    #[default]
    Unified,
    /// Old on the left, new on the right, a divider between.
    SideBySide,
}

/// The foreground colors a [`Diff`] draws additions, removals, and context with.
///
/// Defaults to conventional green/red with context in neutral gray. Passing a
/// `DiffStyle` is an explicit per-instance override and takes precedence over
/// semantic stylesheet and resolver colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffStyle {
    /// Foreground for inserted lines.
    pub added: Color,
    /// Foreground for removed lines.
    pub removed: Color,
    /// Foreground for unchanged (context) lines.
    pub context: Color,
}

impl Default for DiffStyle {
    fn default() -> Self {
        Self {
            added: Color::Rgb(120, 190, 120),
            removed: Color::Rgb(220, 110, 110),
            context: Color::Rgb(150, 150, 150),
        }
    }
}

/// A rendered line-diff view over the rows produced by [`rows`].
///
/// ```
/// use tuika::components::diff::rows;
/// use tuika::prelude::*;
/// // The pure classifier is usable on its own:
/// let rows = rows("a\nb\nc", "a\nB\nc");
/// assert_eq!(rows[1].tag, DiffTag::Delete);
/// assert_eq!(rows[2].tag, DiffTag::Insert);
/// // …or wrap it in a view.
/// let _view = Diff::new("a\nb", "a\nc").mode(DiffMode::SideBySide).line_numbers(true);
/// ```
pub struct Diff {
    rows: Vec<DiffRow>,
    mode: DiffMode,
    line_numbers: bool,
    style: Option<DiffStyle>,
}

impl Diff {
    /// Diff `old` against `new` (each split into lines on `\n`).
    pub fn new(old: &str, new: &str) -> Self {
        Self::from_rows(rows(old, new))
    }

    /// Build a view from pre-computed rows (e.g. from an external differ).
    pub fn from_rows(rows: Vec<DiffRow>) -> Self {
        Self {
            rows,
            mode: DiffMode::default(),
            line_numbers: false,
            style: None,
        }
    }

    /// Choose unified (default) or side-by-side layout.
    pub fn mode(mut self, mode: DiffMode) -> Self {
        self.mode = mode;
        self
    }

    /// Show a line-number gutter (old and new numbers).
    pub fn line_numbers(mut self, show: bool) -> Self {
        self.line_numbers = show;
        self
    }

    /// Override the add/remove/context colors.
    pub fn style(mut self, style: DiffStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// Width of a line-number column: digits of the largest number on either side.
    fn number_width(&self) -> u16 {
        if !self.line_numbers {
            return 0;
        }
        let max = self
            .rows
            .iter()
            .flat_map(|r| [r.old_line, r.new_line])
            .flatten()
            .max()
            .unwrap_or(1);
        max.to_string().len() as u16
    }

    fn row_style(&self, tag: DiffTag, ctx: &RenderCtx) -> Style {
        let role = match tag {
            DiffTag::Equal => StyleRole::DIFF_CONTEXT,
            DiffTag::Insert => StyleRole::DIFF_ADDED,
            DiffTag::Delete => StyleRole::DIFF_REMOVED,
        };
        let resolved = ctx.style(role).apply(ctx.style(StyleRole::DIFF).to_style());
        match self.style {
            Some(style) => resolved.fg(match tag {
                DiffTag::Equal => style.context,
                DiffTag::Insert => style.added,
                DiffTag::Delete => style.removed,
            }),
            None => resolved,
        }
    }

    fn render_unified(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        let base = ctx.style(StyleRole::DIFF).to_style();
        let gutter_style = ctx.style(StyleRole::DIFF_GUTTER).apply(base);
        let numw = self.number_width();
        for (row, dr) in self.rows.iter().enumerate() {
            let y = area.y.saturating_add(row as u16);
            if y >= area.bottom() {
                break;
            }
            let style = self.row_style(dr.tag, ctx);
            let marker = match dr.tag {
                DiffTag::Equal => ' ',
                DiffTag::Insert => '+',
                DiffTag::Delete => '-',
            };
            // Fill the row background first so short lines still read as a block.
            for x in area.x..area.right() {
                surface.set(x, y, ' ', base);
            }
            let mut x = area.x;
            if numw > 0 {
                let old = dr.old_line.map(|n| n.to_string()).unwrap_or_default();
                let new = dr.new_line.map(|n| n.to_string()).unwrap_or_default();
                let gutter = format!(
                    "{old:>ow$} {new:>nw$} ",
                    ow = numw as usize,
                    nw = numw as usize
                );
                x = surface.set_string(x, y, &gutter, gutter_style);
            }
            x = surface.set_string(
                x,
                y,
                &format!("{marker} "),
                style.add_modifier(Modifier::BOLD),
            );
            surface.set_string(x, y, &dr.text, style);
        }
    }

    fn render_side_by_side(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        let base = ctx.style(StyleRole::DIFF).to_style();
        let numw = self.number_width();
        // Divider column between the two halves.
        let divider_style = ctx.style(StyleRole::DIFF_DIVIDER).apply(base);
        let usable = area.width.saturating_sub(3); // " │ "
        let col_w = usable / 2;
        if col_w == 0 {
            // Too narrow to split; fall back to unified.
            self.render_unified(area, surface, ctx);
            return;
        }
        let left = Rect::new(area.x, area.y, col_w, area.height);
        let div_x = area.x + col_w + 1;
        let right = Rect::new(area.x + col_w + 3, area.y, col_w, area.height);

        for (row, dr) in self.rows.iter().enumerate() {
            let y = area.y.saturating_add(row as u16);
            if y >= area.bottom() {
                break;
            }
            // Background fill for the whole row.
            for x in area.x..area.right() {
                surface.set(x, y, ' ', base);
            }
            surface.set(div_x, y, '│', divider_style);

            // Left side shows deletions and context; right side insertions and
            // context. The tag decides which sides are populated.
            let (show_left, show_right) = match dr.tag {
                DiffTag::Equal => (true, true),
                DiffTag::Delete => (true, false),
                DiffTag::Insert => (false, true),
            };
            if show_left {
                self.render_cell(surface, left, y, dr.old_line, dr, '-', numw, ctx);
            }
            if show_right {
                self.render_cell(surface, right, y, dr.new_line, dr, '+', numw, ctx);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_cell(
        &self,
        surface: &mut Surface,
        cell: Rect,
        y: u16,
        number: Option<usize>,
        dr: &DiffRow,
        change_marker: char,
        numw: u16,
        ctx: &RenderCtx,
    ) {
        let style = self.row_style(dr.tag, ctx);
        let mut sub = surface.child(cell);
        let mut x = cell.x;
        if numw > 0 {
            let num = number.map(|n| n.to_string()).unwrap_or_default();
            x = sub.set_string(
                x,
                y,
                &format!("{num:>w$} ", w = numw as usize),
                ctx.style(StyleRole::DIFF_GUTTER)
                    .apply(ctx.style(StyleRole::DIFF).to_style()),
            );
        }
        let marker = if dr.tag == DiffTag::Equal {
            ' '
        } else {
            change_marker
        };
        x = sub.set_string(
            x,
            y,
            &format!("{marker} "),
            style.add_modifier(Modifier::BOLD),
        );
        sub.set_string(x, y, &dr.text, style);
    }

    /// The width the widest row wants, for `measure`.
    fn intrinsic_width(&self) -> u16 {
        let numw = self.number_width();
        let text = self
            .rows
            .iter()
            .map(|r| crate::width::str_cols(&r.text))
            .max()
            .unwrap_or(0);
        // marker + space + text, plus the number columns.
        let base = text.saturating_add(2);
        match self.mode {
            DiffMode::Unified => base.saturating_add(numw.saturating_mul(2).saturating_add(2)),
            // Two columns of (number + marker + text) plus the divider.
            DiffMode::SideBySide => base
                .saturating_add(numw.saturating_add(1))
                .saturating_mul(2)
                .saturating_add(3),
        }
    }
}

impl View for Diff {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        Size::new(
            self.intrinsic_width().min(available.width),
            (self.rows.len() as u16).min(available.height),
        )
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.is_empty() {
            return;
        }
        match self.mode {
            DiffMode::Unified => self.render_unified(area, surface, ctx),
            DiffMode::SideBySide => self.render_side_by_side(area, surface, ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Theme;
    use crate::tests::support::row;

    #[test]
    fn classifies_equal_delete_insert() {
        let rows = rows("a\nb\nc", "a\nB\nc");
        let tags: Vec<DiffTag> = rows.iter().map(|r| r.tag).collect();
        // "b" removed, "B" added, surrounded by equal context.
        assert_eq!(
            tags,
            vec![
                DiffTag::Equal,
                DiffTag::Delete,
                DiffTag::Insert,
                DiffTag::Equal
            ]
        );
        // Line numbers track each side independently.
        let del = rows.iter().find(|r| r.tag == DiffTag::Delete).unwrap();
        assert_eq!((del.old_line, del.new_line), (Some(2), None));
        let ins = rows.iter().find(|r| r.tag == DiffTag::Insert).unwrap();
        assert_eq!((ins.old_line, ins.new_line), (None, Some(2)));
    }

    #[test]
    fn empty_sides_are_pure_insert_or_delete() {
        assert!(rows("", "").is_empty());
        let added = rows("", "x\ny");
        assert!(added.iter().all(|r| r.tag == DiffTag::Insert));
        assert_eq!(added.len(), 2);
        let removed = rows("x\ny", "");
        assert!(removed.iter().all(|r| r.tag == DiffTag::Delete));
    }

    #[test]
    fn unified_renders_markers_and_numbers() {
        let theme = Theme::default();
        let diff = Diff::new("a\nb", "a\nc").line_numbers(true);
        let buf = crate::testing::render(&diff, 30, 3, &theme);
        let whole: String = (0..buf.area.height)
            .map(|y| row(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(whole.contains('-'), "a removal marker: {whole:?}");
        assert!(whole.contains('+'), "an addition marker: {whole:?}");
        assert!(whole.contains('b'));
        assert!(whole.contains('c'));
    }

    #[test]
    fn side_by_side_has_divider_and_both_texts() {
        let theme = Theme::default();
        let diff = Diff::new("a\nb", "a\nc").mode(DiffMode::SideBySide);
        let buf = crate::testing::render(&diff, 40, 3, &theme);
        let whole: String = (0..buf.area.height)
            .map(|y| row(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(whole.contains('│'), "column divider present: {whole:?}");
        assert!(whole.contains('b') && whole.contains('c'));
    }

    #[test]
    fn added_and_removed_use_style_colors() {
        let theme = Theme::default();
        let style = DiffStyle {
            added: Color::Rgb(1, 2, 3),
            removed: Color::Rgb(4, 5, 6),
            context: Color::Rgb(7, 8, 9),
        };
        let diff = Diff::new("keep\nremove", "keep\nadd").style(style);
        let buf = crate::testing::render(&diff, 20, 3, &theme);
        let mut fgs = Vec::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                fgs.push(buf[(x, y)].fg);
            }
        }
        assert!(fgs.contains(&Color::Rgb(1, 2, 3)), "added color used");
        assert!(fgs.contains(&Color::Rgb(4, 5, 6)), "removed color used");
    }

    #[test]
    fn semantic_roles_style_diff_and_instance_override_wins() {
        let theme = Theme::default();
        let sheet = crate::StyleSheet {
            diff: crate::style::StyleBundle::new().bg(Color::Blue),
            diff_added: crate::style::StyleBundle::new().fg(Color::Yellow),
            diff_removed: crate::style::StyleBundle::new().fg(Color::Magenta),
            ..crate::StyleSheet::from_theme(&theme)
        };
        let diff = Diff::new("old", "new");
        let buffer = crate::testing::render_with_sheet(&diff, 12, 2, &theme, sheet);
        assert_eq!(buffer[(0, 0)].fg, Color::Magenta);
        assert_eq!(buffer[(0, 0)].bg, Color::Blue);
        assert_eq!(buffer[(0, 1)].fg, Color::Yellow);

        let override_diff = Diff::new("old", "new").style(DiffStyle {
            added: Color::Green,
            removed: Color::Red,
            context: Color::White,
        });
        let buffer = crate::testing::render_with_sheet(&override_diff, 12, 2, &theme, sheet);
        assert_eq!(buffer[(0, 0)].fg, Color::Red);
        assert_eq!(buffer[(0, 1)].fg, Color::Green);
        assert_eq!(buffer[(0, 1)].bg, Color::Blue);
    }

    #[test]
    fn narrow_side_by_side_falls_back_without_panic() {
        let theme = Theme::default();
        let diff = Diff::new("a", "b").mode(DiffMode::SideBySide);
        for w in [0u16, 1, 2, 3, 4] {
            let _ = crate::testing::render(&diff, w, 2, &theme);
        }
    }
}
