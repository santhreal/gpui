//! Color blending for transitions.

/// Blends two sRGB colors given as `[r, g, b, a]` in `[0, 1]`, interpolating
/// premultiplied components: `a = lerp(a0, a1, t)` and
/// `c = lerp(c0 * a0, c1 * a1, t) / a`.
///
/// A wash fading in from transparent keeps its hue instead of passing through
/// grey. `t` clamps to `[0, 1]`; `t <= 0` returns `from` and `t >= 1` returns
/// `to` unchanged. When both alphas are zero the result is `to` with zero
/// alpha.
#[inline]
pub fn mix_premultiplied(from: [f32; 4], to: [f32; 4], t: f32) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return from;
    }
    if t >= 1.0 {
        return to;
    }
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    let alpha = lerp(from[3], to[3]);
    if alpha <= f32::EPSILON {
        return [to[0], to[1], to[2], 0.0];
    }
    [
        lerp(from[0] * from[3], to[0] * to[3]) / alpha,
        lerp(from[1] * from[3], to[1] * to[3]) / alpha,
        lerp(from[2] * from[3], to[2] * to[3]) / alpha,
        alpha,
    ]
}
