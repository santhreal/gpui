//! Caller-supplied time and the process-wide motion policy.
//!
//! Nothing in this crate reads a clock. Every stateful type takes a
//! [`Timestamp`] from the caller, so a frame samples every value at one
//! instant and tests drive time explicitly.

use std::time::{Duration, Instant};

/// A monotonic point in time supplied by the caller.
pub trait Timestamp: Copy {
    /// Seconds from `earlier` to `self`, or zero when `earlier` is later.
    fn seconds_since(self, earlier: Self) -> f32;
}

impl Timestamp for Instant {
    #[inline]
    fn seconds_since(self, earlier: Self) -> f32 {
        self.saturating_duration_since(earlier).as_secs_f32()
    }
}

/// A duration measured from a caller-chosen epoch.
impl Timestamp for Duration {
    #[inline]
    fn seconds_since(self, earlier: Self) -> f32 {
        self.saturating_sub(earlier).as_secs_f32()
    }
}

/// Seconds measured from a caller-chosen epoch.
impl Timestamp for f64 {
    #[inline]
    fn seconds_since(self, earlier: Self) -> f32 {
        let seconds = self - earlier;
        if seconds > 0.0 { seconds as f32 } else { 0.0 }
    }
}

/// Reduced-motion and duration-scale settings applied to every animation.
///
/// Reduced motion collapses each motion to its reduced variant (see
/// [`crate::resolve_motion`]) and settles time-based transitions at their
/// targets. The duration scale multiplies every duration: `4.0` runs each
/// animation four times slower, for frame-by-frame inspection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionPolicy {
    reduced: bool,
    duration_scale: f32,
}

impl Default for MotionPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl MotionPolicy {
    /// Full motion at authored speed.
    pub const DEFAULT: Self = Self {
        reduced: false,
        duration_scale: 1.0,
    };
    /// Reduced motion at authored speed.
    pub const REDUCED: Self = Self {
        reduced: true,
        duration_scale: 1.0,
    };
    /// Smallest accepted duration scale. Zero would finish every transition
    /// before its first frame.
    pub const MIN_DURATION_SCALE: f32 = 0.01;
    /// Largest accepted duration scale.
    pub const MAX_DURATION_SCALE: f32 = 100.0;

    /// Whether reduced motion is on.
    #[inline]
    pub const fn reduced(self) -> bool {
        self.reduced
    }

    /// The multiplier applied to every duration.
    #[inline]
    pub const fn duration_scale(self) -> f32 {
        self.duration_scale
    }

    /// Returns this policy with reduced motion set to `reduced`.
    #[inline]
    pub const fn with_reduced(mut self, reduced: bool) -> Self {
        self.reduced = reduced;
        self
    }

    /// Returns this policy with the duration scale set to `scale`, clamped to
    /// [`Self::MIN_DURATION_SCALE`]..=[`Self::MAX_DURATION_SCALE`]. A
    /// non-finite scale resets to `1.0`.
    #[inline]
    pub fn with_duration_scale(mut self, scale: f32) -> Self {
        self.duration_scale = if scale.is_finite() {
            scale.clamp(Self::MIN_DURATION_SCALE, Self::MAX_DURATION_SCALE)
        } else {
            1.0
        };
        self
    }

    /// Parses a duration scale such as the value of a slow-motion environment
    /// variable. Returns `None` for text that is not a finite number; clamps
    /// finite numbers to the accepted range.
    pub fn parse_duration_scale(text: &str) -> Option<f32> {
        let scale = text.trim().parse::<f32>().ok()?;
        scale
            .is_finite()
            .then(|| scale.clamp(Self::MIN_DURATION_SCALE, Self::MAX_DURATION_SCALE))
    }

    /// Scales an authored duration by the policy.
    #[inline]
    pub fn scale(self, duration: Duration) -> Duration {
        if self.duration_scale == 1.0 {
            duration
        } else {
            duration.mul_f32(self.duration_scale)
        }
    }

    /// Converts wall-clock seconds into authored seconds: the time an
    /// animation authored at scale `1.0` has advanced.
    #[inline]
    pub fn authored_seconds(self, wall_seconds: f32) -> f32 {
        wall_seconds / self.duration_scale
    }
}
