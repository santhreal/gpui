//! Frame driving for views: a frame is requested while a motion value moves
//! and none once every value is at rest. A value tracked within bounds scopes
//! the frame it needs to those bounds.

use std::hash::{BuildHasher, Hash};

use super::{
    Animator, AnimatorRegistry, FrameInstant, FrameSpring, MotionPolicy, Timestamp as _, TweenStore,
};
use crate::{App, Bounds, Pixels, Window};

/// Motion state that a [`MotionFrame`] brings to its instant.
pub trait Advance {
    /// Brings the motion to `frame` and returns whether it is still moving.
    fn advance(&mut self, frame: &MotionFrame) -> bool;
}

impl Advance for Animator<FrameInstant> {
    /// Updates the animator at the frame's instant.
    fn advance(&mut self, frame: &MotionFrame) -> bool {
        !self.update(frame.now).at_rest
    }
}

impl<K: Hash + Eq + Copy, S: BuildHasher + Default> Advance
    for AnimatorRegistry<K, FrameInstant, S>
{
    /// Updates every animator at the frame's instant.
    fn advance(&mut self, frame: &MotionFrame) -> bool {
        self.update_all(frame.now) > 0
    }
}

impl<K: Hash + Eq, S: BuildHasher + Default> Advance for TweenStore<K, FrameInstant, S> {
    /// Ends the store's frame (see [`TweenStore::end_frame`]). Advance the
    /// store after the frame has read its keys.
    fn advance(&mut self, frame: &MotionFrame) -> bool {
        self.end_frame(frame.policy, frame.now)
    }
}

/// Frame driving for one view.
///
/// A view holds one driver. Each render begins a [`MotionFrame`] with
/// [`Self::begin`], runs every motion value of the view through the frame, and
/// ends it with [`Self::end`]. The driver requests the next animation frame for
/// the view while a value is still moving and requests none once every value
/// is at rest. A frame whose moving values were all tracked within bounds
/// repaints only those bounds.
#[derive(Clone, Copy, Debug, Default)]
pub struct MotionDriver {
    previous: Option<FrameInstant>,
}

impl MotionDriver {
    /// Begins a frame at the current [`App::frame_instant`] under the current
    /// [`App::motion_policy`].
    pub fn begin(&mut self, cx: &App) -> MotionFrame {
        let now = cx.frame_instant();
        let delta_seconds = self
            .previous
            .map_or(0.0, |previous| now.seconds_since(previous));
        self.previous = Some(now);
        MotionFrame {
            now,
            delta_seconds,
            policy: cx.motion_policy(),
            moving: false,
            damage: None,
        }
    }

    /// Ends `frame` and returns whether a value the frame advanced is still
    /// moving.
    ///
    /// A moving value tracked with [`MotionFrame::track`] requests the next
    /// animation frame for the view that is rendering, which repaints the view.
    /// Otherwise, moving values tracked with [`MotionFrame::track_within`]
    /// declare the union of their bounds as damage and request the next
    /// animation frame scoped to the damage declared at paint, as
    /// [`Window::request_animation_frame_at_paint`] describes. With no value
    /// moving, requests nothing.
    pub fn end(&mut self, frame: MotionFrame, window: &mut Window) -> bool {
        if frame.moving {
            window.request_animation_frame();
            return true;
        }
        let Some(damage) = frame.damage else {
            self.previous = None;
            return false;
        };
        window.declare_damage(damage);
        window.request_animation_frame_at_paint();
        true
    }
}

/// One frame of a view's motion: the instant every value samples, the time
/// since the view's previous moving frame, the motion policy, whether a value
/// is still moving, and the bounds of the moving values tracked within bounds.
/// Created by [`MotionDriver::begin`].
#[derive(Clone, Copy, Debug)]
pub struct MotionFrame {
    now: FrameInstant,
    delta_seconds: f32,
    policy: MotionPolicy,
    moving: bool,
    damage: Option<Bounds<Pixels>>,
}

impl MotionFrame {
    /// The instant every value of the frame samples.
    pub fn now(&self) -> FrameInstant {
        self.now
    }

    /// Seconds since the view's previous frame, or zero for the first frame
    /// after the view came to rest.
    pub fn delta_seconds(&self) -> f32 {
        self.delta_seconds
    }

    /// The app's motion policy when the frame began.
    pub fn policy(&self) -> MotionPolicy {
        self.policy
    }

    /// Brings `motion` to this frame and records whether it is still moving.
    pub fn track(&mut self, motion: &mut impl Advance) {
        let moving = motion.advance(self);
        self.moving |= moving;
    }

    /// Brings `motion` to this frame. While it is still moving, records
    /// `bounds`, in window coordinates, as the region its next frame changes.
    /// Returns whether it is still moving.
    pub fn track_within(&mut self, motion: &mut impl Advance, bounds: Bounds<Pixels>) -> bool {
        let moving = motion.advance(self);
        if moving {
            self.damage = Some(self.damage.map_or(bounds, |damage| damage.union(&bounds)));
        }
        moving
    }

    /// Steps `spring` toward `target` by [`Self::delta_seconds`] converted to
    /// authored seconds, records whether it is still moving, and returns its
    /// value. A spring that comes to rest, or any spring under reduced motion,
    /// is placed on `target`.
    pub fn step_spring(&mut self, spring: &mut FrameSpring, target: f32) -> f32 {
        if self.policy.reduced() {
            spring.snap(target);
            return target;
        }
        spring.step(target, self.policy.authored_seconds(self.delta_seconds));
        if spring.is_at_rest(target) {
            spring.snap(target);
        } else {
            self.moving = true;
        }
        spring.value()
    }
}
