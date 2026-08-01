use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tuika::mouse::SelectionState;
use tuika::prelude::{
    Clock, Mouse, MouseButton, MouseKind, Runner, RunnerConfig, SystemClock, Text,
};
use tuika::testing::render;
use tuika::ui::Rect;

#[derive(Clone)]
struct ManualClock(Arc<Mutex<Instant>>);

impl ManualClock {
    fn new(now: Instant) -> Self {
        Self(Arc::new(Mutex::new(now)))
    }

    fn advance(&self, duration: Duration) {
        let mut now = self.0.lock().expect("manual clock lock");
        *now += duration;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.0.lock().expect("manual clock lock")
    }
}

fn click(state: &mut SelectionState, clock: &dyn Clock, column: u16) {
    let down = Mouse::at(MouseKind::Down(MouseButton::Left), column, 0);
    let up = Mouse::at(MouseKind::Up(MouseButton::Left), column, 0);
    state.handle_with_clock(&down, clock);
    state.handle_with_clock(&up, clock);
}

#[test]
fn selection_double_click_uses_the_injected_monotonic_clock() {
    let clock = ManualClock::new(Instant::now());
    let mut state = SelectionState::new();
    click(&mut state, &clock, 1);
    clock.advance(Duration::from_millis(100));
    click(&mut state, &clock, 1);

    let theme = tuika::Theme::default();
    let buffer = render(&Text::raw("hello world"), 11, 1, &theme);
    assert!(state.resolve(&buffer, Rect::new(0, 0, 11, 1)));
    assert_eq!(state.range().expect("word selection").end, (4, 0));

    state.clear();
    click(&mut state, &clock, 7);
    clock.advance(Duration::from_millis(501));
    click(&mut state, &clock, 7);
    assert!(!state.resolve(&buffer, Rect::new(0, 0, 11, 1)));
}

#[test]
fn system_and_manual_clocks_are_accepted_by_the_runner() {
    let _system = Runner::with_clock(RunnerConfig::default(), SystemClock);
    let clock = ManualClock::new(Instant::now());
    let _manual = Runner::with_clock(RunnerConfig::default(), clock.clone());
    clock.advance(Duration::from_secs(1));
}
