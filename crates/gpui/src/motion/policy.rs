//! The app-wide motion policy.

use super::MotionPolicy;
use crate::{App, Global};

/// Storage of the app-wide policy. Private so that every change goes through
/// [`App::set_motion_policy`], which redraws the windows.
struct AppMotionPolicy(MotionPolicy);

impl Global for AppMotionPolicy {}

impl App {
    /// The motion policy of the app: reduced motion and the duration scale.
    /// [`MotionPolicy::DEFAULT`], full motion at authored speed, until
    /// [`App::set_motion_policy`] sets another.
    pub fn motion_policy(&self) -> MotionPolicy {
        self.try_global::<AppMotionPolicy>()
            .map_or(MotionPolicy::DEFAULT, |policy| policy.0)
    }

    /// Sets the motion policy of the app. A changed policy redraws every
    /// window.
    pub fn set_motion_policy(&mut self, policy: MotionPolicy) {
        if self.motion_policy() != policy {
            self.set_global(AppMotionPolicy(policy));
            self.refresh_windows();
        }
    }

    /// Whether reduced motion is on: non-essential animations, such as loading
    /// spinners, render a static state instead of animating. Reads the reduced
    /// flag of [`App::motion_policy`].
    pub fn reduce_motion(&self) -> bool {
        self.motion_policy().reduced()
    }

    /// Turns reduced motion on or off and keeps the duration scale of
    /// [`App::motion_policy`].
    pub fn set_reduce_motion(&mut self, reduce_motion: bool) {
        self.set_motion_policy(self.motion_policy().with_reduced(reduce_motion));
    }
}
