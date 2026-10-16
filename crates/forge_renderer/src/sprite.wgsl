struct CameraUniform {
    viewport: vec4<f32>,
    camera: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> view: CameraUniform;

struct VertexInput {
    @builtin(vertex_index) vertex_index: u32,
    @location(0) position: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) scale: vec2<f32>,
    @location(3) rotation_radians: f32,
    @location(4) tint: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tint: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[input.vertex_index];
    let local = corner * input.size * input.scale * 0.5;
    let sprite_cos = cos(input.rotation_radians);
    let sprite_sin = sin(input.rotation_radians);
    let rotated_local = vec2<f32>(
        sprite_cos * local.x - sprite_sin * local.y,
        sprite_sin * local.x + sprite_cos * local.y,
    );
    let world_position = input.position + rotated_local;
    let camera_delta = world_position - view.camera.xy;
    let camera_angle = -view.camera.z;
    let camera_cos = cos(camera_angle);
    let camera_sin = sin(camera_angle);
    let camera_local = vec2<f32>(
        camera_cos * camera_delta.x - camera_sin * camera_delta.y,
        camera_sin * camera_delta.x + camera_cos * camera_delta.y,
    );
    let pixels = camera_local * view.camera.w * view.viewport.zw;
    let ndc = vec2<f32>(
        2.0 * pixels.x / view.viewport.x,
        -2.0 * pixels.y / view.viewport.y,
    );

    var output: VertexOutput;
    output.position = vec4<f32>(ndc, 0.0, 1.0);
    output.tint = input.tint;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.tint;
}
