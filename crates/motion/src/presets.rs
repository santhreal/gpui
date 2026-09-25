//! The reference motion catalog: named transitions with their spans and
//! curves, the shared durations, and the settle spring curve.

use std::time::Duration;

use crate::{Easing, MotionSpec, SpringConfig, SpringCurve};

/// Short fade for hover, focus, and small state changes: 140 ms.
pub const FADE_DURATION: Duration = Duration::from_millis(140);
/// Entrance of menus, sheets, and larger surfaces: 380 ms.
pub const ENTER_DURATION: Duration = Duration::from_millis(380);

/// The settle spring: `k = 280`, `c = 30`, unit mass, damping ratio 0.896.
pub const SETTLE_SPRING: SpringConfig = SpringConfig::new(280.0, 30.0, 1.0);
/// [`SETTLE_SPRING`] released at rest over 550 ms, as an easing curve.
pub const SETTLE: SpringCurve = SpringCurve::over(SETTLE_SPRING, Duration::from_millis(550));

/// [`SETTLE`] at normalized time `t`, clamped to `[0, 1]`.
#[inline]
pub fn settle(t: f32) -> f32 {
    SETTLE.eval_clamped(t)
}

/// Entrance: 500 ms exponential-style arrival, paired with opacity 0 to 1 and a
/// 4 px rise.
pub const FADE_IN: MotionSpec = MotionSpec::new(500, Easing::EaseOutExpo);
/// Quick opacity fade: 150 ms `ease`.
pub const FADE_QUICK: MotionSpec = MotionSpec::new(150, Easing::Ease);
/// Popover entrance: 140 ms `ease`, paired with a 2 px drop.
pub const MENU_IN: MotionSpec = MotionSpec::new(140, Easing::Ease);
/// Popover exit: 100 ms `ease`.
pub const MENU_OUT: MotionSpec = MotionSpec::new(100, Easing::Ease);
/// Dialog entrance: 180 ms `ease`.
pub const DIALOG_IN: MotionSpec = MotionSpec::new(180, Easing::Ease);
/// Splash exit: 150 ms hold, then 500 ms `ease` fade with a 6 px lift.
pub const SPLASH_OUT: MotionSpec = MotionSpec::new(500, Easing::Ease).with_delay(150);
/// Width and height transitions: 200 ms `ease-out`.
pub const RESIZE: MotionSpec = MotionSpec::new(200, Easing::EaseOut);
/// Tab reorder slide: 150 ms `ease-out`.
pub const TAB_SLIDE: MotionSpec = MotionSpec::new(150, Easing::EaseOut);
/// Section collapse: 180 ms `ease-out`.
pub const COLLAPSE: MotionSpec = MotionSpec::new(180, Easing::EaseOut);
/// Disclosure chevron: 200 ms `ease`.
pub const CHEVRON: MotionSpec = MotionSpec::new(200, Easing::Ease);
/// Programmatic scroll: 500 ms `ease-in-out` over the whole distance.
pub const SCROLL_GLIDE: MotionSpec = MotionSpec::new(500, Easing::EaseInOut);
/// Hover color wash: 150 ms on the standard curve.
pub const HOVER_FADE: MotionSpec = MotionSpec::new(150, Easing::Standard);
/// Loader pulse period: 2400 ms.
pub const ZERON_PULSE: MotionSpec = MotionSpec::new(2400, Easing::Ease);
/// Gradient spinner wave period: 750 ms.
pub const GRADIENT_SPIN: MotionSpec = MotionSpec::new(750, Easing::Ease);

/// Every preset by name.
pub const PRESETS: [(&str, MotionSpec); 14] = [
    ("FADE_IN", FADE_IN),
    ("FADE_QUICK", FADE_QUICK),
    ("MENU_IN", MENU_IN),
    ("MENU_OUT", MENU_OUT),
    ("DIALOG_IN", DIALOG_IN),
    ("SPLASH_OUT", SPLASH_OUT),
    ("RESIZE", RESIZE),
    ("TAB_SLIDE", TAB_SLIDE),
    ("COLLAPSE", COLLAPSE),
    ("CHEVRON", CHEVRON),
    ("SCROLL_GLIDE", SCROLL_GLIDE),
    ("HOVER_FADE", HOVER_FADE),
    ("ZERON_PULSE", ZERON_PULSE),
    ("GRADIENT_SPIN", GRADIENT_SPIN),
];
