//! Motion math for user interfaces: easing curves, springs, tweens, motion
//! roles, loader waveforms, and the reduced-motion policy.
//!
//! The crate reads no clock and allocates nothing per sample. Stateful types
//! take a caller-supplied [`Timestamp`], so every value in a frame samples one
//! instant and tests drive time directly. Rendering and frame scheduling are
//! the caller's concern; `gpui::motion` binds this crate to GPUI.
//!
//! # Modules
//!
//! - [`bezier`], [`easing`]: CSS cubic beziers and closed-form easing
//!   functions, `fn(f32) -> f32` over normalized time.
//! - [`spring`], [`frame`]: damped harmonic oscillators stepped in closed
//!   form, by elapsed time or by frame delta.
//! - [`spec`], [`store`]: duration-based transitions and a keyed store of
//!   them with frame liveness.
//! - [`role`], [`tokens`], [`animator`]: the seven motion roles, their
//!   parameters, reduced-motion resolution, and keyed animators.
//! - [`presets`]: the named transition catalog and shared durations.
//! - [`loaders`], [`stream`], [`fling`], [`color`]: loader waveforms,
//!   streaming text and scroll-follow math, gesture release, and color mixing.

pub mod animator;
pub mod bezier;
pub mod color;
pub mod easing;
mod error;
pub mod fling;
pub mod frame;
pub mod loaders;
pub mod presets;
pub mod role;
pub mod spec;
pub mod spring;
pub mod store;
pub mod stream;
mod time;
pub mod tokens;

pub use animator::{Animator, AnimatorRegistry, AnimatorSample};
pub use bezier::CubicBezier;
pub use color::mix_premultiplied;
pub use easing::{
    Easing, bounce, ease_in_out, ease_out_cubic, ease_out_quad, ease_out_quint, lerp, linear,
    pulsating_between, quadratic,
};
pub use error::MotionError;
pub use frame::{FrameSpring, MAX_FRAME_STEP_SECONDS, POINTER_SPRING};
pub use role::{
    DirectThenSpringModel, DurationModel, FlipModel, MotionModel, MotionRole, REDUCED_FADE_MS,
    ResolvedMotion, SpringFadeModel, TwoStepModel, resolve_motion,
};
pub use spec::{MotionSpec, Tween};
pub use spring::{
    REST_POSITION_TOLERANCE, REST_VELOCITY_TOLERANCE, SpringConfig, SpringCurve, SpringState,
};
pub use store::TweenStore;
pub use stream::StickSpring;
pub use time::{MotionPolicy, Timestamp};
pub use tokens::MotionTokens;
