// Album art as a stretched quad: the deck's cover over the furniture, and the
// backdrop's reduced copy under the visuals.
//
// The quad's corners arrive already in clip space, worked out on the CPU the way the
// overlay does it, so there is no transform here and no uniform to keep in step.

struct VertexIn {
    @location(0) clip: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) alpha: f32,
}

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) alpha: f32,
}

@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var picture_sampler: sampler;

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(in.clip, 0.0, 1.0);
    out.uv = in.uv;
    out.alpha = in.alpha;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let texel = textureSample(picture, picture_sampler, in.uv);
    return vec4<f32>(texel.rgb, texel.a * in.alpha);
}
