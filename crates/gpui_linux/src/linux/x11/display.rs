use anyhow::Context as _;
use uuid::Uuid;
use x11rb::{connection::Connection as _, xcb_ffi::XCBConnection};

use gpui::{Bounds, DisplayId, Pixels, PlatformDisplay, Size, px};

#[derive(Debug)]
pub(crate) struct X11Display {
    x_screen_index: usize,
    bounds: Bounds<Pixels>,
    scale_factor: f32,
    uuid: Uuid,
}

impl X11Display {
    pub(crate) fn new(
        xcb: &XCBConnection,
        scale_factor: f32,
        x_screen_index: usize,
    ) -> anyhow::Result<Self> {
        let screen = xcb
            .setup()
            .roots
            .get(x_screen_index)
            .with_context(|| format!("No screen found with index {x_screen_index}"))?;
        Ok(Self::from_screen(
            x_screen_index,
            screen.width_in_pixels,
            screen.height_in_pixels,
            scale_factor,
        ))
    }

    /// The display of screen `x_screen_index`, `width` x `height` device
    /// pixels, under the client's single `scale_factor`.
    fn from_screen(x_screen_index: usize, width: u16, height: u16, scale_factor: f32) -> Self {
        Self {
            x_screen_index,
            bounds: Bounds {
                origin: Default::default(),
                size: Size {
                    width: px(width as f32 / scale_factor),
                    height: px(height as f32 / scale_factor),
                },
            },
            scale_factor,
            uuid: Uuid::from_bytes([0; 16]),
        }
    }
}

impl PlatformDisplay for X11Display {
    fn id(&self) -> DisplayId {
        DisplayId::new(self.x_screen_index as u64)
    }

    fn uuid(&self) -> anyhow::Result<Uuid> {
        Ok(self.uuid)
    }

    fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    fn scale_factor(&self) -> Option<f32> {
        Some(self.scale_factor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// X11 fixes one scale for every window, and the display reports its
    /// bounds divided by it. A caller that converts root pixels to logical
    /// pixels before any window exists reads the same scale from the
    /// display, and dividing the root size by it gives the bounds back.
    #[test]
    fn display_reports_the_scale_its_bounds_are_divided_by() {
        for scale in [1.0_f32, 1.25, 1.5, 2.0, 3.0] {
            let display = X11Display::from_screen(1, 3840, 2160, scale);
            assert_eq!(display.scale_factor(), Some(scale));
            let bounds = display.bounds();
            assert_eq!(bounds.size.width, px(3840.0 / scale));
            assert_eq!(bounds.size.height, px(2160.0 / scale));
            assert_eq!(display.id(), DisplayId::new(1));
        }
    }
}
