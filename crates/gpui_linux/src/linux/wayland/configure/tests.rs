//! WHY: during an interactive resize a Wayland window applies one configure
//! per frame, and the compositor takes the size the window commits. The
//! class closed here is a configure the throttle loses: a resizing
//! configure acked without its size (the window stays at an older size than
//! the compositor set, and the resize ends short of the pointer), one
//! dropped unacked, a waiting configure that the next frame does not apply,
//! a serial acked after a newer one (a protocol error), a configure that
//! ends the resize held back, and more than one resize applied per frame.
//! The model check runs every sequence of configures and frames up to a
//! fixed length. Not covered: the window's scheduling of the frame that
//! serves a waiting configure, and the size arithmetic that follows.

use gpui::{Tiling, px, size};

use super::{Configure, ConfigureThrottle, InProgressConfigure};

/// The toplevel state of a configure, sized from its serial so every
/// configure in a sequence has a distinct size.
fn toplevel(serial: u32, resizing: bool) -> InProgressConfigure {
    InProgressConfigure {
        size: Some(size(px(serial as f32), px(serial as f32))),
        fullscreen: false,
        maximized: false,
        resizing,
        tiling: Tiling::default(),
    }
}

fn applied(configure: Option<Configure>) -> Option<(u32, Option<u32>)> {
    configure.map(|c| {
        let width = c.toplevel.and_then(|t| t.size).map(|s| f32::from(s.width) as u32);
        (c.serial, width)
    })
}

#[test]
fn a_resize_after_one_this_frame_waits_and_the_next_frame_applies_the_latest() {
    let mut throttle = ConfigureThrottle::default();
    assert_eq!(applied(throttle.receive(Some(toplevel(1, true)), 1)), Some((1, Some(1))));
    assert_eq!(applied(throttle.receive(Some(toplevel(2, true)), 2)), None);
    assert_eq!(applied(throttle.receive(Some(toplevel(3, true)), 3)), None);
    assert_eq!(applied(throttle.frame()), Some((3, Some(3))));
    // The frame applied a resize: the rest of that frame is throttled.
    assert_eq!(applied(throttle.receive(Some(toplevel(4, true)), 4)), None);
    assert_eq!(applied(throttle.frame()), Some((4, Some(4))));
    assert_eq!(applied(throttle.frame()), None);
}

#[test]
fn a_surface_configure_without_toplevel_state_carries_a_waiting_resize() {
    let mut throttle = ConfigureThrottle::default();
    throttle.receive(Some(toplevel(1, true)), 1);
    assert_eq!(applied(throttle.receive(Some(toplevel(2, true)), 2)), None);
    assert_eq!(applied(throttle.receive(None, 3)), None);
    assert_eq!(applied(throttle.frame()), Some((3, Some(2))));
}

#[test]
fn a_configure_that_ends_the_resize_applies_at_once_and_discards_the_waiting_one() {
    let mut throttle = ConfigureThrottle::default();
    throttle.receive(Some(toplevel(1, true)), 1);
    assert_eq!(applied(throttle.receive(Some(toplevel(2, true)), 2)), None);
    assert_eq!(applied(throttle.receive(Some(toplevel(3, false)), 3)), Some((3, Some(3))));
    assert_eq!(applied(throttle.frame()), None);
}

#[derive(Clone, Copy, Debug)]
enum Event {
    /// A surface configure with no toplevel state.
    Bare,
    /// A surface configure after a toplevel configure in the resizing state.
    Resizing,
    /// A surface configure after a toplevel configure outside a resize.
    Settled,
    Frame,
}

const EVENTS: [Event; 4] = [Event::Bare, Event::Resizing, Event::Settled, Event::Frame];

/// The surface as the compositor observes it.
#[derive(Default)]
struct Surface {
    last_received: Option<u32>,
    last_acked: Option<u32>,
    latest_size: Option<u32>,
    applied_size: Option<u32>,
    resizes_this_frame: u32,
}

impl Surface {
    fn apply(&mut self, configure: Configure) {
        if let Some(acked) = self.last_acked {
            assert!(configure.serial > acked, "serial {} acked after {acked}", configure.serial);
        }
        self.last_acked = Some(configure.serial);
        if let Some(t) = configure.toplevel {
            self.applied_size = t.size.map(|s| f32::from(s.width) as u32);
            if t.resizing {
                self.resizes_this_frame += 1;
                assert!(self.resizes_this_frame <= 1, "two resizes applied in one frame");
            }
        }
    }

    /// Every configure received so far is acked and its latest size applied.
    fn assert_current(&self, sequence: &[Event], step: usize) {
        assert_eq!(
            self.last_acked, self.last_received,
            "{sequence:?} step {step}: unacked configure"
        );
        assert_eq!(
            self.applied_size, self.latest_size,
            "{sequence:?} step {step}: stale size"
        );
    }
}

fn run(sequence: &[Event]) {
    let mut throttle = ConfigureThrottle::default();
    let mut surface = Surface::default();
    let mut serial = 0;
    for (step, &event) in sequence.iter().enumerate() {
        if let Event::Frame = event {
            surface.resizes_this_frame = 0;
            if let Some(configure) = throttle.frame() {
                surface.apply(configure);
            }
            surface.assert_current(sequence, step);
            continue;
        }
        serial += 1;
        surface.last_received = Some(serial);
        let state = match event {
            Event::Bare => None,
            Event::Resizing => Some(toplevel(serial, true)),
            Event::Settled => Some(toplevel(serial, false)),
            Event::Frame => unreachable!(),
        };
        if state.is_some() {
            surface.latest_size = Some(serial);
        }
        let waits_for_frame = state.as_ref().is_some_and(|s| s.resizing)
            || (state.is_none() && surface.applied_size != surface.latest_size);
        match throttle.receive(state, serial) {
            Some(configure) => {
                surface.apply(configure);
                surface.assert_current(sequence, step);
            }
            None => {
                assert!(
                    waits_for_frame,
                    "{sequence:?} step {step}: a configure outside a resize waited"
                );
                assert_eq!(
                    surface.resizes_this_frame, 1,
                    "{sequence:?} step {step}: waited unthrottled"
                );
            }
        }
    }
}

fn sequences(len: usize, prefix: &mut Vec<Event>, visit: &mut impl FnMut(&[Event])) {
    visit(prefix);
    if prefix.len() == len {
        return;
    }
    for event in EVENTS {
        prefix.push(event);
        sequences(len, prefix, visit);
        prefix.pop();
    }
}

#[test]
fn every_sequence_acks_in_order_and_each_frame_applies_the_latest_configure() {
    let mut count = 0;
    sequences(8, &mut Vec::new(), &mut |sequence| {
        let mut ended = sequence.to_vec();
        ended.push(Event::Frame);
        run(&ended);
        count += 1;
    });
    assert_eq!(count, (0..=8).map(|n| EVENTS.len().pow(n)).sum::<usize>());
}
