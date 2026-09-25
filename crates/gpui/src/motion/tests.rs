//! Frame driving of views through [`MotionDriver`] and the app motion policy.
//!
//! WHY: closes the classes "a view keeps requesting animation frames once its
//! motion is at rest", "a moving value stops receiving frames before it
//! arrives", and "the app motion policy does not reach a view's motion".
//! Every motion role is enumerated from [`MotionRole::ALL`] and driven through
//! both an [`Animator`] and an [`AnimatorRegistry`], beside a [`TweenStore`]
//! and a [`FrameSpring`], so a new role is covered without editing this file.
//! Not caught: a new [`Advance`] implementor that no [`Kind`] drives, and
//! frames requested by elements other than the driver.

use std::time::Duration;

use super::*;
use crate::{Context, IntoElement, Render, TestAppContext, Window, WindowHandle, div, px, size};

/// Clock time between simulated frames.
const FRAME: Duration = Duration::from_millis(16);
/// Frames after which a motion that still requests frames fails the test.
const MAX_FRAMES: u32 = 1000;
/// Renders at rest that must request no frame.
const IDLE_FRAMES: u32 = 30;
const KEY: u8 = 0;
const TARGET: f32 = 1.0;

/// A motion value type driven through a [`MotionFrame`].
#[derive(Clone, Copy, Debug)]
enum Kind {
    Animator(MotionRole),
    Registry(MotionRole),
    Tweens,
    Spring,
}

impl Kind {
    /// Every role through an animator and through a registry, then the tween
    /// store and the frame spring.
    fn all() -> impl Iterator<Item = Self> {
        MotionRole::ALL
            .into_iter()
            .map(Self::Animator)
            .chain(MotionRole::ALL.into_iter().map(Self::Registry))
            .chain([Self::Tweens, Self::Spring])
    }

    /// The span a reduced-motion run of this kind moves for: the length of the
    /// reduced resolution of its role, zero for the tween store and the frame
    /// spring, and `None` for a resolution with no fixed length.
    fn reduced_span(self) -> Option<Duration> {
        let role = match self {
            Self::Animator(role) | Self::Registry(role) => role,
            Self::Tweens | Self::Spring => return Some(Duration::ZERO),
        };
        match resolve_motion(role, &MotionTokens::reference(), true) {
            ResolvedMotion::Instant | ResolvedMotion::SteadyOn => Some(Duration::ZERO),
            ResolvedMotion::FadeOnly { duration_ms }
            | ResolvedMotion::Duration { duration_ms, .. } => {
                Some(Duration::from_millis(duration_ms.into()))
            }
            ResolvedMotion::Spring(_) => None,
        }
    }
}

/// A view that moves one value of `kind` from 0 to [`TARGET`] through a
/// [`MotionDriver`].
struct MotionView {
    kind: Kind,
    start: bool,
    renders: usize,
    driver: MotionDriver,
    animator: Animator<FrameInstant>,
    registry: AnimatorRegistry<u8, FrameInstant>,
    tweens: TweenStore<u8, FrameInstant>,
    tween_target: f32,
    spring: FrameSpring,
    spring_target: f32,
    value: f32,
}

impl MotionView {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            start: false,
            renders: 0,
            driver: MotionDriver::default(),
            animator: Animator::at_rest(0.0),
            registry: AnimatorRegistry::new(),
            tweens: TweenStore::new(),
            tween_target: 0.0,
            spring: FrameSpring::default(),
            spring_target: 0.0,
            value: 0.0,
        }
    }

    fn start_motion(&mut self, frame: &MotionFrame) {
        let policy = frame.policy();
        let resolve = |role| resolve_motion(role, &MotionTokens::reference(), policy.reduced());
        match self.kind {
            Kind::Animator(role) => {
                self.animator
                    .apply(TARGET, resolve(role), policy, frame.now());
            }
            Kind::Registry(role) => {
                self.registry
                    .animate(KEY, 0.0, TARGET, resolve(role).model(), policy, frame.now());
            }
            Kind::Tweens => self.tween_target = TARGET,
            Kind::Spring => self.spring_target = TARGET,
        }
    }
}

impl Render for MotionView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let mut frame = self.driver.begin(cx);
        if std::mem::take(&mut self.start) {
            self.start_motion(&frame);
        }
        self.value = match self.kind {
            Kind::Animator(_) => {
                frame.track(&mut self.animator);
                self.animator.value()
            }
            Kind::Registry(_) => {
                frame.track(&mut self.registry);
                self.registry.get(&KEY).map_or(0.0, Animator::value)
            }
            Kind::Tweens => {
                let value = self.tweens.glide(
                    KEY,
                    self.tween_target,
                    presets::HOVER_FADE,
                    false,
                    frame.policy(),
                    frame.now(),
                );
                frame.track(&mut self.tweens);
                value
            }
            Kind::Spring => frame.step_spring(&mut self.spring, self.spring_target),
        };
        self.driver.end(frame, window);
        div()
    }
}

/// What a view did while its motion ran to rest and then idled.
#[derive(Debug)]
struct Outcome {
    /// Frames the view requested while moving.
    requested: usize,
    /// Clock time from the start of the motion to the first render at rest.
    settled_at: Duration,
    /// The value at the first render at rest.
    value: f32,
    /// Frames requested by renders at rest.
    idle_requests: usize,
    /// The value after the renders at rest.
    idle_value: f32,
}

/// Delivers the frame callbacks the previous render requested and returns how
/// many there were.
fn simulate_next_frame(window: &WindowHandle<MotionView>, cx: &mut TestAppContext) -> usize {
    let requested = window
        .update(cx, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    cx.run_until_parked();
    requested
}

/// Opens a view of `kind` under `policy`, starts its motion, and steps frames
/// of [`FRAME`] until the view stops requesting them. Then renders the view
/// [`IDLE_FRAMES`] more times at rest.
fn run(kind: Kind, policy: MotionPolicy, cx: &mut TestAppContext) -> Result<Outcome, String> {
    cx.update(|cx| cx.set_motion_policy(policy));
    let window = cx.open_window(size(px(100.0), px(100.0)), move |_, _| {
        MotionView::new(kind)
    });
    cx.run_until_parked();
    window
        .update(cx, |view, _, cx| {
            view.start = true;
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    let mut requested = 0;
    let mut settled_at = None;
    for frame in 0..MAX_FRAMES {
        cx.executor().advance_clock(FRAME);
        let pending = simulate_next_frame(&window, cx);
        if pending == 0 {
            settled_at = Some(FRAME * frame);
            break;
        }
        requested += pending;
    }
    let value = window.update(cx, |view, _, _| view.value).unwrap();

    let mut idle_requests = 0;
    for _ in 0..IDLE_FRAMES {
        window.update(cx, |_, _, cx| cx.notify()).unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(FRAME);
        idle_requests += simulate_next_frame(&window, cx);
    }
    let idle_value = window.update(cx, |view, _, _| view.value).unwrap();
    window
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();

    let settled_at = settled_at.ok_or_else(|| {
        format!("{kind:?} under {policy:?} requested frames for {MAX_FRAMES} frames")
    })?;
    Ok(Outcome {
        requested,
        settled_at,
        value,
        idle_requests,
        idle_value,
    })
}

/// Records a failure when `outcome` did not end on [`TARGET`] or requested a
/// frame at rest.
fn check_rest(kind: Kind, outcome: &Outcome, failures: &mut Vec<String>) {
    if outcome.value != TARGET || outcome.idle_value != TARGET {
        failures.push(format!(
            "{kind:?} came to rest at {} and idled at {}",
            outcome.value, outcome.idle_value
        ));
    }
    if outcome.idle_requests != 0 {
        failures.push(format!(
            "{kind:?} requested {} frames at rest",
            outcome.idle_requests
        ));
    }
}

#[gpui::test]
fn every_kind_requests_frames_while_moving_and_none_at_rest(cx: &mut TestAppContext) {
    let mut failures = Vec::new();
    for kind in Kind::all() {
        match run(kind, MotionPolicy::DEFAULT, cx) {
            Err(failure) => failures.push(failure),
            Ok(outcome) => {
                if outcome.requested == 0 {
                    failures.push(format!("{kind:?} requested no frame while moving"));
                }
                check_rest(kind, &outcome, &mut failures);
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[gpui::test]
fn reduced_motion_settles_every_kind_within_its_reduced_span(cx: &mut TestAppContext) {
    let mut failures = Vec::new();
    for kind in Kind::all() {
        let Some(span) = kind.reduced_span() else {
            failures.push(format!("{kind:?} has no fixed span under reduced motion"));
            continue;
        };
        match run(kind, MotionPolicy::REDUCED, cx) {
            Err(failure) => failures.push(failure),
            Ok(outcome) => {
                // The first render at or past the span comes to rest; a
                // zero span comes to rest on the render that starts it.
                let latest = if span.is_zero() { span } else { span + FRAME };
                if outcome.settled_at < span || outcome.settled_at > latest {
                    failures.push(format!(
                        "{kind:?} came to rest after {:?}, outside {span:?}..={latest:?}",
                        outcome.settled_at
                    ));
                }
                check_rest(kind, &outcome, &mut failures);
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[gpui::test]
fn duration_scale_stretches_every_kind(cx: &mut TestAppContext) {
    const SCALE: u32 = 2;
    let slow_policy = MotionPolicy::DEFAULT.with_duration_scale(SCALE as f32);
    let mut failures = Vec::new();
    for kind in Kind::all() {
        let (authored, slow) = match (
            run(kind, MotionPolicy::DEFAULT, cx),
            run(kind, slow_policy, cx),
        ) {
            (Ok(authored), Ok(slow)) => (authored, slow),
            (authored, slow) => {
                failures.extend(authored.err());
                failures.extend(slow.err());
                continue;
            }
        };
        // A settle time `t` rests on the first frame at or past it, so the
        // authored run rests before `t + FRAME` and the slow run at or past
        // `SCALE * t`.
        if slow.settled_at + FRAME * SCALE < authored.settled_at * SCALE {
            failures.push(format!(
                "{kind:?} came to rest after {:?} at scale {SCALE} and {:?} at scale 1",
                slow.settled_at, authored.settled_at
            ));
        }
        check_rest(kind, &slow, &mut failures);
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[gpui::test]
fn reduced_motion_is_the_reduced_flag_of_the_app_motion_policy(cx: &mut TestAppContext) {
    let window = cx.open_window(size(px(100.0), px(100.0)), |_, _| {
        MotionView::new(Kind::Tweens)
    });
    cx.run_until_parked();
    let renders = |cx: &mut TestAppContext| window.update(cx, |view, _, _| view.renders).unwrap();
    let initial = renders(cx);
    let slow = MotionPolicy::DEFAULT.with_duration_scale(4.0);

    cx.update(|cx| {
        assert_eq!(cx.motion_policy(), MotionPolicy::DEFAULT);
        assert!(!cx.reduce_motion());
        cx.set_motion_policy(slow.with_reduced(true));
        assert!(cx.reduce_motion());
        cx.set_reduce_motion(false);
        assert_eq!(cx.motion_policy(), slow);
    });
    cx.run_until_parked();
    assert_eq!(renders(cx), initial + 1, "a changed policy redraws once");

    cx.update(|cx| {
        cx.set_motion_policy(slow);
        cx.set_reduce_motion(false);
    });
    cx.run_until_parked();
    assert_eq!(
        renders(cx),
        initial + 1,
        "an unchanged policy redraws nothing"
    );
}
