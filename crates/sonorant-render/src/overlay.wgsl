// The overlay's shapes: filled rectangles, gradients and anti-aliased line segments.
//
// Positions arrive in clip space, worked out when the shape was queued, with the pixel
// position alongside so a line can measure how much of each pixel it covers. A vertex
// with a zero half-width belongs to a plain fill; otherwise it belongs to a quad drawn
// round a segment, and each pixel takes as much colour as its distance to that segment
// allows, so a line at any angle is anti-aliased.
//
// Colours are sRGB-encoded with straight alpha and blended as such (see colour.rs).

struct VertexIn {
    @location(0) clip: vec2<f32>,
    @location(1) pos: vec2<f32>,
    @location(2) colour: vec4<f32>,
    // The segment's ends in pixels, for lines.
    @location(3) segment: vec4<f32>,
    @location(4) half_width: f32,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) colour: vec4<f32>,
    @location(1) segment: vec4<f32>,
    @location(2) half_width: f32,
    @location(3) pos: vec2<f32>,
};

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = vec4<f32>(v.clip, 0.0, 1.0);
    out.colour = v.colour;
    out.segment = v.segment;
    out.half_width = v.half_width;
    out.pos = v.pos;
    return out;
}

@fragment
fn fs_main(v: VertexOut) -> @location(0) vec4<f32> {
    if (v.half_width <= 0.0) {
        return v.colour;
    }
    let a = v.segment.xy;
    let b = v.segment.zw;
    let ab = b - a;
    let t = clamp(dot(v.pos - a, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
    let d = length(v.pos - (a + ab * t));
    let coverage = clamp(v.half_width + 0.5 - d, 0.0, 1.0);
    return vec4<f32>(v.colour.rgb, v.colour.a * coverage);
}
