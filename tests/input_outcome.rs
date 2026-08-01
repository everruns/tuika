//! One interaction contract across host-owned component states.

use tuika::prelude::*;

fn key(code: KeyCode) -> Event {
    Event::Key(Key::new(code))
}

#[test]
fn constructors_and_defaults_start_from_the_same_state() {
    assert_eq!(
        SelectState::default().selected(),
        SelectState::new().selected()
    );
    assert_eq!(SelectState::default().selected(), Some(0));
    assert_eq!(SelectState::unselected().selected(), None);

    assert_eq!(
        ScrollState::default().is_stuck_to_bottom(),
        ScrollState::new().is_stuck_to_bottom()
    );
    assert!(ScrollState::default().is_stuck_to_bottom());

    assert_eq!(
        MultiSelectState::default().cursor().selected(),
        MultiSelectState::new().cursor().selected()
    );

    assert_eq!(
        TabSelectState::default().selected(),
        TabSelectState::new().selected()
    );
}

#[test]
fn interactive_components_share_one_outcome_vocabulary() {
    let mut select = SelectState::default();
    assert_eq!(select.handle(&key(KeyCode::Down), 3), InputOutcome::Changed);
    assert_eq!(
        select.handle(&key(KeyCode::Enter), 3),
        InputOutcome::Submitted
    );
    assert_eq!(select.selected(), Some(1));

    let mut tabs = TabsState::default();
    assert_eq!(tabs.handle(&key(KeyCode::Right), 3), InputOutcome::Changed);
    let mut single_tab = TabsState::default();
    assert_eq!(
        single_tab.handle(&key(KeyCode::Right), 1),
        InputOutcome::Consumed
    );

    let mut tab_select = TabSelectState::default();
    assert_eq!(
        tab_select.handle(&key(KeyCode::Enter), 3),
        InputOutcome::Submitted
    );

    let mut slider = SliderState::default();
    assert_eq!(slider.handle(&key(KeyCode::Right)), InputOutcome::Changed);
    slider.set_value(slider.max());
    assert_eq!(slider.handle(&key(KeyCode::Right)), InputOutcome::Consumed);

    let mut form = FormState::default();
    assert_eq!(form.handle(&key(KeyCode::Tab), 2), InputOutcome::Changed);
    assert_eq!(
        form.handle(&key(KeyCode::Enter), 2),
        InputOutcome::Submitted
    );
    form.focus(usize::MAX);
    assert_eq!(form.handle(&key(KeyCode::Tab), 2), InputOutcome::Changed);
    assert_eq!(form.focused(), 0);

    select.select(Some(usize::MAX));
    assert_eq!(select.handle(&key(KeyCode::Down), 3), InputOutcome::Changed);
    assert_eq!(select.selected(), Some(0));

    let mut scroll = ScrollState::default();
    scroll.clamp(100, 10);
    assert_eq!(
        scroll.handle(&key(KeyCode::PageUp), 100, 10),
        InputOutcome::Changed
    );

    let mut input = TextInputState::default();
    assert_eq!(
        input.handle(&key(KeyCode::Char('a'))),
        InputOutcome::Changed
    );
    assert_eq!(input.handle(&key(KeyCode::Enter)), InputOutcome::Submitted);
    let mut empty_input = TextInputState::default();
    assert_eq!(
        empty_input.handle(&key(KeyCode::Left)),
        InputOutcome::Consumed
    );
    assert_eq!(
        empty_input.handle(&Event::Paste(String::new())),
        InputOutcome::Consumed
    );
    assert_eq!(
        empty_input.handle(&Event::Resize {
            width: 80,
            height: 24,
        }),
        InputOutcome::Ignored
    );
}

#[test]
fn outcomes_map_uniformly_to_event_propagation() {
    let root_path: tuika::InputOutcome = InputOutcome::Changed;
    assert_eq!(root_path, InputOutcome::Changed);
    assert_eq!(InputOutcome::Ignored.flow(), EventFlow::Ignored);
    for outcome in [
        InputOutcome::Consumed,
        InputOutcome::Changed,
        InputOutcome::Submitted,
        InputOutcome::Cancelled,
    ] {
        assert_eq!(outcome.flow(), EventFlow::Consumed);
        assert!(outcome.consumed());
    }
}
