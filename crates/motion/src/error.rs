use std::fmt;

/// Errors from validating motion parameters and parsing motion names.
#[derive(Debug, Clone, PartialEq)]
pub enum MotionError {
    /// Spring parameters are non-physical: stiffness and mass must be finite
    /// and positive, damping finite and non-negative.
    InvalidSpringParameters {
        /// The rejected stiffness.
        stiffness: f32,
        /// The rejected damping coefficient.
        damping: f32,
        /// The rejected mass.
        mass: f32,
    },
    /// A cubic bezier control point has an X coordinate outside `[0, 1]`, so
    /// the curve is not a function of time.
    InvalidBezierControlPoints {
        /// The rejected first control point X.
        x1: f32,
        /// The rejected second control point X.
        x2: f32,
    },
    /// A motion role name is not one of the seven roles.
    UnknownRole(String),
    /// An easing curve name is not one of the named curves.
    UnknownCurve(String),
}

impl fmt::Display for MotionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSpringParameters {
                stiffness,
                damping,
                mass,
            } => write!(
                f,
                "invalid spring parameters: stiffness {stiffness}, damping {damping}, mass {mass}; \
                 stiffness and mass must be finite and positive, damping finite and non-negative"
            ),
            Self::InvalidBezierControlPoints { x1, x2 } => write!(
                f,
                "invalid cubic bezier control points: x1={x1}, x2={x2}; both X coordinates must lie \
                 within [0, 1]"
            ),
            Self::UnknownRole(name) => write!(
                f,
                "unknown motion role '{name}'; expected one of: {}",
                crate::MotionRole::NAMES.join(", ")
            ),
            Self::UnknownCurve(name) => write!(
                f,
                "unknown easing curve '{name}'; expected one of: {}",
                crate::Easing::NAMES.join(", ")
            ),
        }
    }
}

impl std::error::Error for MotionError {}
