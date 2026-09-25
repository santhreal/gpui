//! Contracts of the closed-form springs, the frame-delta spring, the settle
//! curve, and the scroll-follow spring.
//!
//! WHY: closes the class "spring math drifts from the damped oscillator, depends
//! on frame timing, loses velocity on retarget, or fails to terminate". The
//! analytic references are computed in f64 from the textbook solutions of
//! `x'' + 2 zeta w0 x' + w0^2 x = 0`, independent of the propagator under
//! test. Not covered: perceptual tuning of the stiffness and damping values.

use motion::{
    FrameSpring, MAX_FRAME_STEP_SECONDS, MotionError, POINTER_SPRING, SpringConfig, SpringCurve,
    SpringState, StickSpring, presets,
};

/// Displacement from target at `t` for a spring released at rest from unit
/// displacement, from the textbook solution for each damping regime.
fn analytic_displacement(config: SpringConfig, t: f64) -> f64 {
    let w0 = (f64::from(config.stiffness) / f64::from(config.mass)).sqrt();
    let zeta = f64::from(config.damping)
        / (2.0 * (f64::from(config.stiffness) * f64::from(config.mass)).sqrt());
    if zeta < 1.0 - 1e-4 {
        let wd = w0 * (1.0 - zeta * zeta).sqrt();
        (-zeta * w0 * t).exp() * ((wd * t).cos() + zeta * w0 / wd * (wd * t).sin())
    } else if zeta > 1.0 + 1e-4 {
        let root = (zeta * zeta - 1.0).sqrt();
        let r1 = -w0 * (zeta - root);
        let r2 = -w0 * (zeta + root);
        (r2 * (r1 * t).exp() - r1 * (r2 * t).exp()) / (r2 - r1)
    } else {
        (1.0 + w0 * t) * (-w0 * t).exp()
    }
}

const REGIMES: [(&str, SpringConfig); 4] = [
    ("underdamped", SpringConfig::new(280.0, 30.0, 1.0)),
    ("light", SpringConfig::new(300.0, 12.0, 1.0)),
    ("critical", POINTER_SPRING),
    ("overdamped", SpringConfig::new(100.0, 40.0, 1.0)),
];

#[test]
fn closed_form_matches_the_textbook_solution_in_every_regime() {
    for (name, config) in REGIMES {
        for step in 0..=1000 {
            let t = step as f32 / 1000.0;
            let state = config.evaluate(1.0, 0.0, 0.0, t);
            let expected = analytic_displacement(config, f64::from(t));
            assert!(
                (f64::from(state.position) - expected).abs() < 2e-5,
                "{name} at {t}: {} vs {expected}",
                state.position
            );
        }
    }
}

#[test]
fn stepping_in_frames_matches_one_step_over_the_same_time() {
    for (name, config) in REGIMES {
        for frame_seconds in [1.0 / 30.0, 1.0 / 60.0, 1.0 / 144.0] {
            let mut state = SpringState {
                position: 40.0,
                velocity: -300.0,
            };
            let frames = (0.6 / frame_seconds) as u32;
            for _ in 0..frames {
                state = config.step(state, 0.0, frame_seconds);
            }
            let direct = config.evaluate(40.0, -300.0, 0.0, frames as f32 * frame_seconds);
            assert!(
                (state.position - direct.position).abs() < 1e-3
                    && (state.velocity - direct.velocity).abs() < 1e-2,
                "{name} at {frame_seconds}: {state:?} vs {direct:?}"
            );
        }
    }
}

#[test]
fn settle_time_bounds_every_later_sample() {
    for (name, config) in REGIMES {
        let start = SpringState {
            position: 1.0,
            velocity: 4.0,
        };
        let epsilon = 0.001;
        let settle = config.settle_time(start, 0.0, epsilon).as_secs_f32();
        assert!(
            settle.is_finite() && settle > 0.0 && settle < 5.0,
            "{name}: {settle}"
        );
        for step in 0..=2000 {
            let t = settle + step as f32 / 1000.0;
            let state = config.evaluate(start.position, start.velocity, 0.0, t);
            assert!(
                config.is_settled(state, 0.0, epsilon * 1.01),
                "{name} unsettled at {t} after settle time {settle}: {state:?}"
            );
        }
    }
    assert_eq!(
        REGIMES[0].1.settle_time(SpringState::at(0.0), 0.0, 0.001),
        std::time::Duration::ZERO
    );
    let undamped = SpringConfig::new(100.0, 0.0, 1.0);
    assert_eq!(
        undamped.settle_time(SpringState::at(1.0), 0.0, 0.001),
        std::time::Duration::MAX
    );
}

#[test]
fn time_to_rest_terminates_for_springs_that_never_rest() {
    let undamped = SpringConfig::new(100.0, 0.0, 1.0);
    assert_eq!(undamped.time_to_rest(1.0, 0.0, 0.0, 1.0e9), None);
    assert_eq!(undamped.time_to_rest(1.0, 0.0, 0.0, f32::INFINITY), None);
    assert_eq!(undamped.time_to_rest(1.0, 0.0, 0.0, -1.0), None);
    let rest = REGIMES[0]
        .1
        .time_to_rest(1.0, 0.0, 0.0, 2.0)
        .expect("rests");
    assert!(rest > 0.3 && rest < 1.0, "{rest}");
    let after = REGIMES[0].1.evaluate(1.0, 0.0, 0.0, rest);
    assert!(after.is_at_rest(0.0));
}

#[test]
fn spring_validation_rejects_non_physical_parameters() {
    assert!(SpringConfig::try_new(280.0, 30.0, 1.0).is_ok());
    assert!(SpringConfig::try_new(280.0, 0.0, 1.0).is_ok());
    for (k, c, m) in [
        (0.0, 30.0, 1.0),
        (-1.0, 30.0, 1.0),
        (280.0, -1.0, 1.0),
        (280.0, 30.0, 0.0),
        (f32::NAN, 30.0, 1.0),
        (280.0, f32::INFINITY, 1.0),
    ] {
        let rejected = SpringConfig::try_new(k, c, m);
        assert!(
            matches!(
                rejected,
                Err(MotionError::InvalidSpringParameters { stiffness, damping, mass })
                    if stiffness.to_bits() == k.to_bits()
                        && damping.to_bits() == c.to_bits()
                        && mass.to_bits() == m.to_bits()
            ),
            "k={k} c={c} m={m}: {rejected:?}"
        );
    }
}

#[test]
fn settle_curve_matches_the_k280_c30_closed_form_over_550_ms() {
    assert_eq!(presets::settle(0.0), 0.0);
    assert_eq!(presets::settle(1.0), 1.0);
    assert!((presets::SETTLE.span().as_secs_f32() - 0.55).abs() < 1e-6);
    let (_, zeta) = presets::SETTLE_SPRING.canonical();
    assert!((zeta - 0.896_421).abs() < 1e-5, "{zeta}");
    for step in 0..550 {
        let t = step as f32 / 550.0;
        let expected = (1.0 - analytic_displacement(presets::SETTLE_SPRING, f64::from(t) * 0.55))
            .clamp(0.0, 1.0);
        let value = presets::settle(t);
        assert!((0.0..=1.0).contains(&value), "{t}: {value}");
        assert!(
            (f64::from(value) - expected).abs() < 2e-5,
            "{t}: {value} vs {expected}"
        );
    }
    let curve = SpringCurve::settling(presets::SETTLE_SPRING, 0.001);
    assert!(curve.span().as_secs_f32() > 0.3 && curve.span().as_secs_f32() < 1.0);
}

#[test]
fn frame_spring_glides_to_rest_without_overshoot() {
    let mut spring = FrameSpring::default();
    let mut peak = 0.0_f32;
    let mut frames = 0;
    while !spring.is_at_rest(1.0) {
        peak = peak.max(spring.step(1.0, 1.0 / 120.0));
        frames += 1;
        assert!(frames < 600, "not at rest after 5 s: {:?}", spring.state());
    }
    assert!(
        peak <= 1.0 + 1e-6,
        "critically damped spring overshot: {peak}"
    );
    assert!(frames > 60, "rested after only {frames} frames");
}

#[test]
fn frame_spring_reverses_mid_flight_with_its_velocity() {
    let mut spring = FrameSpring::default();
    for _ in 0..30 {
        spring.step(1.0, 1.0 / 120.0);
    }
    let before = spring.state();
    assert!(before.position > 0.1 && before.position < 1.0, "{before:?}");
    assert!(before.velocity > 0.5, "{before:?}");
    spring.step(0.0, 1e-4);
    assert!(
        (spring.velocity() - before.velocity).abs() < 0.05 * before.velocity,
        "velocity jumped on retarget: {before:?} -> {:?}",
        spring.state()
    );
    let mut frames = 0;
    while !spring.is_at_rest(0.0) {
        spring.step(0.0, 1.0 / 120.0);
        frames += 1;
        assert!(frames < 600);
    }
}

#[test]
fn frame_spring_clamps_stalled_and_invalid_deltas() {
    let mut stalled = FrameSpring::default();
    let mut capped = FrameSpring::default();
    stalled.step(1.0, 10.0);
    capped.step(1.0, MAX_FRAME_STEP_SECONDS);
    assert_eq!(stalled.state(), capped.state());
    assert!(stalled.value() > 0.0 && stalled.value() < 1.0);

    let mut spring = FrameSpring::default();
    for delta in [f32::NAN, f32::INFINITY, -1.0] {
        spring.step(1.0, delta);
        assert_eq!(spring.state(), SpringState::at(0.0), "delta {delta}");
    }
}

#[test]
fn frame_spring_is_independent_of_frame_split() {
    let mut coarse = FrameSpring::new(POINTER_SPRING, 0.0);
    let mut fine = FrameSpring::new(POINTER_SPRING, 0.0);
    for _ in 0..30 {
        coarse.step(1.0, 1.0 / 60.0);
        fine.step(1.0, 1.0 / 120.0);
        fine.step(1.0, 1.0 / 120.0);
    }
    assert!((coarse.value() - fine.value()).abs() < 1e-5);
    assert!((coarse.velocity() - fine.velocity()).abs() < 1e-4);
}

#[test]
fn step_ramp_follows_a_moving_target_at_its_steady_lag() {
    let config = POINTER_SPRING;
    let (w0, zeta) = config.canonical();
    let target_velocity = 200.0;
    let mut state = SpringState::at(0.0);
    let mut target = 0.0;
    for _ in 0..240 {
        state = config.step_ramp(state, target, target_velocity, 1.0 / 60.0);
        target += target_velocity / 60.0;
    }
    let lag = 2.0 * zeta * target_velocity / w0;
    assert!(
        ((target - state.position) - lag).abs() < 0.05,
        "{state:?} target {target}"
    );
    assert!((state.velocity - target_velocity).abs() < 0.5);
}

#[test]
fn stick_spring_never_passes_its_target_and_bounds_catch_up() {
    let mut spring = StickSpring::new();
    let mut pos = 0.0;
    let mut target = 0.0;
    for frame in 0..600 {
        if frame < 300 {
            target += 18.0;
        }
        pos = spring.step(pos, target, if frame % 7 == 0 { 3.5 } else { 1.0 });
        assert!(
            pos <= target,
            "passed target at frame {frame}: {pos} > {target}"
        );
    }
    assert_eq!(pos, target);
    assert!(spring.is_idle());

    let mut cold = StickSpring::new();
    let far = cold.step(0.0, 1.0e6, f32::MAX);
    let mut bounded = StickSpring::new();
    let capped = bounded.step(0.0, 1.0e6, motion::stream::STICK_MAX_CATCHUP_FRAMES);
    assert_eq!(far, capped);
    let mut nan = StickSpring::new();
    assert_eq!(nan.step(5.0, 100.0, f32::NAN), 5.0);
}
