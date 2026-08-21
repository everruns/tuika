//! The few `Stream` adapters the async runner needs.
//!
//! The runner drives two streams (terminal events and application messages) and
//! nothing else, so it needs exactly three things: the [`Stream`] trait to name
//! in its bounds, a cancel-safe `next`, and a `filter_map` to turn crossterm's
//! raw events into tuika's. `tokio-stream` supplies all three, but the trait it
//! offers *is* `futures_core::Stream` — which the build already contains,
//! because the `async` feature turns on `crossterm/event-stream` and that pulls
//! `futures-core` in. Owning the three adapters therefore drops a dependency
//! without adding one, and keeps the public bounds on
//! [`run_with_messages`](super::AsyncRunner::run_with_messages) unchanged: a
//! host still passes a `tokio_stream::wrappers::ReceiverStream` or any other
//! `futures` stream, because it is the same trait.

use std::pin::Pin;
use std::task::{Context, Poll};

pub use futures_core::Stream;

/// Await the stream's next item.
///
/// Cancel-safe, which is what lets this be a `tokio::select!` branch: the
/// returned future only ever observes a `Poll::Ready` item by yielding it, so a
/// branch that loses the select drops the future without consuming anything.
pub(crate) fn next<S>(stream: &mut S) -> impl Future<Output = Option<S::Item>> + '_
where
    S: Stream + Unpin + ?Sized,
{
    std::future::poll_fn(move |cx| Pin::new(&mut *stream).poll_next(cx))
}

/// Map each item and drop the `None`s, the `StreamExt::filter_map` shape.
pub(crate) fn filter_map<S, F, T>(stream: S, f: F) -> FilterMap<S, F>
where
    S: Stream + Unpin,
    F: FnMut(S::Item) -> Option<T>,
{
    FilterMap { stream, f }
}

/// The stream returned by [`filter_map`].
pub(crate) struct FilterMap<S, F> {
    stream: S,
    f: F,
}

// `Unpin` on the source is required by the constructor, so this adapter is
// `Unpin` too and can be projected with `Pin::new` instead of a pin-projection
// dependency.
impl<S, F, T> Stream for FilterMap<S, F>
where
    S: Stream + Unpin,
    F: FnMut(S::Item) -> Option<T> + Unpin,
{
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let this = self.get_mut();
        loop {
            match Pin::new(&mut this.stream).poll_next(cx) {
                // A filtered-out item is not a wake-up: keep polling the source
                // until it yields a kept item or reports pending/end itself.
                Poll::Ready(Some(item)) => match (this.f)(item) {
                    Some(mapped) => return Poll::Ready(Some(mapped)),
                    None => continue,
                },
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// A stream that is immediately and permanently exhausted.
pub(crate) fn empty<T>() -> Empty<T> {
    Empty(std::marker::PhantomData)
}

/// The stream returned by [`empty`].
pub(crate) struct Empty<T>(std::marker::PhantomData<fn() -> T>);

impl<T> Stream for Empty<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<T>> {
        Poll::Ready(None)
    }
}

/// A stream that yields an iterator's items and then ends.
#[cfg(test)]
pub(crate) fn iter<I>(items: I) -> Iter<I::IntoIter>
where
    I: IntoIterator,
{
    Iter(items.into_iter())
}

/// The stream returned by [`iter`].
#[cfg(test)]
pub(crate) struct Iter<I>(I);

#[cfg(test)]
impl<I: Iterator + Unpin> Stream for Iter<I> {
    type Item = I::Item;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<I::Item>> {
        Poll::Ready(self.get_mut().0.next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn iter_yields_then_ends() {
        let mut s = iter([1, 2, 3]);
        assert_eq!(next(&mut s).await, Some(1));
        assert_eq!(next(&mut s).await, Some(2));
        assert_eq!(next(&mut s).await, Some(3));
        assert_eq!(next(&mut s).await, None);
        // Exhaustion is permanent, so a select branch gated on "not done" can
        // rely on one `None` meaning the source is finished.
        assert_eq!(next(&mut s).await, None);
    }

    #[tokio::test]
    async fn empty_ends_immediately() {
        let mut s = empty::<u8>();
        assert_eq!(next(&mut s).await, None);
    }

    #[tokio::test]
    async fn filter_map_drops_and_maps() {
        let mut s = filter_map(iter([1, 2, 3, 4]), |n| (n % 2 == 0).then(|| n * 10));
        assert_eq!(next(&mut s).await, Some(20));
        assert_eq!(next(&mut s).await, Some(40));
        assert_eq!(next(&mut s).await, None);
    }

    #[tokio::test]
    async fn filter_map_skips_a_leading_run_without_stalling() {
        // Every item before the first kept one is filtered out; the adapter must
        // keep polling rather than return `Pending` with no waker registered.
        let mut s = filter_map(iter([1, 1, 1, 2]), |n| (n % 2 == 0).then_some(n));
        assert_eq!(next(&mut s).await, Some(2));
        assert_eq!(next(&mut s).await, None);
    }
}
