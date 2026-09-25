//! A path clip brackets its subtree in draw order and replays unchanged.
//!
//! Catches a clip marker keyed so that a child sorts outside its clip (a
//! child at a higher z-index, or a child of an outer clip after an inner clip
//! closed), which draws the child unclipped, and a replay that records a
//! clip's scope twice, which nests one more layer per replayed frame. Does
//! not catch a renderer that composites a correctly ordered clip wrongly.

use super::*;
use crate::{point, px, size};

fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<ScaledPixels> {
    Bounds {
        origin: point(ScaledPixels(x), ScaledPixels(y)),
        size: size(ScaledPixels(w), ScaledPixels(h)),
    }
}

fn quad(bounds: Bounds<ScaledPixels>) -> Quad {
    Quad {
        bounds,
        content_mask: rect(0.0, 0.0, 1000.0, 1000.0).into(),
        ..Quad::default()
    }
}

/// A clip path covering the square from `lo` to `hi`.
fn square(lo: f32, hi: f32) -> Path<ScaledPixels> {
    let mut path = Path::new(point(px(lo), px(lo)));
    path.line_to(point(px(hi), px(lo)));
    path.line_to(point(px(hi), px(hi)));
    path.line_to(point(px(lo), px(hi)));
    let mut path = path.scale(1.0);
    path.content_mask = rect(0.0, 0.0, 1000.0, 1000.0).into();
    path
}

/// `S` for a clip start, `E` for a clip end, `Q` for each quad, in draw
/// order.
fn sequence(scene: &Scene) -> String {
    scene
        .batches()
        .map(|batch| match batch {
            PrimitiveBatch::StartPathClip(_) => "S".to_string(),
            PrimitiveBatch::EndPathClip => "E".to_string(),
            PrimitiveBatch::Quads(range) => "Q".repeat(range.len()),
            other => panic!("unexpected batch {other:?}"),
        })
        .collect()
}

#[test]
fn a_child_at_any_z_index_draws_inside_its_clip() {
    for z_index in [i32::MIN, -1, 1, i32::MAX] {
        let mut scene = Scene::default();
        scene.push_path_clip(square(0.0, 100.0));
        scene.push_z_index(z_index);
        scene.insert_primitive(quad(rect(10.0, 10.0, 20.0, 20.0)));
        scene.pop_z_index();
        scene.pop_path_clip();
        scene.finish();
        assert_eq!(sequence(&scene), "SQE", "z-index {z_index}");
    }
}

/// Quads painted in the outer clip after the inner clip closed, stacked so
/// each overlaps the one before and none overlaps the inner clip. A quad that
/// does not overlap the inner clip may draw before or after it; every quad
/// must fall between the outer clip's start and end.
#[test]
fn children_of_an_outer_clip_painted_after_an_inner_clip_draw_inside_the_outer_clip() {
    let mut scene = Scene::default();
    scene.push_path_clip(square(0.0, 200.0));
    scene.push_path_clip(square(0.0, 50.0));
    scene.insert_primitive(quad(rect(10.0, 10.0, 20.0, 20.0)));
    scene.pop_path_clip();
    for offset in 0..4 {
        let offset = offset as f32 * 5.0;
        scene.insert_primitive(quad(rect(100.0 + offset, 100.0, 50.0, 50.0)));
    }
    scene.pop_path_clip();
    scene.finish();
    let drawn = sequence(&scene);
    assert_eq!(drawn.matches('Q').count(), 5, "{drawn}");
    assert!(
        drawn.starts_with('S') && drawn.ends_with('E') && drawn.contains("SQE"),
        "a quad drew outside its clip: {drawn}"
    );
}

/// Replaying a clipped subtree, and replaying that replay, records the same
/// operations and draws the same batches as the original paint.
#[test]
fn replaying_a_clip_records_the_same_operations() {
    let mut first = Scene::default();
    first.push_path_clip(square(0.0, 100.0));
    first.insert_primitive(quad(rect(10.0, 10.0, 20.0, 20.0)));
    first.pop_path_clip();
    first.finish();

    let operations = |scene: &Scene| {
        scene
            .paint_operations
            .iter()
            .map(std::mem::discriminant)
            .collect::<Vec<_>>()
    };
    let mut previous = first;
    for _ in 0..3 {
        let mut next = Scene::default();
        next.replay(0..previous.len(), &previous);
        next.finish();
        assert_eq!(operations(&next), operations(&previous));
        assert_eq!(next.scopes.len(), 1, "replay left a scope open");
        assert_eq!(sequence(&next), "SQE");
        previous = next;
    }
}
