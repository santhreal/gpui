//! A keyed store of eased values with frame-based liveness.
//!
//! Each key holds one transition. A caller reads keys while building a frame
//! and calls [`TweenStore::end_frame`] once per frame: keys nobody read during
//! the previous full frame are dropped (their element unmounted), and the
//! return value reports whether any transition is still moving, which is the
//! signal to request another frame. A store with nothing moving requests
//! nothing.

use std::{
    collections::HashMap,
    hash::{BuildHasher, Hash, RandomState},
};

use crate::{MotionPolicy, MotionSpec, Timestamp, easing::lerp};

#[derive(Debug, Clone, Copy)]
struct Entry<T> {
    origin: f32,
    target: f32,
    started: T,
    spec: MotionSpec,
    seen: u64,
    rest_when_absent: bool,
}

impl<T: Timestamp> Entry<T> {
    #[inline]
    fn sample(&self, now: T, policy: MotionPolicy) -> (f32, bool) {
        if self.origin == self.target || policy.reduced() {
            return (self.target, true);
        }
        let authored = policy.authored_seconds(now.seconds_since(self.started));
        let (progress, done) = self.spec.sample(authored);
        if done {
            (self.target, true)
        } else {
            (lerp(self.origin, self.target, progress), false)
        }
    }

    #[inline]
    fn restart(&mut self, target: f32, spec: MotionSpec, now: T, policy: MotionPolicy) {
        self.origin = self.sample(now, policy).0;
        self.target = target;
        self.started = now;
        self.spec = spec;
    }
}

/// Keyed eased values: chased targets, on/off toggles, and elapsed-time
/// fractions.
#[derive(Debug, Clone)]
pub struct TweenStore<K, T, S = RandomState> {
    entries: HashMap<K, Entry<T>, S>,
    frame: u64,
}

impl<K, T, S: Default> Default for TweenStore<K, T, S> {
    fn default() -> Self {
        Self {
            entries: HashMap::default(),
            frame: 0,
        }
    }
}

impl<K: Hash + Eq, T: Timestamp, S: BuildHasher + Default> TweenStore<K, T, S> {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// The value `key` shows at `now` while it travels to `target` along
    /// `spec`.
    ///
    /// The first read of a key returns `target`: a value that appears is drawn
    /// where it is. A changed target restarts the transition from the current
    /// value. `instant`, or reduced motion, sets the value to `target` without
    /// a transition.
    pub fn glide(
        &mut self,
        key: K,
        target: f32,
        spec: MotionSpec,
        instant: bool,
        policy: MotionPolicy,
        now: T,
    ) -> f32 {
        let frame = self.frame;
        let instant = instant || policy.reduced();
        let entry = self.entries.entry(key).or_insert(Entry {
            origin: target,
            target,
            started: now,
            spec,
            seen: frame,
            rest_when_absent: false,
        });
        entry.seen = frame;
        if instant {
            entry.origin = target;
            entry.target = target;
            return target;
        }
        if entry.target != target {
            entry.restart(target, spec, now, policy);
        }
        entry.sample(now, policy).0
    }

    /// The fraction of `spec` elapsed since `key` was first read, from 0 to 1
    /// along the spec's curve. Measures a wait, so reduced motion does not
    /// shorten it; the duration scale applies.
    pub fn since(&mut self, key: K, spec: MotionSpec, policy: MotionPolicy, now: T) -> f32 {
        let frame = self.frame;
        let entry = self.entries.entry(key).or_insert(Entry {
            origin: 0.0,
            target: 1.0,
            started: now,
            spec,
            seen: frame,
            rest_when_absent: false,
        });
        entry.seen = frame;
        entry.sample(now, policy.with_reduced(false)).0
    }

    /// Turns the toggle behind `key` on or off at `now`, transitioning along
    /// `spec` from its current value. An absent key reads as off, so turning
    /// an absent key off stores nothing. Reduced motion sets the endpoint
    /// without a transition.
    pub fn set_toggle(&mut self, key: K, on: bool, spec: MotionSpec, policy: MotionPolicy, now: T) {
        let target = if on { 1.0 } else { 0.0 };
        let frame = self.frame;
        match self.entries.get_mut(&key) {
            Some(entry) => {
                entry.restart(target, spec, now, policy);
                if policy.reduced() {
                    entry.origin = target;
                }
                entry.seen = frame;
            }
            None if on => {
                let origin = if policy.reduced() { target } else { 0.0 };
                self.entries.insert(
                    key,
                    Entry {
                        origin,
                        target,
                        started: now,
                        spec,
                        seen: frame,
                        rest_when_absent: true,
                    },
                );
            }
            None => {}
        }
    }

    /// The toggle value of `key` at `now`, 0 for an absent key. Marks the key
    /// as read this frame.
    pub fn toggle_value(&mut self, key: &K, policy: MotionPolicy, now: T) -> f32 {
        let frame = self.frame;
        match self.entries.get_mut(key) {
            Some(entry) => {
                entry.seen = frame;
                entry.sample(now, policy).0
            }
            None => 0.0,
        }
    }

    /// Ends a frame: drops keys not read during the previous full frame and
    /// toggles that settled off, then reports whether any value is still
    /// moving at `now`.
    pub fn end_frame(&mut self, policy: MotionPolicy, now: T) -> bool {
        self.frame += 1;
        let frame = self.frame;
        let mut active = false;
        self.entries.retain(|_, entry| {
            if entry.seen + 1 < frame {
                return false;
            }
            let settled = entry.sample(now, policy).1;
            active |= !settled;
            !(settled && entry.rest_when_absent && entry.target == 0.0)
        });
        active
    }

    /// Number of stored keys.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the store holds no keys.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drops every key.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}
