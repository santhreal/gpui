//! One animated scalar driven by a [`MotionModel`], and a keyed registry of
//! them.
//!
//! An animator holds where its current motion started (value, velocity, time)
//! and samples in closed form at the caller's `now`, so results do not depend
//! on frame timing. Retargeting starts a new motion from the sampled value and
//! velocity, which keeps position and velocity continuous across the change.

use std::{
    collections::HashMap,
    hash::{BuildHasher, Hash, RandomState},
};

use crate::{MotionModel, MotionPolicy, ResolvedMotion, Timestamp};

/// Targets closer than this are the same target.
const TARGET_TOLERANCE: f32 = 1e-4;
/// Durations at or below this many seconds complete immediately.
const MIN_DURATION_SECONDS: f32 = 1e-4;

/// A sampled animator state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimatorSample {
    /// The value.
    pub value: f32,
    /// The rate of change, in value units per second.
    pub velocity: f32,
    /// Whether the motion has finished.
    pub at_rest: bool,
}

/// One scalar moving under a [`MotionModel`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Animator<T> {
    start_value: f32,
    start_velocity: f32,
    target: f32,
    started: Option<T>,
    model: MotionModel,
    duration_scale: f32,
    value: f32,
    velocity: f32,
}

impl<T: Timestamp> Animator<T> {
    /// An animator at rest on `value`.
    pub const fn at_rest(value: f32) -> Self {
        Self {
            start_value: value,
            start_velocity: 0.0,
            target: value,
            started: None,
            model: MotionModel::INSTANT,
            duration_scale: 1.0,
            value,
            velocity: 0.0,
        }
    }

    /// The value at the last [`Self::update`].
    #[inline]
    pub const fn value(&self) -> f32 {
        self.value
    }

    /// The velocity at the last [`Self::update`].
    #[inline]
    pub const fn velocity(&self) -> f32 {
        self.velocity
    }

    /// The value the current motion ends on.
    #[inline]
    pub const fn target(&self) -> f32 {
        self.target
    }

    /// The model of the current motion.
    #[inline]
    pub const fn model(&self) -> MotionModel {
        self.model
    }

    /// Whether the last [`Self::update`] found the motion finished.
    #[inline]
    pub const fn is_at_rest(&self) -> bool {
        self.started.is_none()
    }

    /// Samples the current motion at `now` without storing the result.
    pub fn sample(&self, now: T) -> AnimatorSample {
        let Some(started) = self.started else {
            return AnimatorSample {
                value: self.target,
                velocity: 0.0,
                at_rest: true,
            };
        };
        let elapsed = now.seconds_since(started) / self.duration_scale;
        let rest = AnimatorSample {
            value: self.target,
            velocity: 0.0,
            at_rest: true,
        };
        match self.model {
            MotionModel::Spring(spring)
            | MotionModel::SpringFade(crate::SpringFadeModel { spring, .. })
            | MotionModel::DirectThenSpring(crate::DirectThenSpringModel {
                snap_spring: spring,
            }) => {
                let state =
                    spring.evaluate(self.start_value, self.start_velocity, self.target, elapsed);
                if state.is_at_rest(self.target) {
                    rest
                } else {
                    AnimatorSample {
                        value: state.position,
                        velocity: state.velocity,
                        at_rest: false,
                    }
                }
            }
            MotionModel::Duration(crate::DurationModel { duration_ms, curve })
            | MotionModel::Flip(crate::FlipModel { duration_ms, curve }) => {
                let total = duration_ms as f32 / 1000.0;
                if total <= MIN_DURATION_SECONDS || elapsed >= total {
                    return rest;
                }
                let normalized = (elapsed / total).clamp(0.0, 1.0);
                let span = self.target - self.start_value;
                AnimatorSample {
                    value: span.mul_add(curve.eval(normalized), self.start_value),
                    velocity: span * curve.slope(normalized) / total,
                    at_rest: false,
                }
            }
            MotionModel::TwoStep(two_step) => {
                let half = two_step.period_ms as f32 / 2000.0;
                if half <= MIN_DURATION_SECONDS {
                    return rest;
                }
                let on = ((elapsed / half).floor() as u64).is_multiple_of(2);
                AnimatorSample {
                    value: if on { self.target } else { self.start_value },
                    velocity: 0.0,
                    at_rest: false,
                }
            }
        }
    }

    /// Samples at `now` and stores the result; a finished motion comes to rest
    /// on its target.
    pub fn update(&mut self, now: T) -> AnimatorSample {
        let sample = self.sample(now);
        self.value = sample.value;
        self.velocity = sample.velocity;
        if sample.at_rest {
            self.started = None;
        }
        sample
    }

    /// Starts a motion from `value` with `velocity` toward `target` at `now`.
    /// A start already at rest on its target does not run.
    pub fn start(
        &mut self,
        value: f32,
        velocity: f32,
        target: f32,
        model: MotionModel,
        policy: MotionPolicy,
        now: T,
    ) {
        self.start_value = value;
        self.start_velocity = velocity;
        self.target = target;
        self.model = model;
        self.duration_scale = policy.duration_scale();
        self.value = value;
        self.velocity = velocity;
        let resting = (value - target).abs() <= TARGET_TOLERANCE && velocity == 0.0;
        self.started = if resting && !matches!(model, MotionModel::TwoStep(_)) {
            None
        } else {
            Some(now)
        };
    }

    /// Moves toward a new `target` from the value and velocity sampled at
    /// `now`. A target within `1e-4` of the current one only updates.
    pub fn retarget(&mut self, target: f32, model: MotionModel, policy: MotionPolicy, now: T) {
        if (self.target - target).abs() <= TARGET_TOLERANCE {
            self.update(now);
            return;
        }
        let sample = self.sample(now);
        self.start_value = sample.value;
        self.start_velocity = sample.velocity;
        self.target = target;
        self.model = model;
        self.duration_scale = policy.duration_scale();
        self.value = sample.value;
        self.velocity = sample.velocity;
        self.started = Some(now);
    }

    /// Moves toward `target` under a resolved role motion. An instant or
    /// steady resolution places the animator at rest on `target`; any other
    /// retargets from the state sampled at `now`.
    pub fn apply(&mut self, target: f32, motion: ResolvedMotion, policy: MotionPolicy, now: T) {
        match motion {
            ResolvedMotion::Instant | ResolvedMotion::SteadyOn => self.snap(target),
            _ => self.retarget(target, motion.model(), policy, now),
        }
    }

    /// Places the animator at rest on `value`.
    pub fn snap(&mut self, value: f32) {
        *self = Self::at_rest(value);
    }
}

/// Animators keyed by identity, independent of any element tree.
#[derive(Debug, Clone)]
pub struct AnimatorRegistry<K, T, S = RandomState> {
    animators: HashMap<K, Animator<T>, S>,
}

impl<K, T, S: Default> Default for AnimatorRegistry<K, T, S> {
    fn default() -> Self {
        Self {
            animators: HashMap::default(),
        }
    }
}

impl<K: Hash + Eq + Copy, T: Timestamp, S: BuildHasher + Default> AnimatorRegistry<K, T, S> {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// The animator for `key`, created at rest on `initial` and started toward
    /// `target` when absent. An existing animator is retargeted to `target`.
    pub fn animate(
        &mut self,
        key: K,
        initial: f32,
        target: f32,
        model: MotionModel,
        policy: MotionPolicy,
        now: T,
    ) -> &mut Animator<T> {
        let animator = self.animators.entry(key).or_insert_with(|| {
            let mut animator = Animator::at_rest(initial);
            animator.start(initial, 0.0, target, model, policy, now);
            animator
        });
        animator.retarget(target, model, policy, now);
        animator
    }

    /// The animator for `key`.
    pub fn get(&self, key: &K) -> Option<&Animator<T>> {
        self.animators.get(key)
    }

    /// The animator for `key`, mutably.
    pub fn get_mut(&mut self, key: &K) -> Option<&mut Animator<T>> {
        self.animators.get_mut(key)
    }

    /// Updates every animator at `now` and returns how many are still moving.
    pub fn update_all(&mut self, now: T) -> usize {
        self.animators
            .values_mut()
            .map(|animator| animator.update(now))
            .filter(|sample| !sample.at_rest)
            .count()
    }

    /// Whether `key` is absent or at rest.
    pub fn is_at_rest(&self, key: &K) -> bool {
        self.animators.get(key).is_none_or(Animator::is_at_rest)
    }

    /// Whether any animator is moving at `now`.
    pub fn any_active(&self, now: T) -> bool {
        self.animators
            .values()
            .any(|animator| !animator.sample(now).at_rest)
    }

    /// Whether any animator whose key matches `filter` is moving at `now`.
    pub fn any_active_where(&self, now: T, mut filter: impl FnMut(&K) -> bool) -> bool {
        self.animators
            .iter()
            .any(|(key, animator)| filter(key) && !animator.sample(now).at_rest)
    }

    /// Removes the animator for `key`.
    pub fn remove(&mut self, key: &K) -> Option<Animator<T>> {
        self.animators.remove(key)
    }

    /// Drops every animator at rest.
    pub fn retain_active(&mut self) {
        self.animators.retain(|_, animator| !animator.is_at_rest());
    }

    /// Number of animators.
    pub fn len(&self) -> usize {
        self.animators.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.animators.is_empty()
    }

    /// Drops every animator.
    pub fn clear(&mut self) {
        self.animators.clear();
    }
}
