//! Loader waveforms: pure functions of a phase in `[0, 1)`.
//!
//! A loader samples its phase from one shared clock (`elapsed / period`,
//! fractional part), so every instance on screen stays phase-locked, and the
//! same phase gives the same frame whether it came from a frame delta or wall
//! time.

use std::f32::consts::TAU;

use crate::easing::lerp;

/// Wave loader pulse period, in milliseconds.
pub const ZERON_PULSE_MS: u64 = 2_400;
/// Gradient matrix spinner wave period, in milliseconds.
pub const GRADIENT_SPIN_MS: u64 = 750;
/// Cells in the wave loader.
pub const ZERON_CELLS: usize = 5;
/// Side length of the gradient spinner matrix.
pub const MATRIX_SIDE: usize = 3;
/// Opacity a wave-loader cell rests at between pulses.
pub const PULSE_MIN_OPACITY: f32 = 0.08;
/// Scale a wave-loader cell rests at between pulses.
pub const PULSE_MIN_SCALE: f32 = 0.9;
/// Per-cell stagger as a fraction of the pulse period: 150 ms of 2400 ms.
pub const PULSE_STAGGER: f32 = 0.15 / 2.4;
/// Per-row RGB tints of the gradient spinner, top to bottom: blue, amber,
/// pink.
pub const GSPIN_ROW_TINTS: [u32; MATRIX_SIDE] = [0xB6D3EF, 0xEDB185, 0xF888A0];
/// Opacity a gradient-spinner cell rests at between pulses.
pub const GSPIN_DIM: f32 = 0.1;
/// Clockwise ring position of each `(row, col)` cell of the 2x3 mini spinner,
/// starting top-left: (0,0), (0,1), (1,1), (2,1), (2,0), (1,0).
pub const MINI_RING: [[usize; 2]; 3] = [[0, 1], [5, 2], [4, 3]];
/// Cells in the mini spinner's ring.
pub const MINI_RING_LEN: f32 = 6.0;
/// Fraction of the pulse cycle the mark's light sweep occupies.
pub const MARK_SPREAD: f32 = 0.55;

/// `[x, y]` of each 100x100 cell of the mark on its 820x940 canvas.
#[rustfmt::skip]
pub const MARK_CELLS: [(f32, f32); 34] = [
    (0., 600.), (0., 720.), (240., 840.), (240., 720.), (120., 840.), (120., 600.), (240., 600.),
    (0., 480.), (0., 360.), (480., 840.), (480., 720.), (120., 360.), (120., 240.), (240., 360.),
    (600., 720.), (480., 600.), (360., 360.), (240., 240.), (600., 600.), (720., 600.), (720., 480.),
    (240., 120.), (600., 380.), (720., 240.), (720., 0.), (480., 240.), (480., 0.), (120., 480.),
    (240., 480.), (360., 840.), (360., 720.), (360., 600.), (360., 480.), (120., 720.),
];

/// Phase of cell `index` given the loader's raw phase: `raw - index * stagger`
/// wrapped into `[0, 1)`.
#[inline]
pub fn staggered_phase(raw_delta: f32, index: usize, stagger: f32) -> f32 {
    (raw_delta - index as f32 * stagger).rem_euclid(1.0)
}

/// Cosine pulse: 0 at phase 0, 1 at phase 0.5, 0 at phase 1.
#[inline]
pub fn pulse_wave(phase: f32) -> f32 {
    0.5 - 0.5 * (phase * TAU).cos()
}

/// Wave-loader cell opacity: [`PULSE_MIN_OPACITY`] to 1 and back.
#[inline]
pub fn pulse_opacity(phase: f32) -> f32 {
    PULSE_MIN_OPACITY + (1.0 - PULSE_MIN_OPACITY) * pulse_wave(phase)
}

/// Wave-loader cell scale: [`PULSE_MIN_SCALE`] to 1 and back.
#[inline]
pub fn pulse_scale(phase: f32) -> f32 {
    PULSE_MIN_SCALE + (1.0 - PULSE_MIN_SCALE) * pulse_wave(phase)
}

/// Gradient-spinner cell opacity at local phase `t`: 1 at the cycle start,
/// linear down to `dim` by 45 %, `dim` until 92 %, linear back to 1.
#[inline]
pub fn gspin_opacity(t: f32, dim: f32) -> f32 {
    let t = t.rem_euclid(1.0);
    if t < 0.45 {
        lerp(1.0, dim, t / 0.45)
    } else if t < 0.92 {
        dim
    } else {
        lerp(dim, 1.0, (t - 0.92) / 0.08)
    }
}

/// Phase offset of cell `(row, col)` in the gradient spinner. The pulse
/// enters at the bottom edge and converges on the top-centre cell.
#[inline]
pub fn gspin_cell_phase(row: usize, col: usize) -> f32 {
    let centre = (MATRIX_SIDE as f32 - 1.0) / 2.0;
    let max = MATRIX_SIDE as f32 - 1.0 + centre;
    let d = MATRIX_SIDE as f32 - 1.0 - row as f32 + (col as f32 - centre).abs();
    if max == 0.0 { 0.0 } else { d / (max + 1.0) }
}

/// Phase added to the mark cell at `(x, y)`. The tail tip `(720, 0)` leads
/// by nearly [`MARK_SPREAD`] and the head `(0, 840)` trails at 0.
#[inline]
pub fn mark_cell_stagger(x: f32, y: f32) -> f32 {
    let t = (820.0 - x + y) / 1660.0;
    (1.0 - t) * MARK_SPREAD
}

/// Phase of the mark cell at `(x, y)` at loader phase `delta`.
#[inline]
pub fn mark_phase(delta: f32, x: f32, y: f32) -> f32 {
    (delta + mark_cell_stagger(x, y)).rem_euclid(1.0)
}

/// Intensity of diagonal `wave_index` of `wave_count` in the matrix wave at
/// raw phase `raw_delta`. The front crosses every diagonal once per period.
#[inline]
pub fn matrix_wave(raw_delta: f32, wave_index: usize, wave_count: usize) -> f32 {
    let count = wave_count.max(1) as f32;
    pulse_wave(staggered_phase(raw_delta, wave_index, 1.0 / count))
}

/// Phase `[0, 1)` of a repeating timeline of `period_seconds` at
/// `elapsed_seconds` on a shared clock. Returns 0 for a non-positive period.
#[inline]
pub fn cycle_phase(elapsed_seconds: f32, period_seconds: f32) -> f32 {
    if period_seconds > 0.0 {
        (elapsed_seconds / period_seconds).fract()
    } else {
        0.0
    }
}
