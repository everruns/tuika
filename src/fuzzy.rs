//! The one fuzzy ranking every searchable surface shares, so a completion popup
//! and the keymap's command search order the same query the same way.

/// Rank `candidate` against `query`, case-insensitively. Lower is better:
/// `0` for an exact (or empty) query, `1` for a prefix, `10 + position` for a
/// substring, and `100 + gaps` for an in-order subsequence. `None` when the
/// query's characters do not all appear in order.
pub(crate) fn score(candidate: &str, query: &str) -> Option<usize> {
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
