@group(0) @binding(0) var retained_frame: texture_2d<f32>;

@fragment
fn fs_present(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(retained_frame, vec2<i32>(position.xy), 0);
}
