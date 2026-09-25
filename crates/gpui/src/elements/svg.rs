use std::{
    fs,
    hash::{Hash, Hasher},
    path::Path,
    sync::Arc,
};

use crate::{
    App, Asset, Bounds, Element, GlobalElementId, Hitbox, InspectorElementId, InteractiveElement,
    Interactivity, IntoElement, LayoutId, Pixels, SharedString, StyleRefinement, Styled,
    Transformation, Window,
};
use gpui_util::ResultExt;

/// An SVG element. The glyph paints as a mask in the current text color: the
/// color set with `text_color` on the element, else the one it inherits.
pub struct Svg {
    interactivity: Interactivity,
    transformation: Option<Transformation>,
    path: Option<SharedString>,
    external_path: Option<SharedString>,
    data: Option<Arc<[u8]>>,
    data_path: Option<SharedString>,
}

/// Create a new SVG element.
#[track_caller]
pub fn svg() -> Svg {
    Svg {
        interactivity: Interactivity::new(),
        transformation: None,
        path: None,
        external_path: None,
        data: None,
        data_path: None,
    }
}

impl Svg {
    /// Set the path to the SVG file for this element.
    pub fn path(mut self, path: impl Into<SharedString>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Set the path to the SVG file for this element.
    pub fn external_path(mut self, path: impl Into<SharedString>) -> Self {
        self.external_path = Some(path.into());
        self
    }

    /// Set the raw SVG data for this element.
    /// The SVG will be rendered directly from the provided bytes.
    pub fn data(mut self, data: &[u8]) -> Self {
        // Generate a unique deterministic path based on the data hash for caching
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        data.hash(&mut hasher);
        let hash = hasher.finish();
        let path = SharedString::from(format!("__binary_svg__{}", hash));
        self.data = Some(Arc::from(data));
        self.data_path = Some(path);
        self
    }

    /// Transform the SVG element with the given transformation.
    /// Note that this won't effect the hitbox or layout of the element, only the rendering.
    pub fn with_transformation(mut self, transformation: Transformation) -> Self {
        self.transformation = Some(transformation);
        self
    }
}

impl Element for Svg {
    type RequestLayoutState = ();
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<crate::ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |style, window, cx| window.request_layout(style, None, cx),
        );
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Hitbox> {
        self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            bounds.size,
            window,
            cx,
            |_, _, hitbox, _, _| hitbox,
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Option<Hitbox>,
        window: &mut Window,
        cx: &mut App,
    ) where
        Self: Sized,
    {
        self.interactivity.paint(
            global_id,
            inspector_id,
            bounds,
            hitbox.as_ref(),
            window,
            cx,
            |style, window, cx| {
                let local_transformation = self
                    .transformation
                    .or(style.transformation)
                    .map(|t| t.into_matrix(bounds.center(), window.scale_factor()))
                    .unwrap_or_default();
                // The glyph paints as a mask in the current text color: the
                // element's own, else the one it inherits, as `currentColor`
                // does in CSS.
                let color = window.text_color();

                window.with_transformation(local_transformation, |window| {
                    let transformation = window.transformation();
                    if let Some((data, path)) = self.data.as_ref().zip(self.data_path.as_ref()) {
                        window
                            .paint_svg(
                                bounds,
                                path.clone(),
                                Some(&**data),
                                transformation,
                                color,
                                cx,
                            )
                            .log_err();
                    } else if let Some(path) = self.external_path.as_ref() {
                        let Some(bytes) = window
                            .use_asset::<SvgAsset>(path, cx)
                            .and_then(|asset| asset.log_err())
                        else {
                            return;
                        };

                        window
                            .paint_svg(
                                bounds,
                                path.clone(),
                                Some(&bytes),
                                transformation,
                                color,
                                cx,
                            )
                            .log_err();
                    } else if let Some(path) = self.path.as_ref() {
                        window
                            .paint_svg(bounds, path.clone(), None, transformation, color, cx)
                            .log_err();
                    }
                });
            },
        );
    }
}

impl IntoElement for Svg {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for Svg {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl InteractiveElement for Svg {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

enum SvgAsset {}

impl Asset for SvgAsset {
    type Source = SharedString;
    type Output = Result<Arc<[u8]>, Arc<std::io::Error>>;

    fn load(
        source: Self::Source,
        _cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        async move {
            let bytes = fs::read(Path::new(source.as_ref())).map_err(|e| Arc::new(e))?;
            let bytes = Arc::from(bytes);
            Ok(bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Context, Hsla, TestAppContext, Window, black, blue, div, prelude::*, px, red, size, svg,
    };

    /// An 8x8 filled square.
    const SQUARE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8"/></svg>"#;

    /// One glyph, with an optional text color of its own, inside a container
    /// with an optional text color.
    struct Glyph {
        parent: Option<Hsla>,
        own: Option<Hsla>,
    }

    impl Render for Glyph {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let mut glyph = svg().data(SQUARE).size(px(8.));
            if let Some(color) = self.own {
                glyph = glyph.text_color(color);
            }
            let mut parent = div().size_full();
            if let Some(color) = self.parent {
                parent = parent.text_color(color);
            }
            parent.child(glyph)
        }
    }

    /// An SVG paints in its own text color, else the text color it inherits,
    /// else the default text color, and never paints nothing for want of a
    /// color of its own.
    ///
    /// Closes the class of glyphs that vanish inside a control that sets the
    /// text color for its label and not on the glyph. Drives the `data`
    /// source; the `path` and `external_path` sources paint with the same
    /// color value, read once before the source is chosen.
    #[gpui::test]
    fn an_svg_paints_in_its_own_else_the_inherited_text_color(cx: &mut TestAppContext) {
        let cases = [
            (Some(red()), None, red()),
            (Some(red()), Some(blue()), blue()),
            (None, Some(blue()), blue()),
            (None, None, black()),
        ];
        for (parent, own, expected) in cases {
            let window = cx.open_window(size(px(40.), px(40.)), |_, _| Glyph { parent, own });
            cx.run_until_parked();
            let colors: Vec<Hsla> = window
                .update(cx, |_, window, _| {
                    window
                        .rendered_frame
                        .scene
                        .monochrome_sprites
                        .iter()
                        .map(|sprite| sprite.color)
                        .collect()
                })
                .unwrap();
            assert_eq!(
                colors,
                [expected],
                "parent {parent:?}, own {own:?}: painted {colors:?}"
            );
        }
    }
}
