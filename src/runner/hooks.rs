//! What a runner does around each event and frame on behalf of optional
//! tooling — today only the devtools inspector.
//!
//! Every method is a no-op without `feature = "devtools"`, so both loops call
//! them unconditionally and carry no `cfg` of their own.

use std::time::{Duration, Instant};

use crate::buffer::Buffer;
use crate::geometry::Rect;
use crate::{Event, Theme, UpdateResult};

#[derive(Clone, Debug, Default)]
pub(crate) struct FrameHooks {
    #[cfg(feature = "devtools")]
    pub(crate) inspector: Option<crate::devtools::Inspector>,
}

impl FrameHooks {
    /// Whether tooling claimed `event` before the application could see it.
    pub(crate) fn intercept(&self, event: &Event) -> bool {
        #[cfg(feature = "devtools")]
        if let Some(inspector) = &self.inspector {
            return inspector.handle(event);
        }
        let _ = event;
        false
    }

    /// Record a delivered event. Returns whether the frame must be repainted
    /// to show it, even if the application left it clean.
    pub(crate) fn delivered(&self, event: &Event, result: UpdateResult) -> bool {
        #[cfg(feature = "devtools")]
        if let Some(inspector) = &self.inspector {
            inspector.log_event(event, result);
            return inspector.mode().is_visible();
        }
        let _ = (event, result);
        false
    }

    /// Start a frame; the returned instant times the application's paint.
    pub(crate) fn begin_frame(&self) -> Instant {
        #[cfg(feature = "devtools")]
        if let Some(inspector) = &self.inspector {
            inspector.begin_frame();
        }
        Instant::now()
    }

    /// Paint tooling over the finished application frame.
    pub(crate) fn paint(&self, buffer: &mut Buffer, area: Rect, theme: &Theme) {
        #[cfg(feature = "devtools")]
        if let Some(inspector) = &self.inspector {
            inspector.paint(buffer, area, theme);
        }
        let _ = (buffer, area, theme);
    }

    /// Record a flushed frame.
    pub(crate) fn finish_frame(&self, paint: Duration, damage: usize) {
        #[cfg(feature = "devtools")]
        if let Some(inspector) = &self.inspector {
            inspector.record_frame(paint, damage);
        }
        let _ = (paint, damage);
    }
}
