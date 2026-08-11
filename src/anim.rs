//! Animation helpers: easing curves and a frame phase.
//!
//! `tuika` has no internal clock — the host owns time and passes a monotonically
//! increasing frame counter (yolop uses `App::busy_frame`) into animated
//! components. This module turns that counter into normalized progress and
//! shapes it with easing curves, the minimal analog of OpenTUI's Timeline API
//! without a scheduler or reconciler.

/// Normalized time in `0.0..=1.0`.
pub type Phase = f32;

/// Linear identity curve.
pub fn linear(t: Phase) -> Phase {
    t.clamp(0.0, 1.0)
}

/// Quadratic ease-in (slow start).
pub fn ease_in(t: Phase) -> Phase {
    let t = t.clamp(0.0, 1.0);
    t * t
}

/// Quadratic ease-out (slow end).
pub fn ease_out(t: Phase) -> Phase {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t)
}

/// Cubic ease-in-out (slow start and end).
pub fn ease_in_out(t: Phase) -> Phase {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        let f = -2.0 * t + 2.0;
        1.0 - f * f * f / 2.0
    }
}

/// Triangle wave in `0.0..=1.0..=0.0` over one period, for ping-pong motion.
///
/// `frame` is the host's counter, `period` the number of frames for a full
/// there-and-back cycle.
pub fn ping_pong(frame: u64, period: u64) -> Phase {
    if period == 0 {
        return 0.0;
    }
    let pos = frame % period;
    let half = period as f32 / 2.0;
    let up = pos as f32 / half;
    if up <= 1.0 { up } else { 2.0 - up }
}

/// Sawtooth wave in `0.0..1.0`, wrapping every `period` frames, for looping
/// motion like an indeterminate marquee.
pub fn sawtooth(frame: u64, period: u64) -> Phase {
    if period == 0 {
        return 0.0;
    }
    (frame % period) as f32 / period as f32
}

/// An easing curve: a pure function shaping normalized progress. The functions
/// in this module ([`linear`], [`ease_in`], [`ease_out`], [`ease_in_out`]) all
/// have this signature, and each [`Timeline`] keyframe stores one to shape
/// the segment leading into it.
pub type Easing = fn(Phase) -> Phase;

/// Linear interpolation from `a` to `b` by `t` in `0.0..=1.0` (clamped).
pub fn lerp(a: f32, b: f32, t: Phase) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// A retargetable eased ramp between two values, for state-driven motion.
///
/// Where a [`Timeline`] plays a fixed choreography from frame 0, a `Transition`
/// follows a *target that changes at runtime* — hover on/off, focus gained,
/// a panel expanding. Give it a new target with [`set_target`](Self::set_target)
/// and it eases there from wherever it currently is, including mid-flight:
/// retargeting starts a fresh segment from the current sampled value, so motion
/// never jumps.
///
/// Like everything animated in tuika it owns no clock: it is a pure function of
/// the host's frame counter, so sampling is deterministic and testable. The
/// usual shape for an animated *style* is a `Transition` driving a `0.0..=1.0`
/// phase that [`style::lerp_color`](crate::style::lerp_color) or a
/// [`style::Gradient`](crate::style::Gradient) maps onto colors:
///
/// ```
/// use tuika::anim::Transition;
/// use tuika::style::lerp_color;
/// use tuika::ui::Color;
///
/// let (normal, hot) = (Color::Rgb(0, 0, 0), Color::Rgb(200, 200, 200));
/// let mut hover = Transition::new(0.0, 12); // 12-frame ease
/// hover.set_target(1.0, 100);               // pointer entered on frame 100
/// let t = hover.sample(106);                // mid-flight...
/// let fg = lerp_color(normal, hot, t);      // ...blends the style
/// # let _ = fg;
/// assert!(t > 0.0 && t < 1.0);
/// assert_eq!(hover.sample(112), 1.0);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct Transition {
    from: f32,
    to: f32,
    /// Host frame at which the current segment began.
    start: u64,
    duration: u64,
    easing: Easing,
}

impl Transition {
    /// A transition at rest on `value`, taking `duration` frames per segment,
    /// eased with [`ease_in_out`]. A zero duration makes every retarget
    /// instantaneous.
    pub fn new(value: f32, duration: u64) -> Self {
        Self {
            from: value,
            to: value,
            start: 0,
            duration,
            easing: ease_in_out,
        }
    }

    /// Replace the easing curve (e.g. [`ease_out`] for snappier arrivals).
    pub fn easing(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }

    /// The value the transition is heading toward (or resting on).
    pub fn target(&self) -> f32 {
        self.to
    }

    /// Head toward `target`, starting a fresh eased segment at host `frame`
    /// from the current sampled value. Setting the target it already has is a
    /// no-op, so calling this every frame with the state-derived target is
    /// fine — motion in flight is not restarted.
    pub fn set_target(&mut self, target: f32, frame: u64) {
        if target == self.to {
            return;
        }
        self.from = self.sample(frame);
        self.to = target;
        self.start = frame;
    }

    /// Jump to `value` immediately, with no animation.
    pub fn snap(&mut self, value: f32) {
        self.from = value;
        self.to = value;
    }

    /// The eased value at host `frame`. Frames before the segment started
    /// return the segment's starting value.
    pub fn sample(&self, frame: u64) -> f32 {
        if self.duration == 0 || self.from == self.to {
            return self.to;
        }
        let elapsed = frame.saturating_sub(self.start);
        if elapsed >= self.duration {
            return self.to;
        }
        let p = elapsed as f32 / self.duration as f32;
        lerp(self.from, self.to, (self.easing)(p))
    }

    /// Whether motion has finished at host `frame` — the host can stop
    /// scheduling animation frames once every transition is settled.
    pub fn is_settled(&self, frame: u64) -> bool {
        self.sample(frame) == self.to
    }
}

/// What a [`Timeline`] does once the playhead passes its last keyframe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Repeat {
    /// Hold the final keyframe's value forever (the default).
    #[default]
    Once,
    /// Jump back to the start and play again, indefinitely.
    Loop,
    /// Play forward then backward, alternating each cycle.
    PingPong,
}

/// One stop in a [`Timeline`]: a value reached at frame offset `at`, with the
/// [`Easing`] that shapes the segment *leading into* it from the previous stop.
#[derive(Clone, Copy, Debug)]
struct Keyframe {
    at: u64,
    value: f32,
    easing: Easing,
}

/// A keyframed animation track: the scheduler-free analog of OpenTUI's Timeline.
///
/// Where the easing functions above shape a single 0→1 ramp, a `Timeline`
/// interpolates a scalar through a sequence of value stops over time, each
/// segment eased independently, with optional looping or ping-pong. It owns no
/// clock and spawns nothing: like every animated component in tuika it is a pure
/// function of the host's frame counter, so [`sample`](Self::sample) is
/// deterministic and testable, and several timelines compose by the host sampling
/// each (one per animated property) rather than a retained tween tree.
///
/// ```
/// use tuika::anim::{Repeat, Timeline, ease_out};
///
/// // Slide 0 → 100 over 30 frames (eased), then hold.
/// let slide = Timeline::new()
///     .keyframe(0, 0.0)
///     .ease(30, 100.0, ease_out);
/// assert_eq!(slide.sample(0), 0.0);
/// assert_eq!(slide.sample(30), 100.0);
/// assert_eq!(slide.sample(999), 100.0); // Repeat::Once holds the end
///
/// // A looping 0 → 1 → 0 pulse over 20 frames.
/// let pulse = Timeline::new()
///     .keyframe(0, 0.0)
///     .keyframe(10, 1.0)
///     .keyframe(20, 0.0)
///     .repeat(Repeat::Loop);
/// assert_eq!(pulse.sample(30), 1.0); // frame 30 wraps to local frame 10
/// ```
#[derive(Clone, Debug, Default)]
pub struct Timeline {
    /// Stops sorted ascending by `at`; a same-`at` insert replaces in place.
    keyframes: Vec<Keyframe>,
    repeat: Repeat,
}

impl Timeline {
    /// An empty timeline. Add stops with [`keyframe`](Self::keyframe) /
    /// [`ease`](Self::ease); an empty timeline samples to `0.0`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a stop of `value` at frame offset `at`, reached **linearly** from the
    /// previous stop. Order-independent: stops are kept sorted by `at`, and a
    /// second stop at the same `at` replaces the first.
    pub fn keyframe(self, at: u64, value: f32) -> Self {
        self.ease(at, value, linear)
    }

    /// Like [`keyframe`](Self::keyframe) but shaping the segment *into* this stop
    /// with `easing` (e.g. [`ease_out`]).
    pub fn ease(mut self, at: u64, value: f32, easing: Easing) -> Self {
        let kf = Keyframe { at, value, easing };
        match self.keyframes.binary_search_by_key(&at, |k| k.at) {
            Ok(i) => self.keyframes[i] = kf,
            Err(i) => self.keyframes.insert(i, kf),
        }
        self
    }

    /// Set the end behavior (default [`Repeat::Once`]).
    pub fn repeat(mut self, repeat: Repeat) -> Self {
        self.repeat = repeat;
        self
    }

    /// The frame offset of the last stop — the length of one play-through. Zero
    /// for an empty or single-stop timeline.
    pub fn duration(&self) -> u64 {
        self.keyframes.last().map(|k| k.at).unwrap_or(0)
    }

    /// Whether the timeline has finished — only ever true for [`Repeat::Once`]
    /// once `frame` reaches [`duration`](Self::duration). Looping timelines never
    /// complete.
    pub fn is_complete(&self, frame: u64) -> bool {
        matches!(self.repeat, Repeat::Once) && frame >= self.duration()
    }

    /// Map the monotonic host `frame` onto a local time in `0..=duration`,
    /// according to the repeat mode.
    fn local_time(&self, frame: u64) -> u64 {
        let duration = self.duration();
        if duration == 0 {
            return 0;
        }
        match self.repeat {
            Repeat::Once => frame.min(duration),
            Repeat::Loop => frame % duration,
            Repeat::PingPong => {
                let pos = frame % duration;
                // Even cycles play forward, odd cycles backward.
                if (frame / duration).is_multiple_of(2) {
                    pos
                } else {
                    duration - pos
                }
            }
        }
    }

    /// The interpolated value at host `frame`. Empty → `0.0`; before the first
    /// stop or after the last (under [`Repeat::Once`]) the value is clamped to
    /// that stop.
    pub fn sample(&self, frame: u64) -> f32 {
        match self.keyframes.as_slice() {
            [] => 0.0,
            [only] => only.value,
            keyframes => {
                let t = self.local_time(frame);
                // Find the segment [lo, hi] straddling t. `t <= duration` always.
                let hi = keyframes
                    .iter()
                    .position(|k| k.at >= t)
                    .unwrap_or(keyframes.len() - 1)
                    .max(1);
                let (lo, hi) = (&keyframes[hi - 1], &keyframes[hi]);
                let span = hi.at - lo.at;
                if span == 0 {
                    return hi.value;
                }
                let p = (t - lo.at) as f32 / span as f32;
                let eased = (hi.easing)(p);
                lo.value + (hi.value - lo.value) * eased
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_endpoints_and_midpoints() {
        for f in [linear, ease_in, ease_out, ease_in_out] {
            assert!((f(0.0) - 0.0).abs() < 1e-6);
            assert!((f(1.0) - 1.0).abs() < 1e-6);
        }
        // Cubic ease-in-out is symmetric about 0.5.
        assert!((ease_in_out(0.5) - 0.5).abs() < 1e-6);
        // Clamps out-of-range input.
        assert_eq!(linear(2.0), 1.0);
        assert_eq!(ease_out(-1.0), 0.0);
    }

    #[test]
    fn ping_pong_and_sawtooth_shapes() {
        assert!((ping_pong(0, 60) - 0.0).abs() < 1e-6);
        assert!((ping_pong(30, 60) - 1.0).abs() < 1e-6); // peak at half period
        assert!((ping_pong(60, 60) - 0.0).abs() < 1e-6); // back to start
        assert!((sawtooth(0, 10) - 0.0).abs() < 1e-6);
        assert!((sawtooth(5, 10) - 0.5).abs() < 1e-6);
        assert!((sawtooth(10, 10) - 0.0).abs() < 1e-6); // wraps
    }

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-4, "{a} != {b}");
    }

    #[test]
    fn timeline_empty_and_single_stop() {
        approx(Timeline::new().sample(0), 0.0);
        approx(Timeline::new().sample(100), 0.0);
        let one = Timeline::new().keyframe(5, 42.0);
        approx(one.sample(0), 42.0);
        approx(one.sample(999), 42.0);
        assert_eq!(one.duration(), 5);
    }

    #[test]
    fn timeline_linear_interpolates_and_holds() {
        let t = Timeline::new().keyframe(0, 0.0).keyframe(10, 100.0);
        approx(t.sample(0), 0.0);
        approx(t.sample(5), 50.0);
        approx(t.sample(10), 100.0);
        // Repeat::Once holds the final value past the end.
        approx(t.sample(50), 100.0);
        assert!(t.is_complete(10));
        assert!(!t.is_complete(9));
    }

    #[test]
    fn timeline_insertion_order_is_normalized() {
        // Stops added out of order still form an ascending, correctly-segmented
        // timeline.
        let a = Timeline::new()
            .keyframe(20, 2.0)
            .keyframe(0, 0.0)
            .keyframe(10, 1.0);
        approx(a.sample(5), 0.5);
        approx(a.sample(15), 1.5);
        // A same-`at` insert replaces in place rather than duplicating.
        let b = Timeline::new().keyframe(10, 1.0).keyframe(10, 9.0);
        approx(b.sample(10), 9.0);
        assert_eq!(b.duration(), 10);
    }

    #[test]
    fn timeline_easing_shapes_the_segment() {
        let eased = Timeline::new().keyframe(0, 0.0).ease(10, 1.0, ease_in);
        // ease_in is quadratic: midpoint sits below the linear 0.5.
        assert!(eased.sample(5) < 0.5);
        approx(eased.sample(0), 0.0);
        approx(eased.sample(10), 1.0);
    }

    #[test]
    fn transition_eases_toward_a_new_target() {
        let mut t = Transition::new(0.0, 10).easing(linear);
        approx(t.sample(0), 0.0);
        assert!(t.is_settled(0));

        t.set_target(1.0, 100);
        approx(t.sample(100), 0.0);
        approx(t.sample(105), 0.5);
        approx(t.sample(110), 1.0);
        approx(t.sample(999), 1.0); // holds after arrival
        assert!(!t.is_settled(105));
        assert!(t.is_settled(110));
    }

    #[test]
    fn transition_retargets_from_mid_flight_value() {
        let mut t = Transition::new(0.0, 10).easing(linear);
        t.set_target(1.0, 0);
        // Reverse halfway there: the new segment starts at 0.5, not at 1.0.
        t.set_target(0.0, 5);
        approx(t.sample(5), 0.5);
        approx(t.sample(10), 0.25);
        approx(t.sample(15), 0.0);
    }

    #[test]
    fn transition_same_target_does_not_restart_motion() {
        let mut t = Transition::new(0.0, 10).easing(linear);
        t.set_target(1.0, 0);
        // Re-asserting the target every frame must not reset the segment.
        t.set_target(1.0, 4);
        approx(t.sample(5), 0.5);
    }

    #[test]
    fn transition_zero_duration_and_snap_are_instant() {
        let mut t = Transition::new(0.0, 0);
        t.set_target(1.0, 42);
        approx(t.sample(42), 1.0);

        let mut s = Transition::new(0.0, 30);
        s.set_target(1.0, 0);
        s.snap(0.25);
        approx(s.sample(1), 0.25);
        assert!(s.is_settled(1));
    }

    #[test]
    fn lerp_clamps_and_interpolates() {
        approx(lerp(0.0, 10.0, 0.5), 5.0);
        approx(lerp(0.0, 10.0, -1.0), 0.0);
        approx(lerp(0.0, 10.0, 2.0), 10.0);
        approx(lerp(10.0, 0.0, 0.25), 7.5);
    }

    #[test]
    fn timeline_loop_and_ping_pong() {
        let looping = Timeline::new()
            .keyframe(0, 0.0)
            .keyframe(10, 1.0)
            .keyframe(20, 0.0)
            .repeat(Repeat::Loop);
        approx(looping.sample(10), 1.0);
        approx(looping.sample(30), 1.0); // 30 % 20 == 10
        assert!(!looping.is_complete(1_000_000));

        let pp = Timeline::new()
            .keyframe(0, 0.0)
            .keyframe(10, 1.0)
            .repeat(Repeat::PingPong);
        approx(pp.sample(0), 0.0);
        approx(pp.sample(10), 1.0); // end of forward cycle
        approx(pp.sample(15), 0.5); // backward cycle, halfway home
        approx(pp.sample(20), 0.0); // back to start
    }
}
