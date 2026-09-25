//! The clock motion values sample.

use scheduler::Instant;

use super::Timestamp;
use crate::App;

/// An instant on the clock GPUI animations sample: the background executor's
/// clock, which a test advances with `advance_clock`.
///
/// Read one per frame with [`App::frame_instant`] and pass it to every motion
/// value the frame samples, so the values of a frame agree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FrameInstant(Instant);

impl Timestamp for FrameInstant {
    #[inline]
    fn seconds_since(self, earlier: Self) -> f32 {
        self.0.saturating_duration_since(earlier.0).as_secs_f32()
    }
}

impl App {
    /// The current instant of the clock GPUI animations sample.
    pub fn frame_instant(&self) -> FrameInstant {
        FrameInstant(self.background_executor().now())
    }
}
