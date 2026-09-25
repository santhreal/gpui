//! Configure pacing for Wayland surfaces. An `xdg_toplevel`, `xdg_popup` or
//! `zwlr_layer_surface_v1` configure states the size and toplevel state a
//! surface takes; the `xdg_surface.configure` that follows carries the
//! serial the surface acks. During an interactive resize a window applies
//! one configure per frame. A resizing configure that arrives after one was
//! applied in the same frame waits with its serial, and the next frame
//! applies and acks the latest one before it renders.

use gpui::{Pixels, Size, Tiling};

/// The state from a toplevel, popup or layer-surface configure, applied at
/// the surface configure that follows it.
#[derive(Debug)]
pub(super) struct InProgressConfigure {
    pub size: Option<Size<Pixels>>,
    pub fullscreen: bool,
    pub maximized: bool,
    pub resizing: bool,
    pub tiling: Tiling,
}

/// A surface configure to apply and ack: the toplevel state it applies, if
/// any came with it, and the serial to ack.
#[derive(Debug)]
pub(super) struct Configure {
    pub toplevel: Option<InProgressConfigure>,
    pub serial: u32,
}

/// Limits interactive resizes to one applied configure per frame.
#[derive(Debug, Default)]
pub(super) struct ConfigureThrottle {
    /// Set when a resizing configure is applied, cleared when a frame starts.
    throttled: bool,
    /// The latest resizing configure that arrived while `throttled` was set.
    waiting: Option<Configure>,
}

impl ConfigureThrottle {
    /// Receives a surface configure with `serial` and the toplevel state
    /// that preceded it. Returns the configure to apply and ack now, or
    /// `None` when it waits for the next frame.
    ///
    /// A configure replaces a waiting one. A configure without toplevel
    /// state takes the waiting state over, since acking a serial acks every
    /// older serial and acking an older serial after a newer one is a
    /// protocol error.
    pub fn receive(
        &mut self,
        toplevel: Option<InProgressConfigure>,
        serial: u32,
    ) -> Option<Configure> {
        let toplevel = toplevel.or_else(|| self.waiting.take().and_then(|w| w.toplevel));
        match toplevel {
            Some(toplevel) if toplevel.resizing && self.throttled => {
                self.waiting = Some(Configure {
                    toplevel: Some(toplevel),
                    serial,
                });
                None
            }
            toplevel => {
                self.waiting = None;
                Some(self.applied(Configure { toplevel, serial }))
            }
        }
    }

    /// Starts a frame. Returns the waiting configure, which the frame applies
    /// and acks before it renders, so the frame draws the latest size and its
    /// commit carries the ack.
    pub fn frame(&mut self) -> Option<Configure> {
        self.throttled = false;
        self.waiting.take().map(|configure| self.applied(configure))
    }

    fn applied(&mut self, configure: Configure) -> Configure {
        if configure.toplevel.as_ref().is_some_and(|t| t.resizing) {
            self.throttled = true;
        }
        configure
    }
}

#[cfg(test)]
mod tests;
