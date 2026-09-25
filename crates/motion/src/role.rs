//! Motion roles, their motion models, and reduced-motion resolution.

use crate::{Easing, MotionError, MotionTokens, SpringConfig};

/// The seven kinds of interface motion. Every animated value belongs to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum MotionRole {
    /// Hover, focus, selection, badge color, scrim.
    Tint,
    /// Expand and collapse.
    Reveal,
    /// Menus, popovers, dialogs, palettes, attached cards.
    Float,
    /// Panel, drawer, and split resize.
    Panel,
    /// List rows entering, leaving, or being dragged.
    Shift,
    /// Programmatic scroll to a position.
    Scroll,
    /// Streaming caret cadence.
    Caret,
}

impl MotionRole {
    /// Every role, in declaration order.
    pub const ALL: [Self; 7] = [
        Self::Tint,
        Self::Reveal,
        Self::Float,
        Self::Panel,
        Self::Shift,
        Self::Scroll,
        Self::Caret,
    ];

    /// The `snake_case` name of every role, in [`Self::ALL`] order.
    pub const NAMES: [&'static str; 7] = [
        "tint", "reveal", "float", "panel", "shift", "scroll", "caret",
    ];

    /// The `snake_case` name.
    pub const fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    /// Parses a role from its `snake_case` name.
    pub fn from_name(name: &str) -> Result<Self, MotionError> {
        Self::NAMES
            .iter()
            .position(|candidate| *candidate == name)
            .map(|index| Self::ALL[index])
            .ok_or_else(|| MotionError::UnknownRole(name.to_string()))
    }
}

/// A duration-based transition.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DurationModel {
    /// Transition length in milliseconds.
    pub duration_ms: u32,
    /// Curve over the transition.
    pub curve: Easing,
}

/// A spring for position paired with a duration-based opacity fade.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SpringFadeModel {
    /// Spring driving position.
    pub spring: SpringConfig,
    /// Distance travelled while entering, in pixels.
    pub rise_px: f32,
    /// Opacity fade length in milliseconds.
    pub fade_duration_ms: u32,
}

/// Direct tracking while dragged, then a spring to the release target.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DirectThenSpringModel {
    /// Spring after release.
    pub snap_spring: SpringConfig,
}

/// A layout change animated as a translation from the old position to the new
/// one (first, last, invert, play).
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FlipModel {
    /// Transition length in milliseconds.
    pub duration_ms: u32,
    /// Curve over the transition.
    pub curve: Easing,
}

/// A two-step blink: target for half a period, start for the other half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TwoStepModel {
    /// Full on-off period in milliseconds.
    pub period_ms: u32,
}

/// The motion model of a role.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(tag = "model", rename_all = "snake_case")
)]
pub enum MotionModel {
    /// Duration-based transition.
    Duration(DurationModel),
    /// Spring.
    Spring(SpringConfig),
    /// Spring position with an opacity fade.
    SpringFade(SpringFadeModel),
    /// Direct tracking, then a spring.
    DirectThenSpring(DirectThenSpringModel),
    /// Translation from the old layout position.
    Flip(FlipModel),
    /// Two-step blink.
    TwoStep(TwoStepModel),
}

impl MotionModel {
    /// A model that reaches its target immediately.
    pub const INSTANT: Self = Self::Duration(DurationModel {
        duration_ms: 0,
        curve: Easing::Linear,
    });
}

/// The motion a role runs under the current policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResolvedMotion {
    /// No transition.
    Instant,
    /// Duration-based transition.
    Duration {
        /// Transition length in milliseconds.
        duration_ms: u32,
        /// Curve over the transition.
        curve: Easing,
    },
    /// Spring.
    Spring(SpringConfig),
    /// Opacity fade with no spatial movement.
    FadeOnly {
        /// Fade length in milliseconds.
        duration_ms: u32,
    },
    /// Steady on, no blinking.
    SteadyOn,
}

impl ResolvedMotion {
    /// The motion model that runs this resolution. A fade-only resolution
    /// runs as an `ease-out` duration model.
    pub const fn model(self) -> MotionModel {
        match self {
            Self::Spring(spring) => MotionModel::Spring(spring),
            Self::FadeOnly { duration_ms } => MotionModel::Duration(DurationModel {
                duration_ms,
                curve: Easing::EaseOut,
            }),
            Self::Duration { duration_ms, curve } => {
                MotionModel::Duration(DurationModel { duration_ms, curve })
            }
            Self::Instant | Self::SteadyOn => MotionModel::INSTANT,
        }
    }
}

/// Opacity-fade length of the reduced variants of [`MotionRole::Reveal`] and
/// [`MotionRole::Float`], in milliseconds.
pub const REDUCED_FADE_MS: u32 = 60;

/// Selects the motion for `role` from `tokens`, or its reduced variant when
/// `reduced` is set. The one place reduced motion changes a role's motion.
pub const fn resolve_motion(
    role: MotionRole,
    tokens: &MotionTokens,
    reduced: bool,
) -> ResolvedMotion {
    if reduced {
        return match role {
            MotionRole::Reveal | MotionRole::Float => ResolvedMotion::FadeOnly {
                duration_ms: REDUCED_FADE_MS,
            },
            MotionRole::Caret => ResolvedMotion::SteadyOn,
            MotionRole::Tint | MotionRole::Panel | MotionRole::Shift | MotionRole::Scroll => {
                ResolvedMotion::Instant
            }
        };
    }
    match role {
        MotionRole::Tint => ResolvedMotion::Duration {
            duration_ms: tokens.tint.duration_ms,
            curve: tokens.tint.curve,
        },
        MotionRole::Reveal => ResolvedMotion::Spring(tokens.reveal),
        MotionRole::Float => ResolvedMotion::Spring(tokens.float.spring),
        MotionRole::Panel => ResolvedMotion::Spring(tokens.panel.snap_spring),
        MotionRole::Shift => ResolvedMotion::Duration {
            duration_ms: tokens.shift.duration_ms,
            curve: tokens.shift.curve,
        },
        MotionRole::Scroll => ResolvedMotion::Duration {
            duration_ms: tokens.scroll.duration_ms,
            curve: tokens.scroll.curve,
        },
        MotionRole::Caret => ResolvedMotion::Duration {
            duration_ms: tokens.caret.period_ms / 2,
            curve: Easing::Linear,
        },
    }
}
