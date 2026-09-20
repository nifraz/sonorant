// The visuals' floating-point target, its glow, and the composite that puts them on the
// screen.
//
// Nostalgia+ built its glow by shrinking the spectrogram to an eighth, pushing the
// colours up and turning brightness into alpha, then blending the blurred result back
// over the image. This does the same in three small passes: threshold into an
// eighth-size target, blur it across and down, and add it during the composite. The
// threshold reads sRGB-encoded brightness, as the original's colour matrix did, while
// everything else works in linear light.

struct Bloom {
    // One texel of the source, for the blur's steps.
    texel: vec2<f32>,
    // 0 across, 1 down.
    vertical: u32,
    // How much of the glow reaches the screen.
    strength: f32,
};

@group(0) @binding(0) var<uniform> bloom: Bloom;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var glow: texture_2d<f32>;
@group(0) @binding(3) var smooth_sampler: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var out: VsOut;
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

// Linear light to sRGB-encoded, to judge brightness as the original did.
fn encode(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// What glows: the colours pushed up, with brightness past a threshold as the amount.
// Premultiplied, so the blur and the composite are plain additions.
@fragment
fn fs_threshold(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(source, smooth_sampler, in.uv, 0.0).rgb;
    let s = encode(c);
    let a = clamp(0.55 * (s.r + s.g + s.b) - 0.38, 0.0, 1.0);
    return vec4<f32>(c * 1.6 * a, a);
}

// Nine taps along one axis, weighted as a Gaussian.
@fragment
fn fs_blur(in: VsOut) -> @location(0) vec4<f32> {
    let step = select(
        vec2<f32>(bloom.texel.x, 0.0),
        vec2<f32>(0.0, bloom.texel.y),
        bloom.vertical == 1u,
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

// The visuals with the glow over them, into the swapchain's sRGB view.
@fragment
fn fs_composite(in: VsOut) -> @location(0) vec4<f32> {
    let base = textureSampleLevel(source, smooth_sampler, in.uv, 0.0).rgb;
    if (bloom.strength <= 0.0) {
        return vec4<f32>(base, 1.0);
    }
    let g = textureSampleLevel(glow, smooth_sampler, in.uv, 0.0) * bloom.strength;
    // Premultiplied over: what the glow covers, it replaces.
    return vec4<f32>(base * (1.0 - clamp(g.a, 0.0, 1.0)) + g.rgb, 1.0);
}
