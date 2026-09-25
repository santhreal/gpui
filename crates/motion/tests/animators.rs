//! Contracts of motion roles, reduced-motion resolution, keyed animators,
//! tweens, the tween store, and the motion policy.
//!
//! WHY: closes the class "a role runs the wrong motion, reduced motion leaves
//! a role animating, a retarget jumps, a finished motion keeps requesting
//! frames, or the duration scale is ignored". Roles are enumerated from
//! [`MotionRole::ALL`], so a new role fails the reduced-motion test until it
//! has a reduced variant. Not covered: how GPUI schedules the frames.

use std::time::Duration;

use motion::{
    Animator, AnimatorRegistry, Easing, MotionError, MotionModel, MotionPolicy, MotionRole,
    MotionSpec, MotionTokens, REDUCED_FADE_MS, ResolvedMotion, SpringConfig, Tween, TweenStore,
    presets, resolve_motion,
};

fn at(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

#[test]
fn every_role_has_a_name_and_a_static_or_brief_reduced_variant() {
    let tokens = MotionTokens::reference();
    for role in MotionRole::ALL {
        assert_eq!(MotionRole::from_name(role.name()), Ok(role));
        match resolve_motion(role, &tokens, true) {
            ResolvedMotion::Instant | ResolvedMotion::SteadyOn => {}
            ResolvedMotion::FadeOnly { duration_ms } => {
                assert_eq!(duration_ms, REDUCED_FADE_MS, "{}", role.name());
            }
            other => panic!("{} animates under reduced motion: {other:?}", role.name()),
        }
        let full = resolve_motion(role, &tokens, false);
        assert!(
            !matches!(full, ResolvedMotion::Instant | ResolvedMotion::SteadyOn),
            "{} has no motion at full motion: {full:?}",
            role.name()
        );
    }
    assert_eq!(
        MotionRole::from_name("wiggle"),
        Err(MotionError::UnknownRole("wiggle".to_string()))
    );
}

#[test]
fn reduced_resolutions_finish_within_their_fade() {
    let tokens = MotionTokens::reference();
    for role in MotionRole::ALL {
        let model = resolve_motion(role, &tokens, true).model();
        let mut animator = Animator::at_rest(0.0);
        animator.start(0.0, 0.0, 1.0, model, MotionPolicy::REDUCED, at(0));
        let sample = animator.update(at(u64::from(REDUCED_FADE_MS)));
        assert!(
            sample.at_rest,
            "{} still moving after the reduced fade",
            role.name()
        );
        assert_eq!(sample.value, 1.0);
    }
}

#[test]
fn duration_model_follows_its_curve_and_reports_the_curve_velocity() {
    let model = MotionTokens::reference().model(MotionRole::Scroll);
    let MotionModel::Duration(duration) = model else {
        panic!("scroll is a duration model: {model:?}");
    };
    let total = duration.duration_ms as f32 / 1000.0;
    let mut animator = Animator::at_rest(10.0);
    animator.start(10.0, 0.0, 110.0, model, MotionPolicy::DEFAULT, at(0));
    for ms in 1..duration.duration_ms {
        let sample = animator.sample(at(u64::from(ms)));
        let progress = ms as f32 / 1000.0 / total;
        let expected = 10.0 + 100.0 * duration.curve.eval(progress);
        assert!((sample.value - expected).abs() < 1e-3, "{ms} ms");
        let slope = 100.0 * duration.curve.slope(progress) / total;
        assert!((sample.velocity - slope).abs() < 1e-2, "{ms} ms velocity");
        assert!(!sample.at_rest);
    }
    let end = animator.update(at(u64::from(duration.duration_ms)));
    assert_eq!((end.value, end.velocity, end.at_rest), (110.0, 0.0, true));
    assert!(animator.is_at_rest());
}

#[test]
fn retarget_keeps_position_and_velocity_continuous() {
    let spring = MotionModel::Spring(SpringConfig::new(220.0, 26.0, 1.0));
    let mut animator = Animator::at_rest(0.0);
    animator.start(0.0, 0.0, 100.0, spring, MotionPolicy::DEFAULT, at(0));
    let before = animator.sample(at(80));
    animator.retarget(-50.0, spring, MotionPolicy::DEFAULT, at(80));
    let after = animator.sample(at(80));
    assert_eq!(before.value, after.value);
    assert_eq!(before.velocity, after.velocity);
    assert_eq!(animator.target(), -50.0);
    let next = animator.sample(at(81));
    assert!((next.value - before.value).abs() < before.velocity.abs() * 0.002 + 0.01);

    let target_before = animator.target();
    animator.retarget(-50.00001, spring, MotionPolicy::DEFAULT, at(90));
    assert_eq!(
        animator.target(),
        target_before,
        "sub-tolerance retarget restarted"
    );
}

#[test]
fn duration_scale_stretches_every_model() {
    let tokens = MotionTokens::reference();
    let slow = MotionPolicy::DEFAULT.with_duration_scale(4.0);
    for role in MotionRole::ALL {
        let model = tokens.model(role);
        let mut normal = Animator::at_rest(0.0);
        let mut scaled = Animator::at_rest(0.0);
        normal.start(0.0, 0.0, 1.0, model, MotionPolicy::DEFAULT, at(0));
        scaled.start(0.0, 0.0, 1.0, model, slow, at(0));
        for ms in [7_u64, 33, 90, 150] {
            let expected = normal.sample(at(ms));
            let actual = scaled.sample(at(ms * 4));
            assert!(
                (expected.value - actual.value).abs() < 1e-4,
                "{} at {ms} ms: {expected:?} vs {actual:?}",
                role.name()
            );
        }
    }
}

#[test]
fn policy_parses_and_clamps_duration_scales() {
    assert_eq!(MotionPolicy::parse_duration_scale(" 4 "), Some(4.0));
    assert_eq!(
        MotionPolicy::parse_duration_scale("0"),
        Some(MotionPolicy::MIN_DURATION_SCALE)
    );
    assert_eq!(
        MotionPolicy::parse_duration_scale("1e9"),
        Some(MotionPolicy::MAX_DURATION_SCALE)
    );
    assert_eq!(MotionPolicy::parse_duration_scale("fast"), None);
    assert_eq!(MotionPolicy::parse_duration_scale("NaN"), None);
    assert_eq!(
        MotionPolicy::DEFAULT
            .with_duration_scale(f32::NAN)
            .duration_scale(),
        1.0
    );
    let slow = MotionPolicy::DEFAULT.with_duration_scale(4.0);
    assert_eq!(
        slow.scale(presets::FADE_DURATION),
        Duration::from_millis(560)
    );
    assert_eq!(
        MotionPolicy::DEFAULT.scale(presets::ENTER_DURATION),
        Duration::from_millis(380)
    );
    assert_eq!(slow.authored_seconds(2.0), 0.5);
}

#[test]
fn caret_blinks_until_replaced_and_rests_when_reduced() {
    let tokens = MotionTokens::reference();
    let mut caret = Animator::at_rest(0.0);
    caret.start(
        0.0,
        0.0,
        1.0,
        tokens.model(MotionRole::Caret),
        MotionPolicy::DEFAULT,
        at(0),
    );
    let half = u64::from(tokens.caret.period_ms / 2);
    assert_eq!(caret.sample(at(1)).value, 1.0);
    assert_eq!(caret.sample(at(half + 1)).value, 0.0);
    assert_eq!(caret.sample(at(2 * half + 1)).value, 1.0);
    assert!(!caret.sample(at(60_000)).at_rest);

    let reduced = resolve_motion(MotionRole::Caret, &tokens, true);
    assert_eq!(reduced, ResolvedMotion::SteadyOn);
    caret.start(0.0, 0.0, 1.0, reduced.model(), MotionPolicy::REDUCED, at(0));
    assert_eq!(caret.update(at(0)).value, 1.0);
    assert!(caret.is_at_rest());
}

#[test]
fn registry_reports_activity_only_while_something_moves() {
    let tokens = MotionTokens::reference();
    let mut registry: AnimatorRegistry<(u8, u64), Duration> = AnimatorRegistry::new();
    assert!(!registry.any_active(at(0)));
    let tint = tokens.model(MotionRole::Tint);
    registry.animate((0, 1), 0.0, 1.0, tint, MotionPolicy::DEFAULT, at(0));
    registry.animate((1, 1), 5.0, 5.0, tint, MotionPolicy::DEFAULT, at(0));
    assert_eq!(registry.len(), 2);
    assert!(registry.is_at_rest(&(1, 1)));
    assert!(registry.any_active(at(10)));
    assert!(registry.any_active_where(at(10), |(surface, _)| *surface == 0));
    assert!(!registry.any_active_where(at(10), |(surface, _)| *surface == 1));
    assert_eq!(registry.update_all(at(10)), 1);
    assert_eq!(
        registry.update_all(at(u64::from(tokens.tint.duration_ms))),
        0
    );
    assert!(!registry.any_active(at(10_000)));
    registry.retain_active();
    assert!(registry.is_empty());

    let first = *registry.animate((2, 0), 0.0, 1.0, tint, MotionPolicy::DEFAULT, at(0));
    let same = *registry.animate((2, 0), 0.0, 1.0, tint, MotionPolicy::DEFAULT, at(30));
    assert_eq!(first.target(), same.target());
    assert!(same.value() > 0.0, "re-reading a key restarted it");
}

#[test]
fn tween_reads_its_target_when_reduced_and_retargets_continuously() {
    let spec = MotionSpec::new(200, Easing::EaseOut);
    let mut tween = Tween::new(0.0, 100.0, spec, at(0));
    assert_eq!(tween.sample(at(50), MotionPolicy::REDUCED), (100.0, true));
    let mid = tween.value(at(100), MotionPolicy::DEFAULT);
    assert!(mid > 0.0 && mid < 100.0);
    tween.retarget(0.0, at(100), MotionPolicy::DEFAULT);
    assert_eq!(tween.value(at(100), MotionPolicy::DEFAULT), mid);
    assert!(tween.is_active(at(299), MotionPolicy::DEFAULT));
    assert!(!tween.is_active(at(300), MotionPolicy::DEFAULT));
}

#[test]
fn store_drops_unread_keys_and_goes_idle_when_settled() {
    let spec = MotionSpec::new(100, Easing::Linear);
    let policy = MotionPolicy::DEFAULT;
    let mut store: TweenStore<&str, Duration> = TweenStore::new();
    assert_eq!(store.glide("width", 40.0, spec, false, policy, at(0)), 40.0);
    assert!(!store.end_frame(policy, at(0)));
    assert_eq!(
        store.glide("width", 140.0, spec, false, policy, at(0)),
        40.0
    );
    assert!((store.glide("width", 140.0, spec, false, policy, at(50)) - 90.0).abs() < 1e-3);
    assert!(store.end_frame(policy, at(50)));
    assert_eq!(
        store.glide("width", 140.0, spec, false, policy, at(100)),
        140.0
    );
    assert!(!store.end_frame(policy, at(100)));

    store.set_toggle("hover", true, spec, policy, at(100));
    assert!((store.toggle_value(&"hover", policy, at(150)) - 0.5).abs() < 1e-3);
    store.set_toggle("hover", false, spec, policy, at(150));
    assert!(store.end_frame(policy, at(150)));
    store.toggle_value(&"hover", policy, at(250));
    assert!(!store.end_frame(policy, at(250)));
    assert_eq!(store.toggle_value(&"hover", policy, at(260)), 0.0);

    assert!(!store.end_frame(policy, at(300)));
    assert!(!store.end_frame(policy, at(400)));
    assert!(store.is_empty(), "unread keys survived two frames");
    store.set_toggle("absent", false, spec, policy, at(400));
    assert!(store.is_empty());
}
