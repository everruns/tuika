//! Small observable values for data-driven views.
//!
//! `Live` is not a reconciler and does not spawn work. Producers own their
//! threads or async tasks; updating a value requests a redraw from a connected
//! runner ([`Runner`](crate::Runner) or
//! [`AsyncRunner`](crate::runner::AsyncRunner)), and views read the latest value
//! each frame.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::task::Waker;
#[cfg(feature = "async")]
use std::task::{Context, Poll};

use crate::geometry::Rect;

use crate::{Element, RenderCtx, Size, Surface, View};

#[derive(Default)]
struct RedrawSignal {
    requested: AtomicBool,
    // The synchronous runner discovers a request by checking the flag once per
    // loop iteration; an async runner has no such iteration to check from and
    // must be woken. Registering the waiting task's waker here keeps that
    // wakeup executor-neutral — the async runner is Tokio-shaped, but nothing
    // in this handle needs to be.
    waker: Mutex<Option<Waker>>,
}

/// A thread-safe request for the terminal runner to redraw.
#[derive(Clone, Default)]
pub struct RedrawHandle {
    signal: Arc<RedrawSignal>,
}

impl RedrawHandle {
    /// Mark the connected runner dirty.
    pub fn request(&self) {
        self.signal.requested.store(true, Ordering::Release);
        let waker = self
            .signal
            .waker
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    pub(crate) fn take(&self) -> bool {
        self.signal.requested.swap(false, Ordering::AcqRel)
    }

    /// Poll for a pending request, registering `cx`'s waker when there is none.
    ///
    /// Cancelling a wait leaves at most a stale waker behind, which costs one
    /// spurious wakeup and never a lost one: the flag is re-checked after
    /// registration, so a request racing with it is still observed.
    #[cfg(feature = "async")]
    pub(crate) fn poll_take(&self, cx: &Context<'_>) -> Poll<()> {
        if self.take() {
            return Poll::Ready(());
        }
        {
            let mut waker = self
                .signal
                .waker
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            *waker = Some(cx.waker().clone());
        }
        if self.take() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }

    /// Wait until a redraw is requested, consuming the request.
    #[cfg(feature = "async")]
    pub(crate) async fn requested(&self) {
        std::future::poll_fn(|cx| self.poll_take(cx)).await;
    }
}

/// Shared application data whose updates invalidate a connected runner.
pub struct Live<T> {
    value: Arc<RwLock<T>>,
    redraw: Option<RedrawHandle>,
}

impl<T> Clone for Live<T> {
    fn clone(&self) -> Self {
        Self {
            value: Arc::clone(&self.value),
            redraw: self.redraw.clone(),
        }
    }
}

impl<T> Live<T> {
    /// Create a value that is read live but does not notify a runner.
    pub fn new(value: T) -> Self {
        Self {
            value: Arc::new(RwLock::new(value)),
            redraw: None,
        }
    }

    /// Create a value whose mutations request redraws through `redraw`.
    pub fn with_redraw(value: T, redraw: RedrawHandle) -> Self {
        Self {
            value: Arc::new(RwLock::new(value)),
            redraw: Some(redraw),
        }
    }

    /// Read the current value without exposing the lock guard.
    pub fn with<R>(&self, read: impl FnOnce(&T) -> R) -> R {
        let value = self.value.read().unwrap_or_else(|error| error.into_inner());
        read(&value)
    }

    /// Mutate the value and request a redraw after releasing the write lock.
    pub fn update<R>(&self, update: impl FnOnce(&mut T) -> R) -> R {
        let result = {
            let mut value = self
                .value
                .write()
                .unwrap_or_else(|error| error.into_inner());
            update(&mut value)
        };
        if let Some(redraw) = &self.redraw {
            redraw.request();
        }
        result
    }

    /// Replace the current value and request a redraw.
    pub fn set(&self, value: T) {
        self.update(|current| *current = value);
    }
}

/// A view derived from the current value of a [`Live`] on every measurement
/// and render pass.
pub struct LiveView<T, F> {
    value: Live<T>,
    build: F,
}

impl<T, F> LiveView<T, F>
where
    F: Fn(&T) -> Element,
{
    /// Bind `value` to a view-building function.
    pub fn new(value: Live<T>, build: F) -> Self {
        Self { value, build }
    }

    fn current(&self) -> Element {
        self.value.with(&self.build)
    }
}

impl<T, F> View for LiveView<T, F>
where
    F: Fn(&T) -> Element,
{
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        self.current().measure(available, ctx)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        self.current().render(area, surface, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Text;
    use crate::style::Theme;
    use crate::tests::support::row;
    use crate::view::element;

    // A waiter that registers before any request must still be woken, and a
    // request that lands first must not be lost. Polled by hand so the
    // guarantee is checked without an executor.
    #[cfg(feature = "async")]
    #[test]
    fn a_waiter_observes_requests_made_before_and_after_it_parks() {
        let handle = RedrawHandle::default();
        let waker = Waker::noop();
        let context = Context::from_waker(waker);

        assert_eq!(handle.poll_take(&context), Poll::Pending, "no request yet");
        handle.request();
        assert_eq!(
            handle.poll_take(&context),
            Poll::Ready(()),
            "a request made while parked is observed"
        );
        assert_eq!(
            handle.poll_take(&context),
            Poll::Pending,
            "the request was consumed"
        );

        let requested = RedrawHandle::default();
        requested.request();
        assert_eq!(
            requested.poll_take(&context),
            Poll::Ready(()),
            "a request made before the first poll is not lost"
        );
    }

    #[test]
    fn live_view_reads_updated_data_without_reconstruction() {
        let redraw = RedrawHandle::default();
        let value = Live::with_redraw(1u32, redraw.clone());
        let view = LiveView::new(value.clone(), |value| {
            element(Text::raw(format!("count: {value}")))
        });
        let theme = Theme::default();
        assert_eq!(
            row(&crate::testing::render(&view, 12, 1, &theme), 0),
            "count: 1"
        );
        value.set(2);
        assert!(redraw.take());
        assert_eq!(
            row(&crate::testing::render(&view, 12, 1, &theme), 0),
            "count: 2"
        );
    }
}
