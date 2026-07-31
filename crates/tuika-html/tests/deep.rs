//! Pathological nesting must degrade, never crash the host.
//!
//! A stack overflow aborts the process rather than failing an assertion, so this
//! lives in its own test binary: if the guard regresses, the failure is loud and
//! attributable instead of taking an unrelated test down with it.

use tuika::Theme;
use tuika::style::StyleSheet;
use tuika_html::{Limits, to_lines, to_lines_with_limits};

/// Deep enough to overflow the stack in html5ever's tree building — and well
/// inside the 256 KiB input bound, which is why the size bound alone is not
/// enough of a guard.
fn deeply_nested(n: usize) -> String {
    format!("<pre>{}x{}</pre>", "<b>".repeat(n), "</b>".repeat(n))
}

#[test]
fn deeply_nested_markup_is_refused_rather_than_parsed() {
    let theme = Theme::default();
    let sheet = StyleSheet::from_theme(&theme);

    let hostile = deeply_nested(20_000);
    assert!(
        hostile.len() < Limits::default().max_input_bytes,
        "the point of this test is input the size bound accepts"
    );
    assert!(to_lines(&hostile, 40, &theme, &sheet).is_empty());
    assert!(to_lines_with_limits(&hostile, 40, &theme, &sheet, Limits::default()).is_none());
}

#[test]
fn nesting_within_the_bound_still_renders() {
    let theme = Theme::default();
    let sheet = StyleSheet::from_theme(&theme);
    let ok = deeply_nested(20);
    let lines = to_lines(&ok, 40, &theme, &sheet);
    assert!(
        lines
            .iter()
            .any(|l| l.spans.iter().any(|s| s.content.contains('x'))),
        "content at a sane depth must survive: {lines:?}"
    );
}

#[test]
fn a_wide_run_of_void_elements_is_not_deep() {
    // `<br>` never closes, so a naive open/close counter would read 5,000 of
    // them as depth 5,000 and refuse an ordinary document.
    let theme = Theme::default();
    let sheet = StyleSheet::from_theme(&theme);
    let wide = format!("<p>{}</p>", "line<br>".repeat(5_000));
    assert!(!to_lines(&wide, 40, &theme, &sheet).is_empty());
}
