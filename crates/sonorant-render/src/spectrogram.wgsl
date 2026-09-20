// The scrolling spectrogram.
//
// The history holds levels in dB on a fixed log-spaced grid, two panes per texel. This
// pass turns each pixel of a pane into a time and a frequency on the display's own
// axis, reads the level there, and colours it through the 256-entry palette with the
// range the row was cut with (or the current range, in global mode). So a palette,
// axis or range change repaints the whole history at once.
//
// Time runs away from the pane's newest edge. The image scrolls by audio time, not by
// frames: `frac` is how far the audio clock has moved into the row after the newest,
// so motion is equally smooth at any refresh rate.

struct Pane {
    // Ring slot of the newest row, rows drawable, rows per layer, ring size.
    newest: u32,
    available: u32,
    layer_rows: u32,
    capacity: u32,
    // Rows across the pane's width, and the clock's progress into the next row.
    visible_rows: f32,
    frac: f32,
    // 1 when the newest row is at the left edge, 0 at the right.
    newest_left: u32,
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
    // The grid the history is stored on.
    grid_bins: u32,
    grid_fmin: f32,
    grid_fmax: f32,
    // 1 to colour every row with the current range instead of its own.
    global_range: u32,
    // 1 to blend between neighbouring rows rather than draw crisp columns.
    smooth_time: u32,
    floor_db: f32,
    ceiling_db: f32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    _pad3: u32,
};

@group(0) @binding(0) var<uniform> pane: Pane;
@group(0) @binding(1) var levels: texture_2d_array<f32>;
@group(0) @binding(2) var ranges: texture_2d<f32>;
@group(0) @binding(3) var palette: texture_2d<f32>;
@group(0) @binding(4) var palette_sampler: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// One triangle covering the viewport; the pass sets the viewport to the pane.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var out: VsOut;
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

fn slot_of(age: i32) -> i32 {
    let cap = i32(pane.capacity);
    return ((i32(pane.newest) - age) % cap + cap) % cap;
}

fn texel(slot: i32) -> vec2<i32> {
    let rows = i32(pane.layer_rows);
    return vec2<i32>(slot % rows, slot / rows);
}

// The level at a fractional grid position in one row.
fn level(pos: f32, slot: i32) -> f32 {
    let t = texel(slot);
    let top = i32(pane.grid_bins) - 1;
    let b0 = clamp(i32(floor(pos)), 0, top);
    let b1 = clamp(b0 + 1, 0, top);
    let f = clamp(pos - floor(pos), 0.0, 1.0);
    let a = textureLoad(levels, vec2<i32>(b0, t.x), t.y, 0);
    let b = textureLoad(levels, vec2<i32>(b1, t.x), t.y, 0);
    let va = select(a.r, a.g, pane.channel == 1u);
    let vb = select(b.r, b.g, pane.channel == 1u);
    return mix(va, vb, f);
}

// A level as a position on the colour ramp, with the row's range or the current one.
fn ramp(db: f32, slot: i32) -> f32 {
    var lo = pane.floor_db;
    var hi = pane.ceiling_db;
    if (pane.global_range == 0u) {
        let r = textureLoad(ranges, texel(slot), 0);
        lo = r.r;
        hi = r.g;
    }
    return clamp((db - lo) / max(hi - lo, 1.0), 0.0, 1.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let across = select(1.0 - in.uv.x, in.uv.x, pane.newest_left == 1u);
    // One row of latency keeps the newest edge on data that exists.
    let age = across * pane.visible_rows + (1.0 - pane.frac);

    let v = 1.0 - in.uv.y;
    var pos = pane.grid_at + v * pane.grid_span;
    if (pane.scale == 0u) {
        let f = pane.fmin + (pane.fmax - pane.fmin) * v;
        pos = (log(max(f, 1e-3) / pane.grid_fmin) / log(pane.grid_fmax / pane.grid_fmin))
            * f32(pane.grid_bins) - 0.5;
    }

    var t = 0.0;
    let a0 = i32(floor(age));
    if (a0 + 1 < i32(pane.available)) {
        let s0 = slot_of(a0);
        t = ramp(level(pos, s0), s0);
        if (pane.smooth_time == 1u) {
            let s1 = slot_of(a0 + 1);
            t = mix(t, ramp(level(pos, s1), s1), age - f32(a0));
        }
    }
    let u = (t * 255.0 + 0.5) / 256.0;
    return textureSampleLevel(palette, palette_sampler, vec2<f32>(u, 0.5), 0.0);
}
