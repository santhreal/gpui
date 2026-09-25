//! Parity of the curves, the preset catalog, and the loader waveforms with the
//! Comet motion catalog they were transcribed from.
//!
//! WHY: closes the class "a curve, preset, or loader waveform drifts from the
//! Comet reference". The `reference` module transcribes the Comet definitions:
//! its cubic-bezier solver, its preset list, and its loader math and tables.
//! Every preset of [`presets::PRESETS`] is paired with a reference row by name
//! and every reference row with a preset, so a preset added on either side
//! fails until it has a counterpart. Each pair is sampled at 1 ms steps and the
//! largest absolute difference must stay within [`MAX_ERROR`].
//! Not caught: changes to the Comet catalog after this transcription, and
//! motion Comet does not define (springs, roles, tweens, reduced motion).

use std::ops::RangeInclusive;

use motion::{Easing, loaders, presets};

/// Largest absolute difference allowed between a motion sample and the
/// reference sample at the same instant. The two solvers stop Newton
/// iteration at an `X` error of `1e-6`, which the steepest reference curve
/// (`y1 / x1 = 6.25`) turns into at most `1.25e-5` of output.
const MAX_ERROR: f32 = 5e-5;

/// Transcription of the Comet motion catalog.
mod reference {
    /// CSS `cubic-bezier(x1, y1, x2, y2)`, solved by Newton iteration with a
    /// bisection fallback.
    #[derive(Clone, Copy, Debug)]
    pub struct Bezier {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    }

    impl Bezier {
        const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
            Self { x1, y1, x2, y2 }
        }

        fn coefficients(a: f32, b: f32) -> (f32, f32, f32) {
            let c = 3.0 * a;
            let bb = 3.0 * (b - a) - c;
            let aa = 1.0 - c - bb;
            (aa, bb, c)
        }

        fn sample_x(&self, t: f32) -> f32 {
            let (a, b, c) = Self::coefficients(self.x1, self.x2);
            ((a * t + b) * t + c) * t
        }

        fn sample_y(&self, t: f32) -> f32 {
            let (a, b, c) = Self::coefficients(self.y1, self.y2);
            ((a * t + b) * t + c) * t
        }

        fn sample_x_derivative(&self, t: f32) -> f32 {
            let (a, b, c) = Self::coefficients(self.x1, self.x2);
            (3.0 * a * t + 2.0 * b) * t + c
        }

        fn solve_t_for_x(&self, x: f32) -> f32 {
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

        pub fn eval(&self, x: f32) -> f32 {
            if x <= 0.0 {
                return 0.0;
            }
            if x >= 1.0 {
                return 1.0;
            }
            self.sample_y(self.solve_t_for_x(x)).clamp(0.0, 1.0)
        }
    }

    pub const EASE_OUT_EXPO: Bezier = Bezier::new(0.16, 1.0, 0.3, 1.0);
    pub const EASE_OUT: Bezier = Bezier::new(0.0, 0.0, 0.58, 1.0);
    pub const EASE: Bezier = Bezier::new(0.25, 0.1, 0.25, 1.0);
    pub const EASE_RESORT: Bezier = Bezier::new(0.22, 1.0, 0.36, 1.0);
    pub const EASE_IN_OUT: Bezier = Bezier::new(0.42, 0.0, 0.58, 1.0);
    pub const EASE_TAILWIND: Bezier = Bezier::new(0.4, 0.0, 0.2, 1.0);

    /// A catalog entry: a delay of `delay_ms`, then `duration_ms` of `curve`.
    #[derive(Clone, Copy, Debug)]
    pub struct Preset {
        pub name: &'static str,
        pub duration_ms: u64,
        pub delay_ms: u64,
        pub curve: Bezier,
    }

    impl Preset {
        const fn new(name: &'static str, duration_ms: u64, curve: Bezier) -> Self {
            Self {
                name,
                duration_ms,
                delay_ms: 0,
                curve,
            }
        }

        const fn with_delay(mut self, delay_ms: u64) -> Self {
            self.delay_ms = delay_ms;
            self
        }

        pub const fn total_ms(&self) -> u64 {
            self.delay_ms + self.duration_ms
        }

        /// Eased progress for raw progress `raw_delta` across the timeline.
        pub fn progress(&self, raw_delta: f32) -> f32 {
            let total = (self.delay_ms + self.duration_ms) as f32;
            if total <= 0.0 || self.duration_ms == 0 {
                return 1.0;
            }
            let t = (raw_delta.clamp(0.0, 1.0) * total - self.delay_ms as f32)
                / self.duration_ms as f32;
            self.curve.eval(t.clamp(0.0, 1.0))
        }
    }

    pub const PRESETS: [Preset; 14] = [
        Preset::new("FADE_IN", 500, EASE_OUT_EXPO),
        Preset::new("FADE_QUICK", 150, EASE),
        Preset::new("MENU_IN", 140, EASE),
        Preset::new("MENU_OUT", 100, EASE),
        Preset::new("DIALOG_IN", 180, EASE),
        Preset::new("SPLASH_OUT", 500, EASE).with_delay(150),
        Preset::new("RESIZE", 200, EASE_OUT),
        Preset::new("TAB_SLIDE", 150, EASE_OUT),
        Preset::new("COLLAPSE", 180, EASE_OUT),
        Preset::new("CHEVRON", 200, EASE),
        Preset::new("SCROLL_GLIDE", 500, EASE_IN_OUT),
        Preset::new("HOVER_FADE", 150, EASE_TAILWIND),
        Preset::new("ZERON_PULSE", 2400, EASE),
        Preset::new("GRADIENT_SPIN", 750, EASE),
    ];

    pub const ZERON_PULSE_MS: u64 = 2_400;
    pub const GRADIENT_SPIN_MS: u64 = 750;
    pub const ZERON_CELLS: usize = 5;
    pub const MATRIX_SIDE: usize = 3;
    pub const PULSE_MIN_OPACITY: f32 = 0.08;
    pub const PULSE_MIN_SCALE: f32 = 0.9;
    pub const PULSE_STAGGER: f32 = 0.15 / 2.4;
    pub const GSPIN_ROW_TINTS: [u32; MATRIX_SIDE] = [0xB6D3EF, 0xEDB185, 0xF888A0];
    pub const GSPIN_DIM: f32 = 0.1;
    pub const MINI_RING: [[usize; 2]; 3] = [[0, 1], [5, 2], [4, 3]];
    pub const MINI_RING_LEN: f32 = 6.0;
    pub const MARK_SPREAD: f32 = 0.55;
    #[rustfmt::skip]
    pub const MARK_CELLS: [(f32, f32); 34] = [
        (0., 600.), (0., 720.), (240., 840.), (240., 720.), (120., 840.), (120., 600.), (240., 600.),
        (0., 480.), (0., 360.), (480., 840.), (480., 720.), (120., 360.), (120., 240.), (240., 360.),
        (600., 720.), (480., 600.), (360., 360.), (240., 240.), (600., 600.), (720., 600.), (720., 480.),
        (240., 120.), (600., 380.), (720., 240.), (720., 0.), (480., 240.), (480., 0.), (120., 480.),
        (240., 480.), (360., 840.), (360., 720.), (360., 600.), (360., 480.), (120., 720.),
    ];

    fn lerp(from: f32, to: f32, t: f32) -> f32 {
        from + (to - from) * t
    }

    /// Phase of a repeating timeline of `period` seconds after `elapsed`
    /// seconds.
    pub fn cycle_phase(elapsed: f32, period: f32) -> f32 {
        (elapsed / period).fract()
    }

    pub fn staggered_phase(raw_delta: f32, index: usize, stagger: f32) -> f32 {
        (raw_delta - index as f32 * stagger).rem_euclid(1.0)
    }

    pub fn pulse_wave(phase: f32) -> f32 {
        0.5 - 0.5 * (phase * std::f32::consts::TAU).cos()
    }

    pub fn pulse_opacity(phase: f32) -> f32 {
        PULSE_MIN_OPACITY + (1.0 - PULSE_MIN_OPACITY) * pulse_wave(phase)
    }

    pub fn pulse_scale(phase: f32) -> f32 {
        PULSE_MIN_SCALE + (1.0 - PULSE_MIN_SCALE) * pulse_wave(phase)
    }

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

    pub fn gspin_cell_phase(row: usize, col: usize) -> f32 {
        let centre = (MATRIX_SIDE as f32 - 1.0) / 2.0;
        let max = MATRIX_SIDE as f32 - 1.0 + centre;
        let d = MATRIX_SIDE as f32 - 1.0 - row as f32 + (col as f32 - centre).abs();
        if max == 0.0 { 0.0 } else { d / (max + 1.0) }
    }

    pub fn matrix_wave(raw_delta: f32, wave_index: usize, wave_count: usize) -> f32 {
        let count = wave_count.max(1) as f32;
        pulse_wave(staggered_phase(raw_delta, wave_index, 1.0 / count))
    }

    fn mark_cell_stagger(x: f32, y: f32) -> f32 {
        let t = (820.0 - x + y) / 1660.0;
        (1.0 - t) * MARK_SPREAD
    }

    pub fn mark_phase(delta: f32, x: f32, y: f32) -> f32 {
        (delta + mark_cell_stagger(x, y)).rem_euclid(1.0)
    }
}

/// The largest `|ours - theirs|` over the sample pairs `pair` returns for
/// every millisecond of `ms`. A NaN sample counts as an infinite difference.
fn max_error(ms: RangeInclusive<u64>, pair: impl Fn(u64) -> (f32, f32)) -> f32 {
    ms.map(|ms| {
        let (ours, theirs) = pair(ms);
        let error = (ours - theirs).abs();
        if error.is_nan() { f32::INFINITY } else { error }
    })
    .fold(0.0, f32::max)
}

/// Records a failure when `error` exceeds [`MAX_ERROR`].
fn check(what: String, error: f32, failures: &mut Vec<String>) {
    if error > MAX_ERROR {
        failures.push(format!("{what}: max abs error {error:e} > {MAX_ERROR:e}"));
    }
}

/// The phase of a repeating timeline `ms` milliseconds after a shared clock
/// started: on a period of `ours_ms` through [`loaders::cycle_phase`], and on
/// a period of `theirs_ms` through the reference.
fn phases(ours_ms: u64, theirs_ms: u64) -> impl Fn(u64) -> (f32, f32) + Copy {
    move |ms| {
        let elapsed = ms as f32 / 1000.0;
        (
            loaders::cycle_phase(elapsed, ours_ms as f32 / 1000.0),
            reference::cycle_phase(elapsed, theirs_ms as f32 / 1000.0),
        )
    }
}

#[test]
fn every_preset_matches_its_reference_timeline_at_every_millisecond() {
    let mut failures = Vec::new();
    for (name, spec) in presets::PRESETS {
        let Some(row) = reference::PRESETS.iter().find(|row| row.name == name) else {
            failures.push(format!("preset {name} has no reference row"));
            continue;
        };
        if (spec.duration_ms, spec.delay_ms) != (row.duration_ms, row.delay_ms) {
            failures.push(format!(
                "preset {name}: {} ms after a {} ms delay, reference {} ms after {} ms",
                spec.duration_ms, spec.delay_ms, row.duration_ms, row.delay_ms
            ));
        }
        let total = row.total_ms();
        let error = max_error(0..=total, |ms| {
            (
                spec.sample(ms as f32 / 1000.0).0,
                row.progress(ms as f32 / total as f32),
            )
        });
        check(format!("preset {name}"), error, &mut failures);
    }
    for row in reference::PRESETS {
        if !presets::PRESETS.iter().any(|(name, _)| *name == row.name) {
            failures.push(format!("reference preset {} has no preset", row.name));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn every_reference_curve_matches_its_named_easing() {
    let curves = [
        (Easing::EaseOutExpo, reference::EASE_OUT_EXPO),
        (Easing::EaseOut, reference::EASE_OUT),
        (Easing::Ease, reference::EASE),
        (Easing::EaseResort, reference::EASE_RESORT),
        (Easing::EaseInOut, reference::EASE_IN_OUT),
        (Easing::Standard, reference::EASE_TAILWIND),
    ];
    let mut failures = Vec::new();
    for (easing, curve) in curves {
        let error = max_error(0..=1000, |ms| {
            let t = ms as f32 / 1000.0;
            (easing.eval(t), curve.eval(t))
        });
        check(format!("curve {}", easing.name()), error, &mut failures);
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn loader_waveforms_match_the_reference_over_two_periods() {
    let mut failures = Vec::new();

    let pulse = phases(loaders::ZERON_PULSE_MS, reference::ZERON_PULSE_MS);
    let pulse_ms = 0..=2 * reference::ZERON_PULSE_MS;
    check(
        "pulse phase".to_string(),
        max_error(pulse_ms.clone(), pulse),
        &mut failures,
    );
    let waveforms: [(&str, fn(f32) -> f32, fn(f32) -> f32); 3] = [
        ("pulse_wave", loaders::pulse_wave, reference::pulse_wave),
        (
            "pulse_opacity",
            loaders::pulse_opacity,
            reference::pulse_opacity,
        ),
        ("pulse_scale", loaders::pulse_scale, reference::pulse_scale),
    ];
    for cell in 0..reference::ZERON_CELLS {
        for (name, ours, theirs) in waveforms {
            let error = max_error(pulse_ms.clone(), |ms| {
                let (our_phase, their_phase) = pulse(ms);
                (
                    ours(loaders::staggered_phase(
                        our_phase,
                        cell,
                        loaders::PULSE_STAGGER,
                    )),
                    theirs(reference::staggered_phase(
                        their_phase,
                        cell,
                        reference::PULSE_STAGGER,
                    )),
                )
            });
            check(format!("{name} of cell {cell}"), error, &mut failures);
        }
    }
    for (x, y) in reference::MARK_CELLS {
        let error = max_error(pulse_ms.clone(), |ms| {
            let (our_phase, their_phase) = pulse(ms);
            (
                loaders::mark_phase(our_phase, x, y),
                reference::mark_phase(their_phase, x, y),
            )
        });
        check(
            format!("mark_phase of cell ({x}, {y})"),
            error,
            &mut failures,
        );
    }

    let spin = phases(loaders::GRADIENT_SPIN_MS, reference::GRADIENT_SPIN_MS);
    let spin_ms = 0..=2 * reference::GRADIENT_SPIN_MS;
    let error = max_error(spin_ms.clone(), |ms| {
        let (our_phase, their_phase) = spin(ms);
        (
            loaders::gspin_opacity(our_phase, loaders::GSPIN_DIM),
            reference::gspin_opacity(their_phase, reference::GSPIN_DIM),
        )
    });
    check("gspin_opacity".to_string(), error, &mut failures);
    let diagonals = 2 * reference::MATRIX_SIDE - 1;
    for wave in 0..diagonals {
        let error = max_error(spin_ms.clone(), |ms| {
            let (our_phase, their_phase) = spin(ms);
            (
                loaders::matrix_wave(our_phase, wave, diagonals),
                reference::matrix_wave(their_phase, wave, diagonals),
            )
        });
        check(
            format!("matrix_wave of diagonal {wave}"),
            error,
            &mut failures,
        );
    }

    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn loader_tables_match_the_reference() {
    assert_eq!(loaders::ZERON_PULSE_MS, reference::ZERON_PULSE_MS);
    assert_eq!(loaders::GRADIENT_SPIN_MS, reference::GRADIENT_SPIN_MS);
    assert_eq!(loaders::ZERON_CELLS, reference::ZERON_CELLS);
    assert_eq!(loaders::MATRIX_SIDE, reference::MATRIX_SIDE);
    assert_eq!(loaders::GSPIN_ROW_TINTS, reference::GSPIN_ROW_TINTS);
    assert_eq!(loaders::MINI_RING, reference::MINI_RING);
    assert_eq!(loaders::MINI_RING_LEN, reference::MINI_RING_LEN);
    assert_eq!(loaders::MARK_CELLS, reference::MARK_CELLS);
    for row in 0..reference::MATRIX_SIDE {
        for col in 0..reference::MATRIX_SIDE {
            assert_eq!(
                loaders::gspin_cell_phase(row, col),
                reference::gspin_cell_phase(row, col),
                "gspin_cell_phase({row}, {col})"
            );
        }
    }
}
