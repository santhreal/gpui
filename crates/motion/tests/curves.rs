//! Contracts of the easing curves, cubic beziers, and the preset catalog.
//!
//! WHY: closes the class "a curve leaves its range, misses an endpoint, or
//! drifts from its analytic definition". Every named curve and every preset is
//! enumerated from the crate's own tables, so a new variant is covered without
//! editing this file. Not covered: how a curve looks on screen.

use motion::{
    CubicBezier, Easing, MotionError, MotionSpec, bezier, ease_in_out, ease_out_cubic,
    ease_out_quad, ease_out_quint, presets,
};

/// Samples `0..=1` at 1 ms resolution over a one-second span.
fn unit_samples() -> impl Iterator<Item = f32> {
    (0..=1000).map(|step| step as f32 / 1000.0)
}

#[test]
fn every_named_curve_meets_its_endpoints_and_stays_in_its_bounds() {
    for easing in Easing::NAMED {
        assert_eq!(easing.eval(0.0), 0.0, "{} at 0", easing.name());
        assert_eq!(easing.eval(1.0), 1.0, "{} at 1", easing.name());
        assert_eq!(easing.eval(-3.0), 0.0, "{} below 0", easing.name());
        assert_eq!(easing.eval(7.0), 1.0, "{} above 1", easing.name());
        let (floor, ceiling) = easing.bezier().map_or((0.0, 1.0), |curve| curve.bounds());
        for t in unit_samples() {
            let value = easing.eval(t);
            assert!(
                value.is_finite() && (floor..=ceiling).contains(&value),
                "{} at {t} = {value}, outside [{floor}, {ceiling}]",
                easing.name()
            );
        }
    }
}

#[test]
fn curves_inside_the_unit_square_never_leave_zero_to_one() {
    for easing in Easing::NAMED {
        let Some(curve) = easing.bezier() else {
            continue;
        };
        if curve.overshoots() {
            continue;
        }
        for t in unit_samples() {
            let value = curve.eval(t);
            assert!(
                (0.0..=1.0).contains(&value),
                "{} at {t} = {value}",
                easing.name()
            );
        }
    }
}

#[test]
fn curve_names_round_trip_and_unknown_names_fail_with_the_name() {
    assert_eq!(Easing::NAMED.len(), Easing::NAMES.len());
    for (easing, name) in Easing::NAMED.into_iter().zip(Easing::NAMES) {
        assert_eq!(easing.name(), name);
        assert_eq!(Easing::from_name(name), Ok(easing));
    }
    assert_eq!(
        Easing::from_name("wobble"),
        Err(MotionError::UnknownCurve("wobble".to_string()))
    );
    let message = Easing::from_name("wobble").unwrap_err().to_string();
    assert!(
        message.contains("'wobble'") && message.contains("ease_out_expo"),
        "{message}"
    );
}

#[test]
fn bezier_solver_inverts_x_to_one_part_in_a_million() {
    for easing in Easing::NAMED {
        let Some(curve) = easing.bezier() else {
            continue;
        };
        for x in unit_samples() {
            let t = curve.solve_t_for_x(x);
            assert!(
                (curve.sample_x(t) - x).abs() < 2e-6,
                "{} x={x} t={t} X(t)={}",
                easing.name(),
                curve.sample_x(t)
            );
        }
    }
}

#[test]
fn slope_matches_a_central_difference_of_eval() {
    const H: f32 = 1e-3;
    for easing in Easing::NAMED {
        if easing == Easing::Step {
            continue;
        }
        for step in 10..=990 {
            let x = step as f32 / 1000.0;
            let numeric = (easing.eval(x + H) - easing.eval(x - H)) / (2.0 * H);
            let analytic = easing.slope(x);
            let tolerance = 2e-2 * numeric.abs().max(1.0);
            assert!(
                (numeric - analytic).abs() < tolerance,
                "{} slope at {x}: analytic {analytic}, numeric {numeric}",
                easing.name()
            );
        }
        assert_eq!(easing.slope(0.0), 0.0);
        assert_eq!(easing.slope(1.0), 0.0);
    }
}

#[test]
fn bezier_validation_rejects_non_functions_of_time() {
    assert!(CubicBezier::try_new(0.25, 0.1, 0.25, 1.0).is_ok());
    assert!(CubicBezier::try_new(0.34, 1.56, 0.64, 1.0).is_ok());
    for (x1, x2) in [
        (-0.1, 0.5),
        (0.5, 1.1),
        (f32::NAN, 0.5),
        (0.5, f32::INFINITY),
    ] {
        assert!(
            matches!(
                CubicBezier::try_new(x1, 0.0, x2, 1.0),
                Err(MotionError::InvalidBezierControlPoints { .. })
            ),
            "x1={x1} x2={x2}"
        );
    }
    assert!(CubicBezier::try_new(0.5, f32::NAN, 0.5, 1.0).is_err());
}

#[test]
fn overshooting_curve_passes_one_and_stays_within_its_handle() {
    let back = bezier::EASE_OUT_BACK;
    let peak = unit_samples().map(|t| back.eval(t)).fold(0.0_f32, f32::max);
    assert!(peak > 1.05 && peak <= 1.56, "peak {peak}");
}

#[test]
fn closed_form_ease_outs_match_their_polynomials() {
    for t in unit_samples() {
        let t64 = f64::from(t);
        let quad = 1.0 - (1.0 - t64).powi(2);
        let cubic = 1.0 - (1.0 - t64).powi(3);
        let quint = 1.0 - (1.0 - t64).powi(5);
        assert!(
            (f64::from(ease_out_quad(t)) - quad).abs() < 1e-6,
            "quad {t}"
        );
        assert!(
            (f64::from(ease_out_cubic(t)) - cubic).abs() < 1e-6,
            "cubic {t}"
        );
        assert!(
            (f64::from(ease_out_quint(t)) - quint).abs() < 1e-6,
            "quint {t}"
        );
        let in_out = ease_in_out(t);
        assert!((0.0..=1.0).contains(&in_out));
    }
    assert_eq!(ease_out_quad(-1.0), 0.0);
    assert_eq!(ease_out_quad(2.0), 1.0);
    assert_eq!(ease_out_cubic(2.0), 1.0);
    assert!((ease_in_out(0.5) - 0.5).abs() < 1e-6);
}

#[test]
fn every_preset_holds_through_its_delay_then_eases_to_one() {
    for (name, spec) in presets::PRESETS {
        assert_eq!(spec.progress(0.0), 0.0, "{name} at 0");
        assert_eq!(spec.progress(1.0), 1.0, "{name} at 1");
        let total = spec.total().as_secs_f32();
        let delay_fraction = spec.delay().as_secs_f32() / total;
        for raw in unit_samples() {
            let progress = spec.progress(raw);
            if raw <= delay_fraction {
                assert_eq!(progress, 0.0, "{name} during delay at {raw}");
            }
            let expected = spec.easing.eval(
                ((raw * total - spec.delay().as_secs_f32()) / spec.duration().as_secs_f32())
                    .clamp(0.0, 1.0),
            );
            assert!((progress - expected).abs() < 1e-5, "{name} at {raw}");
        }
        let (at_end, done) = spec.sample(total);
        assert_eq!((at_end, done), (1.0, true), "{name} sample at total");
        let (_, done_before) = spec.sample(total * 0.5);
        assert!(!done_before, "{name} finished early");
    }
}

#[test]
fn preset_names_are_unique_and_match_their_constants() {
    let mut names: Vec<_> = presets::PRESETS.iter().map(|(name, _)| *name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), presets::PRESETS.len());
    let lookup = |wanted: &str| {
        presets::PRESETS
            .iter()
            .find(|(name, _)| *name == wanted)
            .map(|(_, spec)| *spec)
    };
    assert_eq!(lookup("SPLASH_OUT"), Some(presets::SPLASH_OUT));
    assert_eq!(
        presets::SPLASH_OUT,
        MotionSpec::new(500, Easing::Ease).with_delay(150)
    );
    assert_eq!(
        lookup("HOVER_FADE"),
        Some(MotionSpec::new(150, Easing::Standard))
    );
}

#[test]
fn zero_length_spec_reads_one_and_finishes_after_its_delay() {
    let spec = MotionSpec::new(0, Easing::Ease).with_delay(100);
    assert_eq!(spec.progress(0.0), 1.0);
    assert_eq!(spec.sample(0.05), (1.0, false));
    assert_eq!(spec.sample(0.1), (1.0, true));
    assert_eq!(MotionSpec::new(0, Easing::Ease).sample(0.0), (1.0, true));
}
