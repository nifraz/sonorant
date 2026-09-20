// A phosphor screen: light laid down along a trace, fading with real time.
//
// The accumulator is a floating-point texture the size of the scope, kept between
// frames. Each frame fades it by how much real time has passed and then adds this
// frame's trace additively, so a fast figure leaves a dim tail and a slow one burns in,
// the way a CRT's phosphor behaves. The glow is the accumulator shrunk and blurred, not
// a threshold of it: on a phosphor every lit pixel halos, not only the bright ones.
//
// The composite writes sRGB-encoded light, because the furniture target it goes into is
// the plain swapchain view (see colour.rs).

struct Scope {
    // Where the scope goes in the framebuffer: x, y, width, height.
    rect: vec4<f32>,
    // The trace's colour, straight (not premultiplied).
    colour: vec4<f32>,
    // The accumulator's size in pixels.
    size: vec2<f32>,
    // One texel of whatever this pass reads.
    texel: vec2<f32>,
    // The framebuffer's size, for the composite's clip-space arithmetic.
    screen: vec2<f32>,
    // Half the trace's solid width, in pixels, before the edge is feathered.
    half_width: f32,
    // The light one segment lays down, once, at full coverage.
    deposit: f32,
    // How much of the glow reaches the composite.
    glow: f32,
    // The composite's overall fade, for immersive mode's auto-hide.
    alpha: f32,
    // 0 across, 1 down.
    vertical: u32,
    _pad: u32,
};

@group(0) @binding(0) var<uniform> scope: Scope;

struct Full {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// One triangle covering the target, as the bloom chain does it.
@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> Full {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var out: Full;
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

// The fade. What it returns never reaches the target: the pipeline multiplies the
// destination by the blend constant and the source by zero, so this pass is the
// cheapest way to scale a whole texture in place.
@fragment
fn fs_decay() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0);
}

// ---------------------------------------------------------------- the trace

struct Trace {
    @builtin(position) position: vec4<f32>,
    // The segment's ends and this fragment's place, all in accumulator pixels.
    @location(0) a: vec2<f32>,
    @location(1) b: vec2<f32>,
    @location(2) at: vec2<f32>,
};

// One instance per segment, expanded to a quad a little wider than the line so the
// feathered edge has room. Vertices 0 and 1 sit at `a`, 2 and 3 at `b`.
@vertex
fn vs_trace(
    @builtin(vertex_index) vi: u32,
    @location(0) a: vec2<f32>,
    @location(1) b: vec2<f32>,
) -> Trace {
    let span = b - a;
    let len = length(span);
    // A segment of no length still has to draw: a still trace is a dot, not nothing.
    let dir = select(vec2<f32>(1.0, 0.0), span / len, len > 1e-6);
    let normal = vec2<f32>(-dir.y, dir.x);
    // The solid half-width plus a pixel each way for the feather.
    let out_by = scope.half_width + 1.0;
    let far = vi >= 2u;
    let along = select(-out_by, out_by, far);
    let across = select(-out_by, out_by, (vi & 1u) == 1u);
    let at = select(a, b, far) + dir * along + normal * across;

    var out: Trace;
    // Pixels to clip space, y down.
    let ndc = vec2<f32>(at.x / scope.size.x * 2.0 - 1.0, 1.0 - at.y / scope.size.y * 2.0);
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.a = a;
    out.b = b;
    out.at = at;
    return out;
}

// Distance from `p` to the segment `a`..`b`.
fn distance_to_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let span = b - a;
    let len2 = dot(span, span);
    if (len2 < 1e-12) {
        return distance(p, a);
    }
    let t = clamp(dot(p - a, span) / len2, 0.0, 1.0);
    return distance(p, a + span * t);
}

// Light added along the segment, premultiplied, for an additive blend.
@fragment
fn fs_trace(in: Trace) -> @location(0) vec4<f32> {
    let d = distance_to_segment(in.at, in.a, in.b);
    // One pixel of feather, so a diagonal trace is not a staircase.
    let coverage = clamp(scope.half_width + 0.5 - d, 0.0, 1.0);
    let lit = coverage * scope.deposit * scope.colour.a;
    return vec4<f32>(scope.colour.rgb * lit, lit);
}

// ---------------------------------------------------------------- the glow

@group(0) @binding(1) var accumulator: texture_2d<f32>;
@group(0) @binding(2) var glow: texture_2d<f32>;
@group(0) @binding(3) var smooth_sampler: sampler;

// The accumulator shrunk, four taps half a texel apart so nothing is skipped over.
@fragment
fn fs_down(in: Full) -> @location(0) vec4<f32> {
    let o = scope.texel * 0.5;
    var sum = textureSampleLevel(accumulator, smooth_sampler, in.uv + vec2<f32>(-o.x, -o.y), 0.0);
    sum += textureSampleLevel(accumulator, smooth_sampler, in.uv + vec2<f32>(o.x, -o.y), 0.0);
    sum += textureSampleLevel(accumulator, smooth_sampler, in.uv + vec2<f32>(-o.x, o.y), 0.0);
    sum += textureSampleLevel(accumulator, smooth_sampler, in.uv + vec2<f32>(o.x, o.y), 0.0);
    return sum * 0.25;
}

// Nine taps along one axis, weighted as a Gaussian, as the bloom chain does it.
@fragment
fn fs_blur(in: Full) -> @location(0) vec4<f32> {
    let step = select(
        vec2<f32>(scope.texel.x, 0.0),
        vec2<f32>(0.0, scope.texel.y),
        scope.vertical == 1u,
    );
    let weights = array<f32, 5>(0.2270270, 0.1945946, 0.1216216, 0.0540541, 0.0162162);
    var sum = textureSampleLevel(glow, smooth_sampler, in.uv, 0.0) * weights[0];
    for (var i = 1; i < 5; i++) {
        let offset = step * f32(i);
        sum += textureSampleLevel(glow, smooth_sampler, in.uv + offset, 0.0) * weights[i];
        sum += textureSampleLevel(glow, smooth_sampler, in.uv - offset, 0.0) * weights[i];
    }
    return sum;
}

// ---------------------------------------------------------------- the composite

// A quad over the scope's rectangle, in framebuffer pixels.
//
// Four vertices as a strip, not the oversized triangle the full-target passes use: that
// one runs its coordinates to 2 and relies on the target clipping it, which over a
// placed rectangle would cover four times the area and smear the clamped edge across
// the rest of the deck.
@vertex
fn vs_composite(@builtin(vertex_index) i: u32) -> Full {
    let x = f32(i & 1u);
    let y = f32((i >> 1u) & 1u);
    let at = scope.rect.xy + vec2<f32>(x, y) * scope.rect.zw;
    var out: Full;
    out.position = vec4<f32>(
        at.x / scope.screen.x * 2.0 - 1.0,
        1.0 - at.y / scope.screen.y * 2.0,
        0.0,
        1.0,
    );
    out.uv = vec2<f32>(x, y);
    return out;
}

// Linear light to sRGB-encoded, so it can be added to the furniture target.
fn encode(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// The phosphor over whatever the deck drew: light added, never taken away.
@fragment
fn fs_composite(in: Full) -> @location(0) vec4<f32> {
    let core = textureSampleLevel(accumulator, smooth_sampler, in.uv, 0.0);
    let halo = textureSampleLevel(glow, smooth_sampler, in.uv, 0.0) * scope.glow;
    // Premultiplied throughout, so the two are simply summed.
    let light = (core.rgb + halo.rgb) * scope.alpha;
    return vec4<f32>(encode(min(light, vec3<f32>(1.0))), 1.0);
}
