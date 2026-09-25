//! Named easing curves and closed-form easing functions.
//!
//! Every function maps normalized time in `[0, 1]` to eased progress. The
//! closed-form functions are `fn(f32) -> f32` so they pass directly wherever
//! an easing closure is expected.

use std::f32::consts::PI;

use crate::{
    CubicBezier, MotionError,
    bezier::{
        DECEL, EASE, EASE_IN, EASE_IN_OUT, EASE_OUT, EASE_OUT_BACK, EASE_OUT_EXPO, EASE_RESORT,
        EASE_STANDARD,
    },
};

/// A named easing curve, a custom cubic bezier, or a step.
///
/// Named variants evaluate their [`CubicBezier`] constant; [`Easing::Linear`]
/// returns its clamped input and [`Easing::Step`] holds 0 until the end.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum Easing {
    /// Constant velocity.
    #[default]
    Linear,
    /// CSS `ease`.
    Ease,
    /// CSS `ease-in`.
    EaseIn,
    /// CSS `ease-out`.
    EaseOut,
    /// CSS `ease-in-out`.
    EaseInOut,
    /// Fast deceleration, `cubic-bezier(0, 0, 0.2, 1)`.
    Decel,
    /// Exponential-style arrival, `cubic-bezier(0.16, 1, 0.3, 1)`.
    EaseOutExpo,
    /// List reorder glide, `cubic-bezier(0.22, 1, 0.36, 1)`.
    EaseResort,
    /// Platform-standard curve, `cubic-bezier(0.4, 0, 0.2, 1)`.
    Standard,
    /// Overshooting deceleration, `cubic-bezier(0.34, 1.56, 0.64, 1)`.
    EaseOutBack,
    /// Holds 0 until progress reaches 1.
    Step,
    /// A custom cubic bezier.
    Bezier(CubicBezier),
}

impl Easing {
    /// Every named variant, in declaration order.
    pub const NAMED: [Self; 11] = [
        Self::Linear,
        Self::Ease,
        Self::EaseIn,
        Self::EaseOut,
        Self::EaseInOut,
        Self::Decel,
        Self::EaseOutExpo,
        Self::EaseResort,
        Self::Standard,
        Self::EaseOutBack,
        Self::Step,
    ];

    /// The `snake_case` name of every named variant, in [`Self::NAMED`] order.
    pub const NAMES: [&'static str; 11] = [
        "linear",
        "ease",
        "ease_in",
        "ease_out",
        "ease_in_out",
        "decel",
        "ease_out_expo",
        "ease_resort",
        "standard",
        "ease_out_back",
        "step",
    ];

    /// The cubic bezier behind this curve, or `None` for [`Self::Linear`] and
    /// [`Self::Step`].
    #[inline]
    pub const fn bezier(self) -> Option<CubicBezier> {
        match self {
            Self::Linear | Self::Step => None,
            Self::Ease => Some(EASE),
            Self::EaseIn => Some(EASE_IN),
            Self::EaseOut => Some(EASE_OUT),
            Self::EaseInOut => Some(EASE_IN_OUT),
            Self::Decel => Some(DECEL),
            Self::EaseOutExpo => Some(EASE_OUT_EXPO),
            Self::EaseResort => Some(EASE_RESORT),
            Self::Standard => Some(EASE_STANDARD),
            Self::EaseOutBack => Some(EASE_OUT_BACK),
            Self::Bezier(curve) => Some(curve),
        }
    }

    /// Eased progress for normalized time `t`. Inputs outside `[0, 1]` clamp.
    #[inline]
    pub fn eval(self, t: f32) -> f32 {
        match self {
            Self::Linear => t.clamp(0.0, 1.0),
            Self::Step => {
                if t < 1.0 {
                    0.0
                } else {
                    1.0
                }
            }
            _ => match self.bezier() {
                Some(curve) => curve.eval(t),
                None => t.clamp(0.0, 1.0),
            },
        }
    }

    /// The slope of the curve at normalized time `t`, in eased units per unit
    /// of time. Zero outside `(0, 1)` and for [`Self::Step`].
    pub fn slope(self, t: f32) -> f32 {
        match self {
            Self::Linear => {
                if t > 0.0 && t < 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Step => 0.0,
            _ => self.bezier().map_or(0.0, |curve| curve.slope(t)),
        }
    }

    /// The `snake_case` name, or `"custom"` for [`Self::Bezier`].
    pub const fn name(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Ease => "ease",
            Self::EaseIn => "ease_in",
            Self::EaseOut => "ease_out",
            Self::EaseInOut => "ease_in_out",
            Self::Decel => "decel",
            Self::EaseOutExpo => "ease_out_expo",
            Self::EaseResort => "ease_resort",
            Self::Standard => "standard",
            Self::EaseOutBack => "ease_out_back",
            Self::Step => "step",
            Self::Bezier(_) => "custom",
        }
    }

    /// Parses a named variant from its `snake_case` name.
    pub fn from_name(name: &str) -> Result<Self, MotionError> {
        Self::NAMES
            .iter()
            .position(|candidate| *candidate == name)
            .map(|index| Self::NAMED[index])
            .ok_or_else(|| MotionError::UnknownCurve(name.to_string()))
    }
}

impl From<CubicBezier> for Easing {
    fn from(curve: CubicBezier) -> Self {
        Self::Bezier(curve)
    }
}

/// Linear easing: returns `t` unchanged.
#[inline]
pub fn linear(t: f32) -> f32 {
    t
}

/// Quadratic ease-in: `t * t`.
#[inline]
pub fn quadratic(t: f32) -> f32 {
    t * t
}

/// Quadratic ease-in-out: slow start and end, fastest at the midpoint.
#[inline]
pub fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        2.0 * t * t
    } else {
        let x = -2.0 * t + 2.0;
        1.0 - x * x / 2.0
    }
}

/// Quadratic ease-out, `1 - (1 - t)^2`, with `t` clamped to `[0, 1]`.
#[inline]
pub fn ease_out_quad(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t)
}

/// Cubic ease-out, `1 - (1 - t)^3`, with `t` clamped to `[0, 1]`.
#[inline]
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Quintic ease-out, `1 - (1 - t)^5`.
#[inline]
pub fn ease_out_quint(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(5)
}

/// Runs `easing` forward over the first half of the span and backward over
/// the second half.
#[inline]
pub fn bounce(easing: impl Fn(f32) -> f32) -> impl Fn(f32) -> f32 {
    move |t| {
        if t < 0.5 {
            easing(t * 2.0)
        } else {
            easing((1.0 - t) * 2.0)
        }
    }
}

/// A breathing pulse between `min` and `max`: `s = sin(2 pi t)`, output
/// `min + (max - min) * ((s^3 + s) / 2 + 1) / 2`. Dwells near both ends.
#[inline]
pub fn pulsating_between(min: f32, max: f32) -> impl Fn(f32) -> f32 {
    let range = max - min;
    move |t| {
        let s = (t * 2.0 * PI).sin();
        let breath = (s * s * s + s) / 2.0;
        min + (breath + 1.0) / 2.0 * range
    }
}

/// Linear interpolation, `from + (to - from) * t`, unclamped.
#[inline]
pub fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}
