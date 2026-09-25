//! Release velocity of a drag and the distance it throws.
//!
//! During a drag the pointer places the content every frame. On release the
//! content keeps travelling [`fling_distance`] pixels, carried by a
//! duration-based transition on an overshooting curve.

/// Fastest believable pointer rate, in pixels per millisecond. A faster
/// sample is a pointer jump (window drag, warp) and is clamped.
pub const FLING_MAX_RATE: f32 = 6.0;
/// Weight of the newest sample in the running velocity.
pub const FLING_BLEND: f32 = 0.4;
/// Gap, in milliseconds, that separates two movements. A sample after such a
/// gap replaces the running velocity; a release after it throws nothing.
pub const FLING_STALE_MS: f32 = 90.0;
/// Slowest rate, in pixels per millisecond, that counts as a throw.
pub const FLING_MIN_RATE: f32 = 0.05;
/// Milliseconds of travel at the release rate that a throw covers.
pub const FLING_COAST_MS: f32 = 220.0;

/// The running velocity, in pixels per millisecond, after a pointer sample
/// that moved `delta_px` over `dt_ms`.
///
/// A sample with no elapsed time leaves the velocity unchanged. A sample after
/// a gap of [`FLING_STALE_MS`] or more replaces it; otherwise the clamped rate
/// blends in with weight [`FLING_BLEND`].
#[inline]
pub fn sample_velocity(previous: f32, delta_px: f32, dt_ms: f32) -> f32 {
    if dt_ms.is_nan() || dt_ms <= 0.0 {
        return previous;
    }
    let raw = (delta_px / dt_ms).clamp(-FLING_MAX_RATE, FLING_MAX_RATE);
    if dt_ms >= FLING_STALE_MS {
        return raw;
    }
    previous * (1.0 - FLING_BLEND) + raw * FLING_BLEND
}

/// Signed distance, in pixels, a release at `velocity` throws when its last
/// sample is `age_ms` old.
///
/// Returns 0 under reduced motion, for a stale last sample, for a rate under
/// [`FLING_MIN_RATE`], and for a non-finite velocity.
#[inline]
pub fn fling_distance(velocity: f32, age_ms: f32, reduced: bool) -> f32 {
    if reduced
        || age_ms >= FLING_STALE_MS
        || velocity.abs() < FLING_MIN_RATE
        || !velocity.is_finite()
    {
        return 0.0;
    }
    velocity * FLING_COAST_MS
}
