//! Generic completion and command-palette state plus its rendered popup.
//!
//! [`CompletionState`] owns only query/filter/selection state. The editor and
//! the meaning of an accepted item remain with the host: use a
//! [`Trigger`](crate::components::Trigger) to obtain a token query, synchronize
//! it here, then replace the token with [`CompletionItem::replacement`] when
//! handling [`InputOutcome::Submitted`](crate::InputOutcome::Submitted).

use crate::geometry::Rect;
use crate::style::Style;

use crate::event::{Event, InputOutcome};
use crate::geometry::{Padding, Size};
use crate::surface::Surface;
use crate::view::{RenderCtx, View};
use crate::width::str_cols;

use super::{Boxed, Scrollbar, SelectNavigation, SelectState, VirtualWindow};

/// One candidate in a [`CompletionPalette`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    label: String,
    detail: Option<String>,
    replacement: String,
    keywords: Vec<String>,
}

impl CompletionItem {
    /// Create an item whose inserted replacement is the displayed `label`.
    pub fn new(label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            replacement: label.clone(),
            label,
            detail: None,
            keywords: Vec::new(),
        }
    }

    /// Add secondary text shown after the label and searched at lower priority.
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Set the text inserted when this item is accepted.
    pub fn replacement(mut self, replacement: impl Into<String>) -> Self {
        self.replacement = replacement.into();
        self
    }

    /// Add an alternate search term that is not rendered.
    pub fn keyword(mut self, keyword: impl Into<String>) -> Self {
        self.keywords.push(keyword.into());
        self
    }

    /// Display label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Optional secondary description.
    pub fn detail_text(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// Text the host should insert on acceptance.
    pub fn replacement_text(&self) -> &str {
        &self.replacement
    }

    fn score(&self, query: &str) -> Option<usize> {
        let mut best = fuzzy_score(&self.label, query);
        if let Some(detail) = &self.detail
            && let Some(score) = fuzzy_score(detail, query)
        {
            let score = score.saturating_add(200);
            best = Some(best.map_or(score, |current| current.min(score)));
        }
        for keyword in &self.keywords {
            if let Some(score) = fuzzy_score(keyword, query) {
                let score = score.saturating_add(100);
                best = Some(best.map_or(score, |current| current.min(score)));
            }
        }
        best
    }
}

/// Host-owned filtered selection for a [`CompletionPalette`].
#[derive(Clone, Debug, Default)]
pub struct CompletionState {
    query: String,
    matches: Vec<usize>,
    match_keys: Vec<String>,
    selection: SelectState,
}

impl CompletionState {
    /// An empty state. Call [`sync`](Self::sync) with the candidate collection
    /// before rendering or handling input.
    pub fn new() -> Self {
        Self::default()
    }

    /// Recompute ranked matches for `query`.
    ///
    /// A changed query resets the cursor to the first result. Refreshing the
    /// same query after the candidate collection changes preserves the selected
    /// item when it still exists.
    pub fn sync(&mut self, query: impl Into<String>, items: &[CompletionItem]) {
        let query = query.into();
        let previous_key = self
            .selection
            .selected()
            .and_then(|selected| self.match_keys.get(selected))
            .cloned();
        let query_changed = self.query != query;

        let mut ranked: Vec<(usize, usize)> = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| item.score(&query).map(|score| (index, score)))
            .collect();
        ranked.sort_by(|(left_index, left_score), (right_index, right_score)| {
            left_score
                .cmp(right_score)
                .then_with(|| left_index.cmp(right_index))
        });
        self.matches = ranked.into_iter().map(|(index, _)| index).collect();
        self.match_keys = self
            .matches
            .iter()
            .filter_map(|index| items.get(*index))
            .map(|item| item.replacement.clone())
            .collect();
        self.query = query;

        if self.matches.is_empty() {
            self.selection.select(None);
        } else if query_changed {
            self.selection.select(Some(0));
        } else if let Some(position) = previous_key.and_then(|key| {
            self.match_keys
                .iter()
                .position(|candidate| *candidate == key)
        }) {
            self.selection.select(Some(position));
        } else {
            self.selection.clamp(self.matches.len());
            if self.selection.selected().is_none() {
                self.selection.select(Some(0));
            }
        }
    }

    /// Current query text.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Number of matching candidates.
    pub fn len(&self) -> usize {
        self.matches.len()
    }

    /// Whether the query has no matches.
    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    /// Ranked indices into the collection last passed to [`sync`](Self::sync).
    pub fn matched_indices(&self) -> &[usize] {
        &self.matches
    }

    /// Selection state within the ranked match list.
    pub fn selection(&self) -> &SelectState {
        &self.selection
    }

    /// Handle arrow navigation, Enter acceptance, and Esc cancellation.
    pub fn handle(&mut self, event: &Event) -> InputOutcome {
        self.handle_with(event, SelectNavigation::default())
    }

    /// Handle input with an explicit selection-navigation policy.
    pub fn handle_with(&mut self, event: &Event, navigation: SelectNavigation) -> InputOutcome {
        self.selection
            .handle_with(event, self.matches.len(), navigation)
    }

    /// Selected original collection index.
    pub fn selected_index(&self) -> Option<usize> {
        self.selection
            .selected()
            .and_then(|selected| self.matches.get(selected))
            .copied()
    }

    /// Selected candidate from the collection last synchronized with this
    /// state. Returns `None` if the caller passes a different/shorter slice.
    pub fn selected<'a>(&self, items: &'a [CompletionItem]) -> Option<&'a CompletionItem> {
        self.selected_index().and_then(|index| items.get(index))
    }
}

/// Framed, filter-ranked completion popup derived from [`CompletionState`].
///
/// This view snapshots the matched rows for one frame; persistent query and
/// cursor state remain in the host-owned state.
///
/// ![completion palette demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/completion_palette.gif)
///
/// Run it with `cargo run --example demo -- completion_palette`.
///
/// ```
/// use tuika::prelude::*;
///
/// let items = vec![
///     CompletionItem::new("model").detail("Choose a model"),
///     CompletionItem::new("status").detail("Show session status"),
/// ];
/// let mut state = CompletionState::new();
/// state.sync("mod", &items);
/// let palette = CompletionPalette::new(&items, &state).title("Commands");
/// let rendered = tuika::testing::render(&palette, 40, 5, &Theme::default());
/// assert!(tuika::testing::grid(&rendered).contains("model"));
/// ```
pub struct CompletionPalette {
    title: String,
    query: String,
    rows: Vec<CompletionItem>,
    selected: Option<usize>,
    viewport: u16,
    show_query: bool,
    empty_message: String,
}

impl CompletionPalette {
    /// Snapshot the matches in `state` from `items`.
    pub fn new(items: &[CompletionItem], state: &CompletionState) -> Self {
        Self {
            title: " Completions ".into(),
            query: state.query.clone(),
            rows: state
                .matches
                .iter()
                .filter_map(|index| items.get(*index).cloned())
                .collect(),
            selected: state.selection.selected(),
            viewport: 8,
            show_query: false,
            empty_message: "No matches".into(),
        }
    }

    /// Set the frame title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = format!(" {} ", title.into());
        self
    }

    /// Cap the number of visible candidate rows.
    pub fn viewport(mut self, rows: u16) -> Self {
        self.viewport = rows.max(1);
        self
    }

    /// Show the current query as a first row, useful for a standalone command
    /// palette. Completion popups anchored to an editor normally leave it off.
    pub fn show_query(mut self, show: bool) -> Self {
        self.show_query = show;
        self
    }

    /// Replace the empty-result message.
    pub fn empty_message(mut self, message: impl Into<String>) -> Self {
        self.empty_message = message.into();
        self
    }

    fn panel(&self) -> Boxed<CompletionBody> {
        Boxed::new(CompletionBody {
            query: self.query.clone(),
            rows: self.rows.clone(),
            selected: self.selected,
            viewport: self.viewport,
            show_query: self.show_query,
            empty_message: self.empty_message.clone(),
        })
        .title(self.title.clone())
        .padding(Padding::symmetric(1, 0))
        .background(Style::default())
    }
}

impl View for CompletionPalette {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        self.panel().measure(available, ctx)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        self.panel().render(area, surface, ctx);
    }
}

struct CompletionBody {
    query: String,
    rows: Vec<CompletionItem>,
    selected: Option<usize>,
    viewport: u16,
    show_query: bool,
    empty_message: String,
}

impl CompletionBody {
    fn window(&self) -> VirtualWindow {
        VirtualWindow::around(self.rows.len(), usize::from(self.viewport), self.selected)
    }

    fn row_width(item: &CompletionItem) -> u16 {
        let detail = item
            .detail
            .as_deref()
            .map(|text| str_cols(text).saturating_add(2))
            .unwrap_or(0);
        str_cols(&item.label)
            .saturating_add(detail)
            .saturating_add(2)
    }
}

impl View for CompletionBody {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        let query_width = if self.show_query {
            str_cols(&self.query).saturating_add(2)
        } else {
            0
        };
        let rows_width = self
            .rows
            .iter()
            .map(Self::row_width)
            .max()
            .unwrap_or_else(|| str_cols(&self.empty_message));
        let rows = if self.rows.is_empty() {
            1
        } else {
            self.window().len().min(u16::MAX as usize) as u16
        };
        Size::new(
            query_width.max(rows_width).min(available.width),
            rows.saturating_add(u16::from(self.show_query))
                .min(available.height),
        )
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.is_empty() {
            return;
        }
        let mut y = area.y;
        if self.show_query {
            surface.set_string(area.x, y, "› ", ctx.theme.accent_style());
            surface.set_string(
                area.x.saturating_add(2),
                y,
                &self.query,
                ctx.theme.text_style(),
            );
            y = y.saturating_add(1);
        }
        if y >= area.bottom() {
            return;
        }
        if self.rows.is_empty() {
            surface.set_string(area.x, y, &self.empty_message, ctx.theme.muted_style());
            return;
        }

        let window = self.window();
        let list_area = Rect::new(area.x, y, area.width, area.bottom().saturating_sub(y));
        let overflow = window.overflows();
        let row_width = if overflow {
            list_area.width.saturating_sub(1)
        } else {
            list_area.width
        };
        for (visible, index) in window.range().enumerate() {
            let row_y = list_area.y.saturating_add(visible as u16);
            if row_y >= list_area.bottom() {
                break;
            }
            let item = &self.rows[index];
            let selected = self.selected == Some(index);
            let base = if selected {
                ctx.theme.selection_style()
            } else {
                ctx.theme.text_style()
            };
            if selected {
                surface
                    .child(Rect::new(list_area.x, row_y, row_width, 1))
                    .fill(base);
            }
            let caret = if selected { "› " } else { "  " };
            let mut x = surface.set_string(list_area.x, row_y, caret, base);
            x = surface.set_string(x, row_y, &item.label, base);
            if let Some(detail) = &item.detail
                && x < list_area.x.saturating_add(row_width)
            {
                let detail_style = if selected {
                    base
                } else {
                    ctx.theme.muted_style()
                };
                x = surface.set_string(x, row_y, "  ", detail_style);
                let _ = surface.set_string(x, row_y, detail, detail_style);
            }
        }
        if overflow && row_width < list_area.width {
            Scrollbar::vertical(window).render(
                Rect::new(
                    list_area.right() - 1,
                    list_area.y,
                    1,
                    list_area.height.min(window.len() as u16),
                ),
                surface,
                ctx,
            );
        }
    }
}

fn fuzzy_score(candidate: &str, query: &str) -> Option<usize> {
    let candidate = candidate.to_lowercase();
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    if candidate == query {
        return Some(0);
    }
    if candidate.starts_with(&query) {
        return Some(1);
    }
    if let Some(position) = candidate.find(&query) {
        return Some(10usize.saturating_add(position));
    }

    let mut chars = candidate.chars().enumerate();
    let mut last = 0usize;
    let mut gap = 0usize;
    for needle in query.chars() {
        let (position, _) = chars.find(|(_, character)| *character == needle)?;
        gap = gap.saturating_add(position.saturating_sub(last));
        last = position.saturating_add(1);
    }
    Some(100usize.saturating_add(gap))
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::*;
    use crate::Theme;
    use crate::event::{Key, KeyCode};
    use crate::testing::{grid, render};
    use crate::tests::support::rainbow_theme;

    fn items() -> Vec<CompletionItem> {
        vec![
            CompletionItem::new("status").detail("show session configuration"),
            CompletionItem::new("approvals")
                .detail("change approval policy")
                .keyword("permissions"),
            CompletionItem::new("model").replacement("model gpt-5"),
        ]
    }

    #[test]
    fn sync_ranks_prefix_substring_keyword_and_fuzzy_matches() {
        let items = items();
        let mut state = CompletionState::new();
        state.sync("sta", &items);
        assert_eq!(state.matched_indices(), &[0]);
        state.sync("permissions", &items);
        assert_eq!(state.matched_indices(), &[1]);
        state.sync("aprv", &items);
        assert_eq!(state.matched_indices(), &[1]);
        state.sync("", &items);
        assert_eq!(state.matched_indices(), &[0, 1, 2]);
    }

    #[test]
    fn changed_query_resets_selection_and_same_query_preserves_item() {
        let mut items = items();
        let mut state = CompletionState::new();
        state.sync("", &items);
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Down))),
            InputOutcome::Changed
        );
        assert_eq!(
            state.selected(&items).map(CompletionItem::label),
            Some("approvals")
        );

        items.insert(0, CompletionItem::new("about"));
        state.sync("", &items);
        assert_eq!(
            state.selected(&items).map(CompletionItem::label),
            Some("approvals")
        );

        state.sync("mod", &items);
        assert_eq!(
            state.selected(&items).map(CompletionItem::label),
            Some("model")
        );
    }

    #[test]
    fn navigation_submits_and_empty_results_ignore_input() {
        let items = items();
        let mut state = CompletionState::new();
        state.sync("", &items);
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Enter))),
            InputOutcome::Submitted
        );
        assert_eq!(state.selected(&items).unwrap().replacement_text(), "status");

        state.sync("missing", &items);
        assert_eq!(
            state.handle(&Event::Key(Key::new(KeyCode::Down))),
            InputOutcome::Ignored
        );
        assert!(state.selected(&items).is_none());
    }

    #[test]
    fn palette_renders_query_rows_and_empty_state() {
        let items = items();
        let mut state = CompletionState::new();
        state.sync("app", &items);
        let palette = CompletionPalette::new(&items, &state)
            .title("Commands")
            .show_query(true);
        let rendered = grid(&render(&palette, 42, 5, &Theme::default()));
        assert!(rendered.contains("Commands"));
        assert!(rendered.contains("› app"));
        assert!(rendered.contains("approvals"));

        state.sync("none", &items);
        let empty = CompletionPalette::new(&items, &state).empty_message("Nothing found");
        assert!(grid(&render(&empty, 24, 3, &Theme::default())).contains("Nothing found"));
    }

    #[test]
    fn palette_handles_tiny_sizes() {
        let items = items();
        let mut state = CompletionState::new();
        state.sync("", &items);
        for width in 0..5 {
            for height in 0..4 {
                let _ = render(
                    &CompletionPalette::new(&items, &state),
                    width,
                    height,
                    &Theme::default(),
                );
            }
        }
    }

    #[test]
    fn palette_uses_theme_slots_for_chrome_selection_and_details() {
        let items = items();
        let mut state = CompletionState::new();
        state.sync("", &items);
        let theme = rainbow_theme();
        let buffer = render(&CompletionPalette::new(&items, &state), 42, 5, &theme);

        assert_eq!(buffer[(0, 0)].fg, theme.border);
        assert_eq!(buffer[(2, 1)].fg, theme.selection_fg);
        assert_eq!(buffer[(2, 1)].bg, theme.selection_bg);
        assert_eq!(buffer[(4, 2)].fg, theme.text);
        assert_eq!(buffer[(15, 2)].fg, theme.muted);
    }

    #[test]
    fn fuzzy_scoring_orders_exact_before_prefix_before_subsequence() {
        assert_eq!(fuzzy_score("model", "model"), Some(0));
        assert_eq!(fuzzy_score("models", "model"), Some(1));
        assert!(fuzzy_score("my model", "model").unwrap() >= 10);
        assert!(fuzzy_score("multi option dialog", "mod").unwrap() >= 100);
        assert_eq!(fuzzy_score("status", "xyz"), None);
    }

    #[test]
    fn score_order_is_total_and_stable() {
        let mut ranked = [(2usize, 1usize), (0, 1), (1, 0)];
        ranked.sort_by(|(left_index, left_score), (right_index, right_score)| {
            let ordering: Ordering = left_score
                .cmp(right_score)
                .then_with(|| left_index.cmp(right_index));
            ordering
        });
        assert_eq!(ranked, [(1, 0), (0, 1), (2, 1)]);
    }
}
