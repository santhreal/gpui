//! Draws backdrop blurs and path clips with the DirectX renderer into the
//! render target of a hidden window, and compares every pixel with the math
//! documented on [`gpui::BackdropBlur`] and [`gpui::StartPathClip`], computed
//! on the CPU from a frame drawn without the primitive under test.

mod reference;

use super::DirectXRenderer;
use crate::DirectXDevices;
use gpui::{
    BackdropBlur, Bounds, ContentMask, Corners, DevicePixels, Hsla, Quad, ScaledPixels, Scene,
    WindowBackgroundAppearance, black, hsla, point, size, white,
};
use gpui_util::ResultExt;
use reference::{Image, Outline, Rgba, composite, expected_blur, over, rgba};
use windows::{
    Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
        },
    },
    core::w,
};

/// Width and height of the render target, in device pixels.
const SIZE: i32 = 100;
/// Largest difference, in 8-bit steps, between a drawn channel and the
/// reference.
const TOLERANCE: f32 = 3.0;
/// Distance from a clip outline within which a pixel is antialiased.
const EDGE: f32 = 1.5;

struct HiddenWindow(HWND);

impl Drop for HiddenWindow {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.0) }.log_err();
    }
}

/// A renderer drawing into a `SIZE` square swap chain of a hidden window.
struct Target {
    renderer: DirectXRenderer,
    _window: HiddenWindow,
}

impl Target {
    fn new() -> Self {
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                None,
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                SIZE,
                SIZE,
                None,
                None,
                None,
                None,
            )
        }
        .map(HiddenWindow)
        .expect("creating a hidden window");
        let devices = DirectXDevices::new().expect("creating a Direct3D device");
        let mut renderer =
            DirectXRenderer::new(window.0, &devices, true).expect("creating the renderer");
        renderer
            .resize(size(DevicePixels(SIZE), DevicePixels(SIZE)))
            .expect("sizing the render target");
        Self {
            renderer,
            _window: window,
        }
    }

    fn draw(&mut self, build: impl FnOnce(&mut Scene)) -> Image {
        let mut scene = Scene::default();
        build(&mut scene);
        scene.finish();
        let image = self
            .renderer
            .render_to_image(&scene, WindowBackgroundAppearance::Opaque)
            .expect("drawing the scene");
        Image(
            image
                .pixels()
                .map(|pixel| pixel.0.map(|channel| f32::from(channel) / 255.0))
                .collect(),
        )
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
    Bounds {
        origin: point(ScaledPixels(x), ScaledPixels(y)),
        size: size(ScaledPixels(width), ScaledPixels(height)),
    }
}

fn window_mask() -> ContentMask<ScaledPixels> {
    rect(0.0, 0.0, SIZE as f32, SIZE as f32).into()
}

fn quad(bounds: Bounds<ScaledPixels>, color: Hsla) -> Quad {
    Quad {
        bounds,
        content_mask: window_mask(),
        background: gpui::solid_background(color),
        ..Quad::default()
    }
}

/// White and black vertical stripes 2 pixels wide.
fn stripes(scene: &mut Scene) {
    for index in 0..SIZE / 2 {
        let color = if index % 2 == 0 { white() } else { black() };
        scene.insert_primitive(quad(rect(index as f32 * 2.0, 0.0, 2.0, SIZE as f32), color));
    }
}

/// Fails unless every pixel `(x, y)` for which `range` returns `(lo, hi)` has
/// each channel between `lo` and `hi`, within [`TOLERANCE`].
fn assert_pixels(name: &str, drawn: &Image, range: impl Fn(i32, i32) -> Option<(Rgba, Rgba)>) {
    let mut checked = 0;
    let mut worst: Option<(f32, String)> = None;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let Some((lo, hi)) = range(x, y) else {
                continue;
            };
            checked += 1;
            let pixel = drawn.at(x, y);
            let error = (0..4)
                .map(|c| (lo[c].min(hi[c]) - pixel[c]).max(pixel[c] - lo[c].max(hi[c])))
                .fold(0.0, f32::max)
                * 255.0;
            if error > worst.as_ref().map_or(TOLERANCE, |(worst, _)| *worst) {
                let detail = format!("({x}, {y}) drew {pixel:?}, expected {lo:?}..={hi:?}");
                worst = Some((error, detail));
            }
        }
    }
    assert!(checked >= 100, "{name}: only {checked} pixels checked");
    if let Some((error, detail)) = worst {
        panic!("{name}: off by {error:.1} steps at {detail}");
    }
}

/// Catches a blur that samples the wrong texels, weighs or places a ring
/// wrongly, ignores the body corner radii, or misses the clamp at the frame
/// edge the pane touches. Does not catch an error below [`TOLERANCE`].
#[test]
fn backdrop_blur_draws_the_documented_blur_of_the_frame() {
    let mut target = Target::new();
    let frame = target.draw(stripes);
    let blur = BackdropBlur {
        order: 0,
        pad: 0,
        bounds: rect(30.0, 20.0, 70.0, 60.0),
        content_mask: window_mask(),
        corner_radii: Corners::all(ScaledPixels(10.0)),
        blur_radius: ScaledPixels(6.0),
        saturation: 1.0,
        tint: hsla(0.0, 0.0, 0.0, 0.0),
        transformation: Default::default(),
    };
    let drawn = target.draw(|scene| {
        stripes(scene);
        scene.insert_primitive(blur);
    });
    assert_pixels("blur", &drawn, |x, y| {
        let expected = expected_blur(&frame, &blur, x, y);
        Some((expected, expected))
    });
}

/// Catches saturation, tint, and content mask coverage that differ from the
/// documented math, including a rounded content mask treated as a
/// rectangle.
#[test]
fn backdrop_blur_saturates_tints_and_clips_to_rounded_masks() {
    let background = |scene: &mut Scene| {
        for (index, h) in [0.0, 1.0 / 3.0, 2.0 / 3.0].into_iter().enumerate() {
            let x = index as f32 * 34.0;
            scene.insert_primitive(quad(
                rect(x, 0.0, 34.0, SIZE as f32),
                hsla(h, 1.0, 0.5, 1.0),
            ));
        }
        scene.insert_primitive(quad(rect(0.0, 45.0, SIZE as f32, 10.0), white()));
    };
    let mut target = Target::new();
    let frame = target.draw(background);
    let blur = BackdropBlur {
        order: 0,
        pad: 0,
        bounds: rect(20.0, 20.0, 70.0, 60.0),
        content_mask: ContentMask {
            bounds: rect(30.0, 10.0, 60.0, 70.0),
            corner_radii: Corners::all(ScaledPixels(16.0)),
        },
        corner_radii: Corners::all(ScaledPixels(8.0)),
        blur_radius: ScaledPixels(5.0),
        saturation: 0.4,
        tint: hsla(0.0, 0.0, 0.2, 0.35),
        transformation: Default::default(),
    };
    let drawn = target.draw(|scene| {
        background(scene);
        scene.insert_primitive(blur);
    });
    assert_pixels("saturated blur", &drawn, |x, y| {
        let expected = expected_blur(&frame, &blur, x, y);
        Some((expected, expected))
    });
}

/// Catches a clip that leaks its subtree outside the path, drops it inside,
/// composites straight instead of premultiplied color, or fills the curved
/// corners of the path as triangles.
#[test]
fn path_clip_composites_its_premultiplied_layer_by_path_coverage() {
    let blue = rgba(hsla(2.0 / 3.0, 1.0, 0.5, 1.0));
    let green = hsla(1.0 / 3.0, 1.0, 0.5, 1.0);
    let red = hsla(0.0, 1.0, 0.5, 0.5);
    let outline = Outline::rounded_square(20.0, 80.0, 15.0);
    let mut target = Target::new();
    let drawn = target.draw(|scene| {
        scene.insert_primitive(quad(
            rect(0.0, 0.0, 100.0, 100.0),
            hsla(2.0 / 3.0, 1.0, 0.5, 1.0),
        ));
        scene.push_path_clip(outline.clip_path());
        scene.insert_primitive(quad(rect(0.0, 0.0, 100.0, 50.0), green));
        scene.insert_primitive(quad(rect(50.0, 0.0, 50.0, 100.0), red));
        scene.pop_path_clip();
    });
    assert_pixels("path clip", &drawn, |x, y| {
        let mut layer = [0.0; 4];
        if y < 50 {
            layer = over(layer, rgba(green));
        }
        if x >= 50 {
            layer = over(layer, rgba(red));
        }
        let (lo, hi) = (composite(blue, layer, 0.0), composite(blue, layer, 1.0));
        Some(match outline.coverage(x, y) {
            Some(m) if m > 0.0 => (hi, hi),
            Some(_) => (lo, lo),
            None => (lo, hi),
        })
    });
}

/// Catches a nested clip that replaces its parent's layer or composites
/// onto the frame instead of onto the parent, so the result is not the
/// intersection of both paths.
#[test]
fn nested_path_clips_intersect() {
    let blue = rgba(hsla(2.0 / 3.0, 1.0, 0.5, 1.0));
    let green = rgba(hsla(1.0 / 3.0, 1.0, 0.5, 1.0));
    let outer = Outline::rounded_square(20.0, 80.0, 15.0);
    let inner = Outline::new((10.0, 10.0))
        .line_to((90.0, 10.0))
        .line_to((10.0, 90.0))
        .line_to((10.0, 10.0));
    let mut target = Target::new();
    let drawn = target.draw(|scene| {
        scene.insert_primitive(quad(
            rect(0.0, 0.0, 100.0, 100.0),
            hsla(2.0 / 3.0, 1.0, 0.5, 1.0),
        ));
        scene.push_path_clip(outer.clip_path());
        scene.push_path_clip(inner.clip_path());
        scene.insert_primitive(quad(
            rect(0.0, 0.0, 100.0, 100.0),
            hsla(1.0 / 3.0, 1.0, 0.5, 1.0),
        ));
        scene.pop_path_clip();
        scene.pop_path_clip();
    });
    assert_pixels("nested path clips", &drawn, |x, y| {
        Some(match (outer.coverage(x, y), inner.coverage(x, y)) {
            (Some(outer), Some(inner)) if outer * inner > 0.0 => (green, green),
            (Some(0.0), _) | (_, Some(0.0)) => (blue, blue),
            _ => (blue, green),
        })
    });
}

/// Catches a blur inside a clip that samples the clip layer, where the red
/// quad drawn before it would tint the result, instead of the frame beneath
/// every open clip layer.
#[test]
fn backdrop_blur_inside_a_path_clip_samples_the_frame_beneath_it() {
    let outline = Outline::new((20.0, 20.0))
        .line_to((80.0, 20.0))
        .line_to((80.0, 80.0))
        .line_to((20.0, 80.0))
        .line_to((20.0, 20.0));
    let blur = BackdropBlur {
        order: 0,
        pad: 0,
        bounds: rect(0.0, 0.0, 100.0, 100.0),
        content_mask: window_mask(),
        corner_radii: Corners::default(),
        blur_radius: ScaledPixels(6.0),
        saturation: 1.0,
        tint: hsla(0.0, 0.0, 0.0, 0.0),
        transformation: Default::default(),
    };
    let mut target = Target::new();
    let frame = target.draw(stripes);
    let drawn = target.draw(|scene| {
        stripes(scene);
        scene.push_path_clip(outline.clip_path());
        scene.insert_primitive(quad(rect(30.0, 30.0, 40.0, 40.0), hsla(0.0, 1.0, 0.5, 1.0)));
        scene.insert_primitive(blur);
        scene.pop_path_clip();
    });
    assert_pixels("blur inside a path clip", &drawn, |x, y| {
        let pixel = match outline.coverage(x, y)? {
            m if m > 0.0 => expected_blur(&frame, &blur, x, y),
            _ => frame.at(x, y),
        };
        Some((pixel, pixel))
    });
}
