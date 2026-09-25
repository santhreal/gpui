//! A spring advanced by caller-supplied frame deltas.
//!
//! [`FrameSpring`] suits values coupled to continuous input, such as a hover
//! glow that follows the pointer: each frame moves it toward the current
//! target, and a target change mid-flight keeps the current velocity. Steps
//! use the exact propagator of [`SpringConfig::step`], so the result for a
//! given total time does not depend on how that time is split into frames.

use crate::{SpringConfig, SpringState};

/// Longest frame delta [`FrameSpring::step`] applies, in seconds. A stalled
/// frame advances the spring by this much instead of jumping to rest.
pub const MAX_FRAME_STEP_SECONDS: f32 = 0.05;

/// Critically damped unit-mass spring with stiffness 140: the approach used
/// for pointer-coupled state. `2 * sqrt(140) = 23.664_319`.
pub const POINTER_SPRING: SpringConfig = SpringConfig::new(140.0, 23.664_319, 1.0);

/// A value and velocity moved toward a target one frame at a time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameSpring {
    config: SpringConfig,
    state: SpringState,
}

impl Default for FrameSpring {
    /// A [`POINTER_SPRING`] at rest on zero.
    fn default() -> Self {
        Self::new(POINTER_SPRING, 0.0)
    }
}

impl FrameSpring {
    /// A spring at rest on `value`.
    #[inline]
    pub const fn new(config: SpringConfig, value: f32) -> Self {
        Self {
            config,
            state: SpringState::at(value),
        }
    }

    /// The current value.
    #[inline]
    pub const fn value(&self) -> f32 {
        self.state.position
    }

    /// The current velocity, in value units per second.
    #[inline]
    pub const fn velocity(&self) -> f32 {
        self.state.velocity
    }

    /// The current value and velocity.
    #[inline]
    pub const fn state(&self) -> SpringState {
        self.state
    }

    /// The spring parameters.
    #[inline]
    pub const fn config(&self) -> SpringConfig {
        self.config
    }

    /// Advances toward `target` by `delta_seconds`, clamped to
    /// `0..=`[`MAX_FRAME_STEP_SECONDS`], and returns the new value. A spring
    /// at rest on `target` stays there.
    #[inline]
    pub fn step(&mut self, target: f32, delta_seconds: f32) -> f32 {
        let delta_seconds = if delta_seconds.is_finite() {
            delta_seconds.clamp(0.0, MAX_FRAME_STEP_SECONDS)
        } else {
            0.0
        };
        self.state = self.config.step(self.state, target, delta_seconds);
        self.state.position
    }

    /// Whether the spring is at rest on `target` (see
    /// [`SpringState::is_at_rest`]).
    #[inline]
    pub fn is_at_rest(&self, target: f32) -> bool {
        self.state.is_at_rest(target)
    }

    /// Places the spring at rest on `value`.
    #[inline]
    pub fn snap(&mut self, value: f32) {
        self.state = SpringState::at(value);
    }
}
