//! Frame pacing for X11 windows. A window runs frames while GPUI has frame
//! demand for it (a dirty view, a next-frame callback, a renderer that asks
//! to redraw), at no more than the display refresh rate. A window with no
//! demand, or one the X server does not show, registers no timer and costs
//! the event loop no wakeups.

use std::{
    cell::Cell,
    time::{Duration, Instant},
};

use calloop::{
    LoopHandle, RegistrationToken,
    timer::{TimeoutAction, Timer},
};
use x11rb::protocol::xproto;

use super::X11Client;

/// The frame interval of a display whose refresh rate RandR does not report.
pub(crate) const DEFAULT_FRAME_INTERVAL: Duration = Duration::from_micros(1_000_000 / 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// No frame timer is registered.
    Idle,
    /// A frame timer is registered and has not fired.
    Armed,
    /// The frame timer is running a frame.
    Ticking,
    /// Demand arrived while a frame ran: the timer fires again.
    Rearm,
}

/// The pacing state of one window, apart from the event loop. Each
/// transition returns the timer change the caller applies, so every
/// sequence of transitions is testable without an X server.
#[derive(Clone, Copy, Debug)]
struct Pacer {
    visible: bool,
    phase: Phase,
    interval: Option<Duration>,
    last_frame: Option<Instant>,
}

impl Pacer {
    const fn new() -> Self {
        Self {
            visible: false,
            phase: Phase::Idle,
            interval: None,
            last_frame: None,
        }
    }

    fn interval(&self) -> Duration {
        self.interval.unwrap_or(DEFAULT_FRAME_INTERVAL)
    }

    /// Records frame demand. Returns the deadline of a timer to register.
    fn wake(&mut self, now: Instant) -> Option<Instant> {
        if !self.visible {
            return None;
        }
        match self.phase {
            Phase::Idle => {
                self.phase = Phase::Armed;
                // Demand after an idle stretch draws at once. Demand within
                // an interval of the last frame waits the interval out, so
                // frames never outpace the display however fast demand
                // arrives.
                Some(
                    self.last_frame
                        .map_or(now, |last| (last + self.interval()).max(now)),
                )
            }
            Phase::Ticking => {
                self.phase = Phase::Rearm;
                None
            }
            Phase::Armed | Phase::Rearm => None,
        }
    }

    /// The X server shows the window. Returns the deadline of a timer to
    /// register: a window that becomes visible draws.
    fn show(&mut self, now: Instant) -> Option<Instant> {
        if self.visible {
            return None;
        }
        self.visible = true;
        self.wake(now)
    }

    /// The X server stops showing the window. Returns true when the
    /// registered timer is to be removed; a running frame drops its own
    /// timer when it ends.
    fn hide(&mut self) -> bool {
        self.visible = false;
        if self.phase == Phase::Armed {
            self.phase = Phase::Idle;
            true
        } else {
            false
        }
    }

    /// The timer `wake` or `show` returned could not be registered.
    fn disarm(&mut self) {
        if self.phase == Phase::Armed {
            self.phase = Phase::Idle;
        }
    }

    fn begin_frame(&mut self, now: Instant) {
        self.phase = Phase::Ticking;
        self.last_frame = Some(now);
    }

    /// Returns the next deadline of the timer that ran the frame for
    /// `deadline`, or `None` to drop that timer.
    fn end_frame(&mut self, deadline: Instant, now: Instant) -> Option<Instant> {
        if self.visible && self.phase == Phase::Rearm {
            self.phase = Phase::Armed;
            Some(next_deadline(deadline, now, self.interval()))
        } else {
            self.phase = Phase::Idle;
            None
        }
    }
}

/// The first point of the grid `deadline + k * interval` after `now`. A
/// frame that overran its interval skips the grid points it missed.
fn next_deadline(deadline: Instant, now: Instant, interval: Duration) -> Instant {
    let interval_ns = interval.as_nanos();
    if interval_ns == 0 {
        return now;
    }
    let elapsed_ns = now.saturating_duration_since(deadline).as_nanos();
    // In 1..=interval_ns; RandR intervals are far below u64::MAX nanoseconds.
    let until_next = interval_ns - elapsed_ns % interval_ns;
    now + Duration::from_nanos(u64::try_from(until_next).unwrap_or(u64::MAX))
}

/// A window's frame source: a one-shot calloop timer registered while the
/// window has frame demand, kept at the refresh interval while each frame
/// leaves demand behind, and dropped by the first frame that leaves none.
pub(crate) struct FrameLoop {
    x_window: xproto::Window,
    loop_handle: LoopHandle<'static, X11Client>,
    pacer: Cell<Pacer>,
    timer: Cell<Option<RegistrationToken>>,
}

impl FrameLoop {
    pub(crate) fn new(x_window: xproto::Window, loop_handle: LoopHandle<'static, X11Client>) -> Self {
        Self {
            x_window,
            loop_handle,
            pacer: Cell::new(Pacer::new()),
            timer: Cell::new(None),
        }
    }

    fn update<R>(&self, transition: impl FnOnce(&mut Pacer) -> R) -> R {
        let mut pacer = self.pacer.get();
        let result = transition(&mut pacer);
        self.pacer.set(pacer);
        result
    }

    /// Requests a frame. A window the X server does not show draws when it
    /// is next shown instead.
    pub(crate) fn wake(&self) {
        if let Some(deadline) = self.update(|pacer| pacer.wake(Instant::now())) {
            self.arm(deadline);
        }
    }

    /// Starts frames for a window the X server shows. `interval` returns the
    /// display's frame interval; it runs on the window's first show only.
    pub(crate) fn show(&self, interval: impl FnOnce() -> Duration) {
        if self.pacer.get().interval.is_none() {
            let interval = interval();
            self.update(|pacer| pacer.interval = Some(interval));
        }
        if let Some(deadline) = self.update(|pacer| pacer.show(Instant::now())) {
            self.arm(deadline);
        }
    }

    /// Stops frames for a window the X server does not show, or one that is
    /// being destroyed.
    pub(crate) fn hide(&self) {
        if self.update(Pacer::hide)
            && let Some(token) = self.timer.take()
        {
            self.loop_handle.remove(token);
        }
    }

    pub(crate) fn begin_frame(&self) {
        self.update(|pacer| pacer.begin_frame(Instant::now()));
    }

    /// Ends the frame the timer ran for `deadline` and returns what the
    /// timer does next.
    pub(crate) fn end_frame(&self, deadline: Instant) -> TimeoutAction {
        match self.update(|pacer| pacer.end_frame(deadline, Instant::now())) {
            Some(next) => TimeoutAction::ToInstant(next),
            None => {
                self.timer.set(None);
                TimeoutAction::Drop
            }
        }
    }

    fn arm(&self, deadline: Instant) {
        let x_window = self.x_window;
        match self.loop_handle.insert_source(
            Timer::from_deadline(deadline),
            move |deadline, _, client| client.run_frame(x_window, deadline),
        ) {
            Ok(token) => self.timer.set(Some(token)),
            Err(err) => {
                log::error!("X11: failed to schedule a frame: {}", err.error);
                self.update(Pacer::disarm);
            }
        }
    }
}

#[cfg(test)]
mod tests;
