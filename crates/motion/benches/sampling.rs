//! Per-sample cost of the motion primitives, and of one frame of 1000
//! animated values.

use std::{hint::black_box, time::Duration};

use criterion::{Criterion, criterion_group, criterion_main};
use motion::{
    Animator, AnimatorRegistry, Easing, FrameSpring, MotionPolicy, MotionRole, MotionTokens,
    SpringConfig, presets,
};

const ELEMENTS: u64 = 1000;

fn curves(c: &mut Criterion) {
    let inputs: Vec<f32> = (0..=1000).map(|step| step as f32 / 1000.0).collect();
    let mut group = c.benchmark_group("curve_1001_samples");
    for easing in [
        Easing::Ease,
        Easing::EaseOutExpo,
        Easing::EaseInOut,
        Easing::Linear,
    ] {
        group.bench_function(easing.name(), |b| {
            b.iter(|| {
                inputs
                    .iter()
                    .map(|t| easing.eval(black_box(*t)))
                    .sum::<f32>()
            });
        });
    }
    group.bench_function("settle_spring", |b| {
        b.iter(|| {
            inputs
                .iter()
                .map(|t| presets::settle(black_box(*t)))
                .sum::<f32>()
        });
    });
    group.finish();
}

fn springs(c: &mut Criterion) {
    let config = SpringConfig::new(220.0, 26.0, 1.0);
    c.bench_function("spring_evaluate", |b| {
        b.iter(|| config.evaluate(black_box(0.0), black_box(12.0), 100.0, black_box(0.137)));
    });
    c.bench_function("frame_spring_step", |b| {
        let mut spring = FrameSpring::default();
        b.iter(|| spring.step(black_box(1.0), black_box(1.0 / 120.0)));
    });
}

fn frame_of_1000(c: &mut Criterion) {
    let tokens = MotionTokens::reference();
    let policy = MotionPolicy::DEFAULT;
    let mut group = c.benchmark_group("frame_1000_elements");
    for role in [MotionRole::Tint, MotionRole::Reveal, MotionRole::Shift] {
        let model = tokens.model(role);
        let mut registry: AnimatorRegistry<u64, Duration> = AnimatorRegistry::new();
        for key in 0..ELEMENTS {
            registry.animate(key, 0.0, 100.0 + key as f32, model, policy, Duration::ZERO);
        }
        let now = Duration::from_millis(60);
        group.bench_function(role.name(), |b| {
            b.iter(|| black_box(registry.update_all(black_box(now))));
        });
    }
    let mut animators: Vec<Animator<Duration>> = (0..ELEMENTS)
        .map(|key| {
            let mut animator = Animator::at_rest(0.0);
            animator.start(
                0.0,
                0.0,
                key as f32,
                tokens.model(MotionRole::Reveal),
                policy,
                Duration::ZERO,
            );
            animator
        })
        .collect();
    group.bench_function("reveal_slice", |b| {
        b.iter(|| {
            let now = black_box(Duration::from_millis(60));
            animators
                .iter_mut()
                .map(|animator| animator.update(now).value)
                .sum::<f32>()
        });
    });
    group.finish();
}

criterion_group!(benches, curves, springs, frame_of_1000);
criterion_main!(benches);
