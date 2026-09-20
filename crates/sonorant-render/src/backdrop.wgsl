// The beat-reactive backdrop: a slow field of light behind the analysis, with rings
// riding out from the centre on the beat.
//
// It is drawn into the visuals target, over the blurred album cover when there is one
// and under the spectrogram either way, and it only ever adds light. Nothing here can
// darken the image, so the analysis over it reads the same as it always did; what the
// strength setting buys is how far forward the ground comes.
//
// Everything is in linear light: this target is the floating-point one the glow works
// in, not the encoded swapchain.

struct Field {
    // The palette's deep and hot colours, in linear light.
    deep: vec4<f32>,
    hot: vec4<f32>,
    // The framebuffer, for the aspect ratio.
    size: vec2<f32>,
    // Seconds since the app started, for the slow drift.
    time: f32,
    // Where the beat is, 0 to 1 and round again.
    phase: f32,
    // 1 at an onset, falling away towards 0.
    pulse: f32,
    // Where the music's energy sits, 0 at the bottom of the axis and 1 at the top.
    brightness: f32,
    // How far forward the backdrop comes, 0 to 1.
    strength: f32,
    // 0 when the beat is switched off, and the field only drifts.
    reactive: f32,
};

@group(0) @binding(0) var<uniform> field: Field;

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

// Three ridges at unrelated angles and rates. Three is enough to read as movement
// without settling into a pattern the eye can follow, and it costs three sines.
//
// The frequencies matter more than they look. Low ones put the whole window inside a
// single lobe, and then the field is not a field at all: it is one flat wash that slowly
// brightens and dims, which is exactly the haze this is shaped to avoid. These put two
// or three bands across the screen each, so the crests are somewhere rather than
// everywhere.
fn drift(p: vec2<f32>, t: f32) -> f32 {
    var f = sin(p.x * 15.1 + t * 0.23);
    f += sin((p.x * 4.3 + p.y * 12.2) - t * 0.17);
    f += sin((p.y * 18.0 - p.x * 6.7) + t * 0.31);
    return f / 3.0 * 0.5 + 0.5;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (field.strength <= 0.0) {
        return vec4<f32>(0.0);
    }
    let aspect = max(field.size.x, 1.0) / max(field.size.y, 1.0);
    let p = (in.uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    // Only the crests of the field light up, and the rest of the window stays dark.
    // An even wash over everything is haze, and haze is what the axis labels and the
    // analysis would then have to be read through; bands of light are ground.
    var v = smoothstep(0.40, 0.95, drift(p, field.time)) * 0.6;

    // Rings leaving the centre, one a beat. The sharp edge leads and the light trails
    // inside it, which is what makes it read as something travelling outwards rather
    // than a circle being drawn. The pulse is what makes a hit read as a hit; the phase
    // is what keeps the rings coming in time rather than only when the analysis finds
    // an onset.
    let behind = fract(field.phase - length(p) * 1.6);
    let ring = exp(-behind * 7.0) * field.pulse;
    v += ring * field.reactive * 0.35;

    // Where the music sits decides where the field is strongest: a bass-heavy passage
    // weights the bottom of the screen, a bright one the top.
    let up = 1.0 - in.uv.y;
    v *= mix(1.2 - up * 0.8, 0.4 + up * 0.8, clamp(field.brightness, 0.0, 1.0));

    let amount = clamp(v, 0.0, 1.0) * field.strength;
    let colour = mix(field.deep.rgb, field.hot.rgb, clamp(v * 2.0, 0.0, 1.0));
    return vec4<f32>(colour * amount, amount);
}
