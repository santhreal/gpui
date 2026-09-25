//! Damped harmonic oscillators, integrated in closed form.
//!
//! A spring obeys `m x'' + c x' + k (x - target) = 0`. With natural frequency
//! `w0 = sqrt(k / m)` and damping ratio `zeta = c / (2 sqrt(k m))`, the state
//! `(x - target, v)` advances over `dt` by a 2x2 transition matrix (the
//! propagator) whose form depends on the regime: underdamped (`zeta < 1`),
//! critically damped (`|zeta - 1| <= 1e-4`), or overdamped (`zeta > 1`).
//! Stepping is exact for any `dt`, so results do not depend on frame rate and
//! an interrupted spring keeps its velocity when retargeted.

use std::time::Duration;

use crate::MotionError;

const CRITICAL_DAMPING_TOLERANCE: f32 = 1e-4;

/// Position tolerance of [`SpringState::is_at_rest`], in animated units.
pub const REST_POSITION_TOLERANCE: f32 = 0.001;
/// Velocity tolerance of [`SpringState::is_at_rest`], in animated units per
/// second.
pub const REST_VELOCITY_TOLERANCE: f32 = 0.01;

/// The physical parameters of a damped harmonic oscillator.
///
/// `stiffness` and `mass` must be finite and positive. `damping` must be
/// finite and non-negative. [`SpringConfig::try_new`] enforces this.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SpringConfig {
    /// The spring stiffness, `k`.
    pub stiffness: f32,
    /// The viscous damping coefficient, `c`.
    pub damping: f32,
    /// The moving mass, `m`.
    pub mass: f32,
}

/// The instantaneous position and velocity of a spring.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SpringState {
    /// The current value in the animated unit.
    pub position: f32,
    /// The current value's change per second.
    pub velocity: f32,
}

impl SpringState {
    /// A state at `position` with no velocity.
    #[inline]
    pub const fn at(position: f32) -> Self {
        Self {
            position,
            velocity: 0.0,
        }
    }

    /// Whether the state is within `position_tolerance` of `target` and its
    /// speed is within `velocity_tolerance`.
    #[inline]
    pub fn is_within(self, target: f32, position_tolerance: f32, velocity_tolerance: f32) -> bool {
        (self.position - target).abs() < position_tolerance
            && self.velocity.abs() < velocity_tolerance
    }

    /// Whether the state is at rest on `target`: within
    /// [`REST_POSITION_TOLERANCE`] and [`REST_VELOCITY_TOLERANCE`].
    #[inline]
    pub fn is_at_rest(self, target: f32) -> bool {
        self.is_within(target, REST_POSITION_TOLERANCE, REST_VELOCITY_TOLERANCE)
    }
}

impl SpringConfig {
    /// Creates a spring from its physical parameters without validation.
    #[inline]
    pub const fn new(stiffness: f32, damping: f32, mass: f32) -> Self {
        Self {
            stiffness,
            damping,
            mass,
        }
    }

    /// Creates a spring, rejecting non-physical parameters.
    pub fn try_new(stiffness: f32, damping: f32, mass: f32) -> Result<Self, MotionError> {
        let valid = stiffness.is_finite()
            && stiffness > 0.0
            && damping.is_finite()
            && damping >= 0.0
            && mass.is_finite()
            && mass > 0.0;
        if valid {
            Ok(Self::new(stiffness, damping, mass))
        } else {
            Err(MotionError::InvalidSpringParameters {
                stiffness,
                damping,
                mass,
            })
        }
    }

    /// A critically damped unit-mass spring: the fastest approach with no
    /// overshoot.
    #[inline]
    pub fn critical(stiffness: f32) -> Self {
        Self::new(stiffness, 2.0 * stiffness.sqrt(), 1.0)
    }

    /// Returns the natural angular frequency and damping ratio `(w0, zeta)`.
    #[inline]
    pub fn canonical(&self) -> (f32, f32) {
        let natural_frequency = (self.stiffness / self.mass).sqrt();
        let damping_ratio = self.damping / (2.0 * (self.stiffness * self.mass).sqrt());
        (natural_frequency, damping_ratio)
    }

    /// Advances a spring toward a target that stays fixed for `delta_time`
    /// seconds.
    #[inline]
    pub fn step(&self, state: SpringState, target: f32, delta_time: f32) -> SpringState {
        let propagator = self.propagator(delta_time);
        let displacement = state.position - target;
        SpringState {
            position: target + propagator[0][0] * displacement + propagator[0][1] * state.velocity,
            velocity: propagator[1][0] * displacement + propagator[1][1] * state.velocity,
        }
    }

    /// Advances a spring toward a target moving at a constant velocity.
    ///
    /// A first-order hold removes the frame-rate-dependent lag of treating a
    /// dragged target as stationary between frames.
    pub fn step_ramp(
        &self,
        state: SpringState,
        target: f32,
        target_velocity: f32,
        delta_time: f32,
    ) -> SpringState {
        let (natural_frequency, damping_ratio) = self.canonical();
        let steady_state_lag = -2.0 * damping_ratio * target_velocity / natural_frequency;
        let displacement = state.position - target - steady_state_lag;
        let velocity = state.velocity - target_velocity;
        let propagator = self.propagator(delta_time);
        let target = target + target_velocity * delta_time;
        SpringState {
            position: target
                + steady_state_lag
                + propagator[0][0] * displacement
                + propagator[0][1] * velocity,
            velocity: target_velocity
                + propagator[1][0] * displacement
                + propagator[1][1] * velocity,
        }
    }

    /// The state reached `elapsed` seconds after starting at `(x0, v0)`, with a
    /// fixed target. Returns the start state for `elapsed <= 0`.
    #[inline]
    pub fn evaluate(&self, x0: f32, v0: f32, target: f32, elapsed: f32) -> SpringState {
        let start = SpringState {
            position: x0,
            velocity: v0,
        };
        if elapsed <= 0.0 {
            start
        } else {
            self.step(start, target, elapsed)
        }
    }

    /// The exact state-transition matrix for a constant target over
    /// `delta_time` seconds. Springs that share a configuration and frame delta
    /// may share one matrix.
    pub fn propagator(&self, delta_time: f32) -> [[f32; 2]; 2] {
        let (natural_frequency, damping_ratio) = self.canonical();

        if damping_ratio < 1.0 - CRITICAL_DAMPING_TOLERANCE {
            let decay = damping_ratio * natural_frequency;
            let damped_frequency = natural_frequency * (1.0 - damping_ratio * damping_ratio).sqrt();
            let exponential = (-decay * delta_time).exp();
            let (sine, cosine) = (damped_frequency * delta_time).sin_cos();
            let sine_over_frequency = sine / damped_frequency;
            [
                [
                    exponential * (cosine + decay * sine_over_frequency),
                    exponential * sine_over_frequency,
                ],
                [
                    -exponential * natural_frequency * natural_frequency * sine_over_frequency,
                    exponential * (cosine - decay * sine_over_frequency),
                ],
            ]
        } else if damping_ratio > 1.0 + CRITICAL_DAMPING_TOLERANCE {
            let root = (damping_ratio * damping_ratio - 1.0).sqrt();
            let root_sum = damping_ratio + root;
            let slow_root = -natural_frequency / root_sum;
            let fast_root = -natural_frequency * root_sum;
            let denominator = slow_root - fast_root;
            let slow_exponential = (slow_root * delta_time).exp();
            let fast_exponential = (fast_root * delta_time).exp();
            [
                [
                    (-fast_root * slow_exponential + slow_root * fast_exponential) / denominator,
                    (slow_exponential - fast_exponential) / denominator,
                ],
                [
                    slow_root * fast_root * (fast_exponential - slow_exponential) / denominator,
                    (slow_root * slow_exponential - fast_root * fast_exponential) / denominator,
                ],
            ]
        } else {
            let exponential = (-natural_frequency * delta_time).exp();
            [
                [
                    exponential * (1.0 + natural_frequency * delta_time),
                    exponential * delta_time,
                ],
                [
                    -exponential * natural_frequency * natural_frequency * delta_time,
                    exponential * (1.0 - natural_frequency * delta_time),
                ],
            ]
        }
    }

    /// Tests displacement against `epsilon` and velocity against
    /// `epsilon * w0`, the matching units-per-second scale.
    #[inline]
    pub fn is_settled(&self, state: SpringState, target: f32, epsilon: f32) -> bool {
        let (natural_frequency, _) = self.canonical();
        epsilon.is_finite()
            && epsilon >= 0.0
            && (state.position - target).abs() <= epsilon
            && state.velocity.abs() <= epsilon * natural_frequency
    }

    /// A time after which the spring stays settled within `epsilon` (see
    /// [`Self::is_settled`]), found from the decay envelope rather than by
    /// sampling. Returns [`Duration::MAX`] for an undamped or invalid spring.
    pub fn settle_time(&self, state: SpringState, target: f32, epsilon: f32) -> Duration {
        let displacement = state.position - target;
        if displacement == 0.0 && state.velocity == 0.0 {
            return Duration::ZERO;
        }

        let (natural_frequency, damping_ratio) = self.canonical();
        if !natural_frequency.is_finite()
            || natural_frequency <= 0.0
            || !damping_ratio.is_finite()
            || damping_ratio <= 0.0
            || !epsilon.is_finite()
            || epsilon <= 0.0
        {
            return Duration::MAX;
        }

        let velocity_threshold = epsilon * natural_frequency;

        if damping_ratio < 1.0 - CRITICAL_DAMPING_TOLERANCE {
            let decay = damping_ratio * natural_frequency;
            let damped_frequency = natural_frequency * (1.0 - damping_ratio * damping_ratio).sqrt();
            let sine_coefficient = (state.velocity + decay * displacement) / damped_frequency;
            let position_envelope = displacement.hypot(sine_coefficient);
            let velocity_cosine = damped_frequency * sine_coefficient - decay * displacement;
            let velocity_sine = -damped_frequency * displacement - decay * sine_coefficient;
            let velocity_envelope = velocity_cosine.hypot(velocity_sine);
            find_settle_time(
                epsilon,
                velocity_threshold,
                0.0,
                natural_frequency,
                |time| {
                    let exponential = (-decay * time).exp();
                    (
                        position_envelope * exponential,
                        velocity_envelope * exponential,
                    )
                },
            )
        } else if damping_ratio > 1.0 + CRITICAL_DAMPING_TOLERANCE {
            let root = (damping_ratio * damping_ratio - 1.0).sqrt();
            let root_sum = damping_ratio + root;
            let slow_root = -natural_frequency / root_sum;
            let fast_root = -natural_frequency * root_sum;
            let denominator = slow_root - fast_root;
            let slow_coefficient = (state.velocity - fast_root * displacement) / denominator;
            let fast_coefficient = (slow_root * displacement - state.velocity) / denominator;
            find_settle_time(
                epsilon,
                velocity_threshold,
                0.0,
                natural_frequency,
                |time| {
                    let slow_term = slow_coefficient.abs() * (slow_root * time).exp();
                    let fast_term = fast_coefficient.abs() * (fast_root * time).exp();
                    (
                        slow_term + fast_term,
                        slow_root.abs() * slow_term + fast_root.abs() * fast_term,
                    )
                },
            )
        } else {
            let linear_coefficient = state.velocity + natural_frequency * displacement;
            let position_constant = displacement.abs();
            let position_linear = linear_coefficient.abs();
            let velocity_constant = (linear_coefficient - natural_frequency * displacement).abs();
            let velocity_linear = natural_frequency * linear_coefficient.abs();
            let position_decay_start =
                envelope_decay_start(position_constant, position_linear, natural_frequency);
            let velocity_decay_start =
                envelope_decay_start(velocity_constant, velocity_linear, natural_frequency);
            find_settle_time(
                epsilon,
                velocity_threshold,
                position_decay_start.max(velocity_decay_start),
                natural_frequency,
                |time| {
                    let exponential = (-natural_frequency * time).exp();
                    (
                        (position_constant + position_linear * time) * exponential,
                        (velocity_constant + velocity_linear * time) * exponential,
                    )
                },
            )
        }
    }

    /// Seconds until the spring started at `(x0, v0)` reaches rest on `target`
    /// (see [`SpringState::is_at_rest`]) and stays there for the following
    /// 20 ms, sampled every millisecond. Returns `None` when it does not rest
    /// within `max_seconds`, or when `max_seconds` is negative or not finite.
    ///
    /// Samples are taken at `n * 1 ms` rather than by accumulating a step, so
    /// the sweep ends for any finite window.
    pub fn time_to_rest(&self, x0: f32, v0: f32, target: f32, max_seconds: f32) -> Option<f32> {
        const STEP_SECONDS: f32 = 0.001;
        const VERIFY_STEP_SECONDS: f32 = 0.002;
        const VERIFY_STEPS: u32 = 10;

        if !max_seconds.is_finite() || max_seconds < 0.0 {
            return None;
        }
        let step_count = (max_seconds / STEP_SECONDS)
            .floor()
            .clamp(0.0, f32::from(u16::MAX) * 16.0) as u32;
        (0..=step_count).find_map(|step| {
            let t = step as f32 * STEP_SECONDS;
            if !self.evaluate(x0, v0, target, t).is_at_rest(target) {
                return None;
            }
            let stays = (1..=VERIFY_STEPS).all(|k| {
                let verify_t = (k as f32).mul_add(VERIFY_STEP_SECONDS, t);
                verify_t > max_seconds || self.evaluate(x0, v0, target, verify_t).is_at_rest(target)
            });
            stays.then_some(t)
        })
    }

    /// The amplitude envelope bounding `|x(t) - target|` for a spring released
    /// from rest at `x0`: `A e^(-zeta w0 t)` with `A = |x0 - target| /
    /// sqrt(1 - zeta^2)` when underdamped, and `|x0 - target| e^(-w0 t)`
    /// otherwise.
    pub fn decay_envelope(&self, x0: f32, target: f32, elapsed: f32) -> f32 {
        let initial_offset = (x0 - target).abs();
        let (natural_frequency, damping_ratio) = self.canonical();
        if damping_ratio < 1.0 - CRITICAL_DAMPING_TOLERANCE {
            let amplitude = initial_offset / damping_ratio.mul_add(-damping_ratio, 1.0).sqrt();
            amplitude * (-damping_ratio * natural_frequency * elapsed).exp()
        } else {
            initial_offset * (-natural_frequency * elapsed).exp()
        }
    }
}

fn envelope_decay_start(constant: f32, linear: f32, decay: f32) -> f32 {
    if linear == 0.0 {
        0.0
    } else {
        (1.0 / decay - constant / linear).max(0.0)
    }
}

fn find_settle_time(
    position_threshold: f32,
    velocity_threshold: f32,
    decay_start: f32,
    natural_frequency: f32,
    envelope: impl Fn(f32) -> (f32, f32),
) -> Duration {
    let is_below_threshold = |time| {
        let (position, velocity) = envelope(time);
        position <= position_threshold && velocity <= velocity_threshold
    };

    if is_below_threshold(decay_start) {
        return duration_from_secs(decay_start);
    }

    let mut lower_bound = decay_start;
    let mut upper_bound = decay_start.max(natural_frequency.recip());
    while !is_below_threshold(upper_bound) {
        lower_bound = upper_bound;
        upper_bound *= 2.0;
        if !upper_bound.is_finite() {
            return Duration::MAX;
        }
    }

    for _ in 0..32 {
        let midpoint = (lower_bound + upper_bound) / 2.0;
        if is_below_threshold(midpoint) {
            upper_bound = midpoint;
        } else {
            lower_bound = midpoint;
        }
    }

    duration_from_secs(upper_bound)
}

pub(crate) fn duration_from_secs(seconds: f32) -> Duration {
    if !seconds.is_finite() || seconds >= Duration::MAX.as_secs_f32() {
        Duration::MAX
    } else if seconds <= 0.0 {
        Duration::ZERO
    } else {
        Duration::from_secs_f32(seconds)
    }
}

/// A spring from 0 to 1 released at rest, sampled over a fixed span as an
/// easing function of normalized time.
///
/// `eval(0) = 0` and `eval(1) = 1` exactly; between them the value is the
/// spring's position at `progress * span` and may pass 1 for an underdamped
/// spring. Retargeting a curve restarts it; use [`SpringConfig::step`] to keep
/// velocity across retargets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringCurve {
    config: SpringConfig,
    span_seconds: f32,
}

impl SpringCurve {
    /// A curve spanning the time the spring takes to settle within `epsilon`
    /// (see [`SpringConfig::settle_time`]).
    pub fn settling(config: SpringConfig, epsilon: f32) -> Self {
        let span = config.settle_time(SpringState::at(0.0), 1.0, epsilon);
        Self {
            config,
            span_seconds: span.as_secs_f32(),
        }
    }

    /// A curve spanning a fixed `span`.
    pub const fn over(config: SpringConfig, span: Duration) -> Self {
        Self {
            config,
            span_seconds: span.as_secs_f32(),
        }
    }

    /// The spring configuration.
    #[inline]
    pub fn config(&self) -> SpringConfig {
        self.config
    }

    /// The wall-clock span that normalized time 1 maps to.
    pub fn span(&self) -> Duration {
        duration_from_secs(self.span_seconds)
    }

    /// Spring position at normalized time `progress`.
    #[inline]
    pub fn eval(&self, progress: f32) -> f32 {
        if progress <= 0.0 {
            0.0
        } else if progress >= 1.0 {
            1.0
        } else {
            self.config
                .step(SpringState::at(0.0), 1.0, progress * self.span_seconds)
                .position
        }
    }

    /// [`Self::eval`] clamped to `[0, 1]`.
    #[inline]
    pub fn eval_clamped(&self, progress: f32) -> f32 {
        self.eval(progress).clamp(0.0, 1.0)
    }
}
