//! Duration-based transitions: a span, an optional delay, and a curve.

use std::time::Duration;

use crate::{Easing, MotionPolicy, Timestamp, easing::lerp};

/// A duration-based transition: `delay_ms` of hold at 0, then `duration_ms`
/// of `easing`.
///
/// A timeline spans [`Self::total`], delay included. Raw timeline progress `r`
/// in `[0, 1]` maps to eased progress by
/// `easing((r * total - delay) / duration)`, with the argument clamped to
/// `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MotionSpec {
    /// Length of the eased part, in milliseconds.
    pub duration_ms: u64,
    /// Hold before the eased part starts, in milliseconds.
    pub delay_ms: u64,
    /// The curve applied over the eased part.
    pub easing: Easing,
}

impl MotionSpec {
    /// A spec with no delay.
    #[inline]
    pub const fn new(duration_ms: u64, easing: Easing) -> Self {
        Self {
            duration_ms,
            delay_ms: 0,
            easing,
        }
    }

    /// Returns this spec with a delay.
    #[inline]
    pub const fn with_delay(mut self, delay_ms: u64) -> Self {
        self.delay_ms = delay_ms;
        self
    }

    /// The eased span.
    #[inline]
    pub const fn duration(&self) -> Duration {
        Duration::from_millis(self.duration_ms)
    }

    /// The hold before the eased span.
    #[inline]
    pub const fn delay(&self) -> Duration {
        Duration::from_millis(self.delay_ms)
    }

    /// The whole timeline: delay plus duration.
    #[inline]
    pub const fn total(&self) -> Duration {
        Duration::from_millis(self.delay_ms + self.duration_ms)
    }

    /// Eased progress for raw timeline progress `raw_delta` in `[0, 1]`
    /// across [`Self::total`]. A spec with no duration returns 1.
    #[inline]
    pub fn progress(&self, raw_delta: f32) -> f32 {
        let total = (self.delay_ms + self.duration_ms) as f32;
        if total <= 0.0 || self.duration_ms == 0 {
            return 1.0;
        }
        let t =
            (raw_delta.clamp(0.0, 1.0) * total - self.delay_ms as f32) / self.duration_ms as f32;
        self.easing.eval(t.clamp(0.0, 1.0))
    }

    /// Eased progress `authored_seconds` after the timeline started, and
    /// whether the timeline has finished. Time is in authored seconds; see
    /// [`MotionPolicy::authored_seconds`].
    #[inline]
    pub fn sample(&self, authored_seconds: f32) -> (f32, bool) {
        let total = (self.delay_ms + self.duration_ms) as f32 / 1000.0;
        if total <= 0.0 || authored_seconds >= total {
            return (1.0, true);
        }
        (self.progress(authored_seconds / total), false)
    }
}

/// A transition of one value between two endpoints along a [`MotionSpec`].
///
/// The tween holds its start time; each sample reads the caller's `now`. A
/// finished tween, or any tween under reduced motion, reads as its target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tween<T> {
    /// Value at the start of the timeline.
    pub from: f32,
    /// Value at the end of the timeline.
    pub to: f32,
    /// Timeline start.
    pub started: T,
    /// Timing of the transition.
    pub spec: MotionSpec,
}

impl<T: Timestamp> Tween<T> {
    /// A tween from `from` to `to` starting at `now`.
    #[inline]
    pub const fn new(from: f32, to: f32, spec: MotionSpec, now: T) -> Self {
        Self {
            from,
            to,
            started: now,
            spec,
        }
    }

    /// The value at `now` and whether the tween has finished.
    #[inline]
    pub fn sample(&self, now: T, policy: MotionPolicy) -> (f32, bool) {
        if policy.reduced() || self.from == self.to {
            return (self.to, true);
        }
        let authored = policy.authored_seconds(now.seconds_since(self.started));
        let (progress, done) = self.spec.sample(authored);
        if done {
            (self.to, true)
        } else {
            (lerp(self.from, self.to, progress), false)
        }
    }

    /// The value at `now`.
    #[inline]
    pub fn value(&self, now: T, policy: MotionPolicy) -> f32 {
        self.sample(now, policy).0
    }

    /// Whether the tween is still moving at `now`.
    #[inline]
    pub fn is_active(&self, now: T, policy: MotionPolicy) -> bool {
        !self.sample(now, policy).1
    }

    /// Restarts the tween at `now` from its current value toward `to`, so the
    /// value is continuous across the change.
    pub fn retarget(&mut self, to: f32, now: T, policy: MotionPolicy) {
        if to == self.to {
            return;
        }
        self.from = self.value(now, policy);
        self.to = to;
        self.started = now;
    }
}
