//! CSS `cubic-bezier()` timing functions.

use crate::MotionError;

/// A CSS `cubic-bezier(x1, y1, x2, y2)` timing function with endpoints fixed
/// at `(0, 0)` and `(1, 1)`.
///
/// # Evaluation
///
/// For input progress `x`, the curve parameter `t` with `X(t) = x` is found by
/// up to eight Newton-Raphson steps starting at `t = x` (stopping once
/// `|X(t) - x| < 1e-6`, or when `|X'(t)| < 1e-6`), falling back to 32 steps of
/// bisection over `[0, 1]`. The output is `Y(t)` clamped to the convex hull of
/// the control-point Y values, `[min(0, y1, y2), max(1, y1, y2)]`. A curve
/// whose handles stay inside the unit square therefore never leaves `[0, 1]`,
/// and a curve with a handle outside it overshoots by at most its handle.
/// Inputs at or below 0 return 0 and inputs at or above 1 return 1.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CubicBezier {
    /// First control point X, in `[0, 1]`.
    pub x1: f32,
    /// First control point Y.
    pub y1: f32,
    /// Second control point X, in `[0, 1]`.
    pub x2: f32,
    /// Second control point Y.
    pub y2: f32,
}

impl CubicBezier {
    /// Creates a curve from control points without validation. Use
    /// [`Self::try_new`] for control points that come from data.
    #[inline]
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    /// Creates a curve, rejecting control points whose X lies outside
    /// `[0, 1]` or any non-finite coordinate.
    pub fn try_new(x1: f32, y1: f32, x2: f32, y2: f32) -> Result<Self, MotionError> {
        let unit = 0.0..=1.0;
        if !unit.contains(&x1) || !unit.contains(&x2) || !y1.is_finite() || !y2.is_finite() {
            return Err(MotionError::InvalidBezierControlPoints { x1, x2 });
        }
        Ok(Self { x1, y1, x2, y2 })
    }

    #[inline]
    fn coefficients(a: f32, b: f32) -> (f32, f32, f32) {
        let c = 3.0 * a;
        let bb = 3.0 * (b - a) - c;
        let aa = 1.0 - c - bb;
        (aa, bb, c)
    }

    /// `X(t)` for curve parameter `t`.
    #[inline]
    pub fn sample_x(&self, t: f32) -> f32 {
        let (a, b, c) = Self::coefficients(self.x1, self.x2);
        ((a * t + b) * t + c) * t
    }

    /// `Y(t)` for curve parameter `t`.
    #[inline]
    pub fn sample_y(&self, t: f32) -> f32 {
        let (a, b, c) = Self::coefficients(self.y1, self.y2);
        ((a * t + b) * t + c) * t
    }

    #[inline]
    fn sample_x_derivative(&self, t: f32) -> f32 {
        let (a, b, c) = Self::coefficients(self.x1, self.x2);
        (3.0 * a * t + 2.0 * b) * t + c
    }

    #[inline]
    fn sample_y_derivative(&self, t: f32) -> f32 {
        let (a, b, c) = Self::coefficients(self.y1, self.y2);
        (3.0 * a * t + 2.0 * b) * t + c
    }

    /// The curve parameter `t` with `X(t) = x`, for `x` in `[0, 1]`.
    pub fn solve_t_for_x(&self, x: f32) -> f32 {
        let mut t = x;
        for _ in 0..8 {
            let err = self.sample_x(t) - x;
            if err.abs() < 1e-6 {
                return t;
            }
            let d = self.sample_x_derivative(t);
            if d.abs() < 1e-6 {
                break;
            }
            t -= err / d;
        }
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        for _ in 0..32 {
            let mid = (lo + hi) / 2.0;
            if self.sample_x(mid) < x {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo + hi) / 2.0
    }

    /// The range [`Self::eval`] returns: the convex hull of the control-point
    /// Y values and the endpoints.
    #[inline]
    pub const fn bounds(&self) -> (f32, f32) {
        let floor = if self.y1 < self.y2 { self.y1 } else { self.y2 };
        let ceiling = if self.y1 > self.y2 { self.y1 } else { self.y2 };
        (
            if floor < 0.0 { floor } else { 0.0 },
            if ceiling > 1.0 { ceiling } else { 1.0 },
        )
    }

    /// Whether a handle leaves the unit square, which lets the output pass
    /// its endpoints.
    #[inline]
    pub const fn overshoots(&self) -> bool {
        self.y1 > 1.0 || self.y2 > 1.0 || self.y1 < 0.0 || self.y2 < 0.0
    }

    /// Eased output for input progress `x`.
    #[inline]
    pub fn eval(&self, x: f32) -> f32 {
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        let (floor, ceiling) = self.bounds();
        self.sample_y(self.solve_t_for_x(x)).clamp(floor, ceiling)
    }

    /// The slope `dY/dX` at input progress `x`: eased units per unit of
    /// progress. Zero outside `(0, 1)`.
    pub fn slope(&self, x: f32) -> f32 {
        if x <= 0.0 || x >= 1.0 {
            return 0.0;
        }
        let t = self.solve_t_for_x(x);
        let dx = self.sample_x_derivative(t);
        if dx.abs() < 1e-6 {
            0.0
        } else {
            self.sample_y_derivative(t) / dx
        }
    }
}

/// CSS `ease`: `cubic-bezier(0.25, 0.1, 0.25, 1)`.
pub const EASE: CubicBezier = CubicBezier::new(0.25, 0.1, 0.25, 1.0);
/// CSS `ease-in`: `cubic-bezier(0.42, 0, 1, 1)`.
pub const EASE_IN: CubicBezier = CubicBezier::new(0.42, 0.0, 1.0, 1.0);
/// CSS `ease-out`: `cubic-bezier(0, 0, 0.58, 1)`.
pub const EASE_OUT: CubicBezier = CubicBezier::new(0.0, 0.0, 0.58, 1.0);
/// CSS `ease-in-out`: `cubic-bezier(0.42, 0, 0.58, 1)`.
pub const EASE_IN_OUT: CubicBezier = CubicBezier::new(0.42, 0.0, 0.58, 1.0);
/// Exponential-style arrival: `cubic-bezier(0.16, 1, 0.3, 1)`. Covers most of
/// the distance in the first third of the span.
pub const EASE_OUT_EXPO: CubicBezier = CubicBezier::new(0.16, 1.0, 0.3, 1.0);
/// List reorder glide: `cubic-bezier(0.22, 1, 0.36, 1)`.
pub const EASE_RESORT: CubicBezier = CubicBezier::new(0.22, 1.0, 0.36, 1.0);
/// Platform-standard curve and Tailwind's default transition timing:
/// `cubic-bezier(0.4, 0, 0.2, 1)`.
pub const EASE_STANDARD: CubicBezier = CubicBezier::new(0.4, 0.0, 0.2, 1.0);
/// Fast deceleration: `cubic-bezier(0, 0, 0.2, 1)`.
pub const DECEL: CubicBezier = CubicBezier::new(0.0, 0.0, 0.2, 1.0);
/// Deceleration that passes the target and returns:
/// `cubic-bezier(0.34, 1.56, 0.64, 1)`. Peaks near `1.098`.
pub const EASE_OUT_BACK: CubicBezier = CubicBezier::new(0.34, 1.56, 0.64, 1.0);
