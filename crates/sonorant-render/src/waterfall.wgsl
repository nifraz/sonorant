// The 3D waterfall: the history as a landscape, seen from a camera you can orbit.
//
// The mesh never leaves the GPU. A grid of vertices is displaced in the vertex shader
// straight out of the history texture, so the CPU rebuilds nothing as rows arrive: the
// same index buffer draws a different surface every frame because the texture under it
// has moved on. Frequency runs across the grid on the display's own axis, time runs away
// from the near edge, and height is the level on the same ramp the spectrogram colours
// with, so the two views agree about what is loud.
//
// Normals come from the four neighbouring grid points rather than from a normal map,
// which costs four more texture loads a vertex and gives the surface its shape under
// the light. Distance fog fades the far rows into the background so the landscape has
// somewhere to recede to.

struct Scene {
    // The camera, world space to clip space.
    view_projection: mat4x4<f32>,
    // The background the fog fades into, in linear light. Second, where its alignment
    // costs nothing; a vec4 further down would need padding to reach it.
    fog_colour: vec4<f32>,
    // Where the eye is, for the fog and the light.
    eye: vec3<f32>,
    // The grid: vertices across (frequency) and back (time).
    columns: u32,
    rows: u32,
    // Ring slot of the newest row, rows drawable, rows per layer, ring size.
    newest: u32,
    available: u32,
    layer_rows: u32,
    capacity: u32,
    // Rows of history the grid spans, and the clock's progress into the next one.
    span_rows: f32,
    frac: f32,
    // Which pane's levels: 0 or 1.
    channel: u32,
    // The display axis: 0 linear, 1 logarithmic, from fmin to fmax.
    scale: u32,
    fmin: f32,
    fmax: f32,
    // On a log axis the grid position is `grid_at + v * grid_span`, worked out on the
    // CPU; a linear axis takes the logarithm here instead.
    grid_at: f32,
    grid_span: f32,
    grid_bins: u32,
    grid_fmin: f32,
    grid_fmax: f32,
    // 1 to colour every row with the current range instead of its own.
    global_range: u32,
    floor_db: f32,
    ceiling_db: f32,
    // How tall the loudest level stands, in the surface's own units.
    relief: f32,
    // Where the fog starts and ends, as distances from the eye.
    fog_near: f32,
    fog_far: f32,
    // Light the surface has of its own, so a flat ground still shows its shape.
    sheen: f32,
};

@group(0) @binding(0) var<uniform> scene: Scene;
@group(0) @binding(1) var levels: texture_2d_array<f32>;
@group(0) @binding(2) var ranges: texture_2d<f32>;
@group(0) @binding(3) var palette: texture_2d<f32>;
@group(0) @binding(4) var palette_sampler: sampler;

fn slot_of(age: i32) -> i32 {
    let cap = i32(scene.capacity);
    return ((i32(scene.newest) - age) % cap + cap) % cap;
}

fn texel(slot: i32) -> vec2<i32> {
    let rows = i32(scene.layer_rows);
    return vec2<i32>(slot % rows, slot / rows);
}

// The level at a fractional grid position in one row, as `spectrogram.wgsl` reads it.
fn level(pos: f32, slot: i32) -> f32 {
    let t = texel(slot);
    let top = i32(scene.grid_bins) - 1;
    let b0 = clamp(i32(floor(pos)), 0, top);
    let b1 = clamp(b0 + 1, 0, top);
    let f = clamp(pos - floor(pos), 0.0, 1.0);
    let a = textureLoad(levels, vec2<i32>(b0, t.x), t.y, 0);
    let b = textureLoad(levels, vec2<i32>(b1, t.x), t.y, 0);
    let va = select(a.r, a.g, scene.channel == 1u);
    let vb = select(b.r, b.g, scene.channel == 1u);
    return mix(va, vb, f);
}

// A level as a position on the colour ramp, with the row's own range or the current one.
fn ramp(db: f32, slot: i32) -> f32 {
    var lo = scene.floor_db;
    var hi = scene.ceiling_db;
    if (scene.global_range == 0u) {
        let r = textureLoad(ranges, texel(slot), 0);
        lo = r.r;
        hi = r.g;
    }
    return clamp((db - lo) / max(hi - lo, 1.0), 0.0, 1.0);
}

// The grid position of the frequency at `across`, 0 at the axis's start and 1 at its end.
fn grid_position(across: f32) -> f32 {
    if (scene.scale == 0u) {
        let f = scene.fmin + (scene.fmax - scene.fmin) * across;
        return (log(max(f, 1e-3) / scene.grid_fmin) / log(scene.grid_fmax / scene.grid_fmin))
            * f32(scene.grid_bins) - 0.5;
    }
    return scene.grid_at + across * scene.grid_span;
}

// The height of the surface at a grid point, 0 to 1, and 0 past the end of the history.
fn height_at(across: f32, back: f32) -> f32 {
    let age = back * scene.span_rows + (1.0 - scene.frac);
    let a0 = i32(floor(age));
    if (a0 + 1 >= i32(scene.available)) {
        return 0.0;
    }
    let slot = slot_of(a0);
    return ramp(level(grid_position(across), slot), slot);
}

struct VsOut {
    @builtin(position) position: vec4<f32>,
    // Where on the ramp this vertex sits, for the palette.
    @location(0) ramp: f32,
    // The light already worked out, and how far into the fog the vertex is.
    @location(1) light: f32,
    @location(2) fog: f32,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let columns = max(scene.columns, 2u);
    let rows = max(scene.rows, 2u);
    let column = i % columns;
    let row = i / columns;
    let across = f32(column) / f32(columns - 1u);
    let back = f32(row) / f32(rows - 1u);
    // A step of one grid cell either way, for the normal.
    let dx = 1.0 / f32(columns - 1u);
    let dz = 1.0 / f32(rows - 1u);

    let h = height_at(across, back);
    // The surface spans -1 to 1 across and 0 to -2 away, so the near edge is the newest
    // row and the camera looks along the history.
    let world = vec3<f32>(across * 2.0 - 1.0, h * scene.relief, -back * 2.0);

    // Neighbouring heights give the slope, and the slope gives the normal. Clamped at
    // the edges, where the one-sided difference is the best there is.
    let hl = height_at(max(across - dx, 0.0), back);
    let hr = height_at(min(across + dx, 1.0), back);
    let hb = height_at(across, max(back - dz, 0.0));
    let hf = height_at(across, min(back + dz, 1.0));
    // The tangents, in the same units the world position uses.
    let along = vec3<f32>(dx * 2.0 * 2.0, (hr - hl) * scene.relief, 0.0);
    let away = vec3<f32>(0.0, (hf - hb) * scene.relief, -dz * 2.0 * 2.0);
    let normal = normalize(cross(away, along));

    var out: VsOut;
    out.position = scene.view_projection * vec4<f32>(world, 1.0);
    out.ramp = h;
    // A headlight over the viewer's shoulder, lifted so the ridges catch it, with
    // enough ambient that a face turned away is still coloured rather than black.
    let to_eye = normalize(scene.eye - world);
    let light = normalize(to_eye + vec3<f32>(0.0, 0.8, 0.0));
    out.light = 0.45 + 0.55 * max(dot(normal, light), 0.0);
    let distance = length(scene.eye - world);
    out.fog = clamp(
        (distance - scene.fog_near) / max(scene.fog_far - scene.fog_near, 1e-3),
        0.0,
        1.0,
    );
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let u = (in.ramp * 255.0 + 0.5) / 256.0;
    let colour = textureSampleLevel(palette, palette_sampler, vec2<f32>(u, 0.5), 0.0).rgb;
    // A little light of its own, on top of the palette's, so the flat ground shows its
    // shape rather than being black on black. Small enough that a quiet passage still
    // reads as quiet.
    let sheen = scene.sheen * in.light;
    return vec4<f32>(mix(colour * in.light + sheen, scene.fog_colour.rgb, in.fog), 1.0);
}
