//! Motion for streamed content: the text veil that fades appended chunks in,
//! the stick-to-bottom scroll spring, and the retained-fraction glide.
//!
//! Frame-based quantities are in 60 fps frames: `frames = elapsed_ms /`
//! [`FRAME_MS`].

/// Initial estimate of the gap between appends, in milliseconds.
pub const VEIL_EMA_SEED_MS: f32 = 160.0;
/// Shortest chunk fade, in milliseconds.
pub const VEIL_MIN_FADE_MS: f32 = 120.0;
/// Longest chunk fade, in milliseconds.
pub const VEIL_MAX_FADE_MS: f32 = 400.0;
/// Exponent of the veil dissolve: veil `(1 - p)^1.6`.
pub const VEIL_CURVE_POW: f32 = 1.6;
/// Longest append gap fed into the estimate, in milliseconds.
pub const VEIL_GAP_CLAMP_MS: f32 = 1000.0;

/// Text opacity at fade progress `p`: `1 - (1 - p)^1.6`, with `p` clamped to
/// `[0, 1]`.
#[inline]
pub fn veil_opacity(p: f32) -> f32 {
    1.0 - (1.0 - p.clamp(0.0, 1.0)).powf(VEIL_CURVE_POW)
}

/// Chunk fade duration for the append-gap estimate: `3 * ema`, clamped to
/// [`VEIL_MIN_FADE_MS`]..=[`VEIL_MAX_FADE_MS`].
#[inline]
pub fn veil_duration_ms(ema_ms: f32) -> f32 {
    (ema_ms * 3.0).clamp(VEIL_MIN_FADE_MS, VEIL_MAX_FADE_MS)
}

/// Fade speed multiplier with `active_chunks` fading at once: 1 up to two
/// chunks, plus 0.3 per chunk beyond two.
#[inline]
pub fn veil_boost(active_chunks: usize) -> f32 {
    1.0 + 0.3 * active_chunks.saturating_sub(2) as f32
}

/// The append-gap estimate after a new gap: `0.7 * ema + 0.3 * min(gap, 1000)`.
#[inline]
pub fn veil_ema_next(ema_ms: f32, gap_ms: f32) -> f32 {
    ema_ms * 0.7 + gap_ms.min(VEIL_GAP_CLAMP_MS) * 0.3
}

/// One 60 fps frame, in milliseconds.
pub const FRAME_MS: f32 = 1000.0 / 60.0;
/// Stick spring velocity damping per frame.
pub const STICK_DAMPING: f32 = 0.7;
/// Stick spring stiffness per frame.
pub const STICK_STIFFNESS: f32 = 0.05;
/// Stick spring mass.
pub const STICK_MASS: f32 = 1.25;
/// Largest frame gap a caller feeds into one step.
pub const STICK_MAX_CATCHUP_FRAMES: f32 = 8.0;
/// Weight of the newest target-growth sample in the growth estimate.
pub const STICK_GROWTH_EMA: f32 = 0.12;
/// Largest distance the chase point leads above the true bottom, in pixels.
pub const STICK_CHASE_MAX_LEAD: f32 = 32.0;
/// Distance from the bottom that counts as at the bottom, in pixels.
pub const STICK_AT_BOTTOM_PX: f32 = 2.0;
/// Distance from the bottom within which a user scroll re-attaches the
/// spring, in pixels.
pub const STICK_THRESHOLD_PX: f32 = 70.0;
/// Time the spring keeps running after landing, so a pause in a stream
/// resumes at speed, in milliseconds.
pub const STICK_SETTLE_GRACE_MS: u64 = 500;
/// Beyond this many viewports from the target, a scroll jumps to within this
/// distance and glides the rest.
pub const GLIDE_MAX_VIEWPORTS: f32 = 2.5;
/// Fraction of the remaining glide distance kept per frame.
pub const GLIDE_RETAIN: f32 = 0.85;
/// Remaining distance at which a glide snaps onto its target, in pixels.
pub const GLIDE_SNAP_PX: f32 = 1.0;

/// Converts elapsed milliseconds into 60 fps frames, capped at
/// [`STICK_MAX_CATCHUP_FRAMES`].
#[inline]
pub fn frames_elapsed(elapsed_ms: f32) -> f32 {
    (elapsed_ms / FRAME_MS).min(STICK_MAX_CATCHUP_FRAMES)
}

/// Fraction of the remaining distance a retained-fraction glide covers over
/// `frames`: `1 - GLIDE_RETAIN^frames`.
#[inline]
pub fn glide_fraction(frames: f32) -> f32 {
    1.0 - GLIDE_RETAIN.powf(frames)
}

/// The stick-to-bottom scroll spring.
///
/// Positions are scroll offsets in pixels, larger being closer to the bottom.
/// Per 60 fps sub-frame of length `h <= 1`:
///
/// ```text
/// diff = max(chase - pos, 0)
/// v   += h * ((DAMPING * v + STIFFNESS * diff) / MASS - v)
/// pos  = min(pos + (v + growth) * h, target)
/// ```
///
/// where `growth` is an exponential moving average of target growth per frame
/// and `chase = target - min(9 * growth, CHASE_MAX_LEAD)`. The position never
/// passes the target and snaps onto it within 0.5 px.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StickSpring {
    velocity: f32,
    target_velocity: f32,
    last_target: Option<f32>,
}

impl StickSpring {
    /// A parked spring.
    pub const fn new() -> Self {
        Self {
            velocity: 0.0,
            target_velocity: 0.0,
            last_target: None,
        }
    }

    /// Drops all state; the next step starts cold.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Whether both the spring velocity and the growth estimate are below
    /// 0.05 px per frame.
    pub fn is_idle(&self) -> bool {
        self.velocity < 0.05 && self.target_velocity < 0.05
    }

    /// The smoothed target growth, in pixels per frame.
    pub fn target_velocity(&self) -> f32 {
        self.target_velocity
    }

    /// Advances `frames` 60 fps frames from `pos` toward `target` and returns
    /// the new position. `frames` is clamped to
    /// `0..=`[`STICK_MAX_CATCHUP_FRAMES`] (NaN counts as 0), which bounds the
    /// sub-step loop.
    pub fn step(&mut self, mut pos: f32, target: f32, frames: f32) -> f32 {
        let mut frames = if frames.is_nan() {
            0.0
        } else {
            frames.clamp(0.0, STICK_MAX_CATCHUP_FRAMES)
        };
        let grew = self.last_target.map_or(0.0, |last| target - last);
        self.last_target = Some(target);
        if grew < -1.0 {
            self.target_velocity = 0.0;
        } else {
            let observed = grew.max(0.0) / frames.max(0.25);
            self.target_velocity += STICK_GROWTH_EMA * (observed - self.target_velocity);
        }
        let chase = target - (self.target_velocity * 9.0).min(STICK_CHASE_MAX_LEAD);
        let mut v = self.velocity;
        while frames > 0.0 {
            let h = frames.min(1.0);
            frames -= h;
            let diff = (chase - pos).max(0.0);
            v += h * ((STICK_DAMPING * v + STICK_STIFFNESS * diff) / STICK_MASS - v);
            pos = (pos + (v + self.target_velocity) * h).min(target);
        }
        self.velocity = v;
        if target - pos <= 0.5 { target } else { pos }
    }
}
