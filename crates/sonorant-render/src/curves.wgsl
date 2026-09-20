// The curve strip: one pane's spectrum drawn sideways beside its spectrogram.
//
// Frequency runs up the strip, one row of values per pixel row with the lowest at the
// bottom, and level runs along it from the pane's outer edge (the base) towards the
// spectrogram. Every pixel works out what covers it: the background pattern, the
// filled area under the curve, the curve itself, and the peak, average, minimum and
// reference traces. Lines are measured as distances to the few segments near the
// pixel, so they are anti-aliased and a translucent trace never darkens where its
// segments meet.
//
// Colours are sRGB-encoded and blended as such, as the GDI+ original did: the pass
// draws into the swapchain's plain (non-sRGB) view.

struct Curve {
    // The strip in framebuffer pixels: x, y, width, height.
    rect: vec4<f32>,
    // The level axis: the base's x, +1 when level grows rightwards, and the length.
    base_x: f32,
    dir: f32,
    amp: f32,
    // Rows of values, and the texture row of this pane's first series.
    n: u32,
    first_row: u32,
    // 0 line, 1 bars, 2 LED.
    style: u32,
    bar_size: u32,
    led_segment: u32,
    floor_db: f32,
    span_db: f32,
    // Bit 0 fill, 1 peak trace, 2 average, 3 minimum, 4 reference.
    flags: u32,
    // 0 plain, 1 lines, 2 grid, 3 chessboard.
    background: u32,
    db_step: f32,
    line_width: f32,
    _pad0: u32,
    _pad1: u32,
    // The fill's gradient, from the strip's left edge to its right.
    fill_left: vec4<f32>,
    fill_right: vec4<f32>,
    line: vec4<f32>,
    peak: vec4<f32>,
    average: vec4<f32>,
    minimum: vec4<f32>,
    reference: vec4<f32>,
    grid: vec4<f32>,
    chess: vec4<f32>,
};

// The series, one texture row each, in this order per pane.
const DISPLAY: u32 = 0u;
const PEAK: u32 = 1u;
const AVERAGE: u32 = 2u;
const MINIMUM: u32 = 3u;
const REFERENCE: u32 = 4u;

@group(0) @binding(0) var<uniform> c: Curve;
@group(0) @binding(1) var series: texture_2d<f32>;
@group(0) @binding(2) var palette: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

// A level as an x in framebuffer pixels, at the centre of the pixel it names.
fn x_for(db: f32) -> f32 {
    let t = clamp((db - c.floor_db) / c.span_db, 0.0, 1.0);
    return c.base_x + c.dir * t * c.amp + 0.5;
}

// The value of a series at pixel row `y` of the strip, 0 at the top.
fn value(s: u32, y: i32) -> f32 {
    let n = i32(c.n);
    let i = n - 1 - clamp(y, 0, n - 1);
    return textureLoad(series, vec2<i32>(i, i32(c.first_row + s)), 0).r;
}

fn segment_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
    return length(p - (a + ab * t));
}

// Distance from `p` to a series' polyline, which has a point at the centre of every
// row. Only the segments within two rows can be nearer than a line is wide.
fn polyline_distance(s: u32, p: vec2<f32>, row: i32) -> f32 {
    let top = c.rect.y + 0.5;
    // The segments near this row, and the span of x they cover: a pixel outside that
    // span by more than a line's width can't be covered, and most pixels are.
    var lo = 1e9;
    var hi = -1e9;
    var xs = array<f32, 4>(0.0, 0.0, 0.0, 0.0);
    for (var k = 0; k < 4; k++) {
        let y = clamp(row - 2 + k, 0, i32(c.n) - 1);
        let x = x_for(value(s, y));
        xs[k] = x;
        lo = min(lo, x);
        hi = max(hi, x);
    }
    let reach = c.line_width * 0.5 + 1.0;
    if (p.x < lo - reach || p.x > hi + reach) {
        return 1e9;
    }
    var d = 1e9;
    for (var k = 0; k < 3; k++) {
        let y = row - 2 + k;
        if (y < 0 || y + 1 >= i32(c.n)) {
            continue;
        }
        let a = vec2<f32>(xs[k], top + f32(y));
        let b = vec2<f32>(xs[k + 1], top + f32(y + 1));
        d = min(d, segment_distance(p, a, b));
    }
    return d;
}

// Porter-Duff "over" on straight-alpha colours.
fn over(dst: vec4<f32>, src: vec4<f32>) -> vec4<f32> {
    let a = src.a + dst.a * (1.0 - src.a);
    if (a <= 0.0) {
        return vec4<f32>(0.0);
    }
    let rgb = (src.rgb * src.a + dst.rgb * dst.a * (1.0 - src.a)) / a;
    return vec4<f32>(rgb, a);
}

fn with_coverage(colour: vec4<f32>, coverage: f32) -> vec4<f32> {
    return vec4<f32>(colour.rgb, colour.a * clamp(coverage, 0.0, 1.0));
}

fn trace(dst: vec4<f32>, s: u32, colour: vec4<f32>, p: vec2<f32>, row: i32) -> vec4<f32> {
    let d = polyline_distance(s, p, row);
    return over(dst, with_coverage(colour, 1.0 - d));
}

fn background(p: vec2<f32>) -> vec4<f32> {
    let local = floor(p - c.rect.xy);
    if (c.background == 3u) {
        let cell = max(8.0, floor(c.rect.z / 6.0));
        let k = vec2<i32>(floor(local / cell));
        if (((k.x + k.y) & 1) == 1) {
            return c.chess;
        }
        return vec4<f32>(0.0);
    }
    if (c.background == 0u) {
        return vec4<f32>(0.0);
    }
    var col = vec4<f32>(0.0);
    // A vertical line at every step of the level scale.
    let t = (p.x - 0.5 - c.base_x) * c.dir / c.amp;
    let db = c.floor_db + t * c.span_db;
    let nearest = round(db / c.db_step) * c.db_step;
    if (nearest >= ceil(c.floor_db / c.db_step) * c.db_step - 0.001
        && nearest <= c.floor_db + c.span_db + 0.001
        && floor(x_for(nearest)) == floor(p.x)) {
        col = c.grid;
    }
    if (c.background == 2u) {
        // Seven rows across, dividing the strip in eight.
        let h = i32(c.rect.w);
        for (var i = 1; i < 8; i++) {
            if (i32(local.y) == h * i / 8) {
                col = c.grid;
            }
        }
    }
    return col;
}

// Bars and LED segments: the rows grouped in fixed-height bars from the bottom, each
// as long as the loudest row in it, coloured along the palette by that level.
fn bars(p: vec2<f32>) -> vec4<f32> {
    let n = i32(c.n);
    let size = i32(max(c.bar_size, 2u));
    let count = max(1, n / size);
    let y = i32(floor(p.y - c.rect.y));
    let from_bottom = n - 1 - y;
    let b = from_bottom / size;
    if (from_bottom < 0 || b >= count) {
        return vec4<f32>(0.0);
    }
    let first = b * size;
    let end = min(n, first + size);
    // The bottom row of every bar is the gap to the next.
    let h = max(1, end - first - 1);
    if (y < n - end || y >= n - end + h) {
        return vec4<f32>(0.0);
    }
    var peak = -140.0;
    for (var i = first; i < end; i++) {
        peak = max(peak, textureLoad(series, vec2<i32>(i, i32(c.first_row)), 0).r);
    }
    let t = clamp((peak - c.floor_db) / c.span_db, 0.0, 1.0);
    let len = i32(t * c.amp);
    if (len < 1) {
        return vec4<f32>(0.0);
    }
    // Pixels out from the base, 0 against it.
    let px = i32(floor(p.x));
    let base = i32(c.base_x);
    var o = px - base;
    if (c.dir < 0.0) {
        o = base - 1 - px;
    }
    if (o < 0 || o >= len) {
        return vec4<f32>(0.0);
    }
    if (c.style == 2u) {
        let seg = i32(max(c.led_segment, 3u));
        let pitch = seg + 2;
        let start = o - o % pitch;
        if (o % pitch >= seg || start + seg > len) {
            return vec4<f32>(0.0);
        }
    }
    let colour = textureLoad(palette, vec2<i32>(clamp(i32((0.35 + 0.6 * t) * 255.0), 0, 255), 0), 0);
    return vec4<f32>(colour.rgb, 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = pos.xy;
    let row = i32(floor(p.y - c.rect.y));
    var col = background(p);

    if (c.style == 0u) {
        // The area between the base and the curve, as the curve runs through this
        // pixel's centre.
        if ((c.flags & 1u) != 0u) {
            let yc = p.y - c.rect.y - 0.5;
            let y0 = i32(floor(yc));
            let x = mix(x_for(value(DISPLAY, y0)), x_for(value(DISPLAY, y0 + 1)), yc - f32(y0));
            let inside = clamp((x - p.x) * c.dir + 0.5, 0.0, 1.0);
            let beyond_base = clamp((p.x - (c.base_x + 0.5)) * c.dir + 0.5, 0.0, 1.0);
            let g = clamp((p.x - c.rect.x) / max(c.rect.z, 1.0), 0.0, 1.0);
            let fill = mix(c.fill_left, c.fill_right, g);
            col = over(col, with_coverage(fill, min(inside, beyond_base)));
        }
        let d = polyline_distance(DISPLAY, p, row);
        col = over(col, with_coverage(c.line, c.line_width * 0.5 + 0.5 - d));
    } else {
        col = over(col, bars(p));
    }

    if ((c.flags & 2u) != 0u) {
        col = trace(col, PEAK, c.peak, p, row);
    }
    if ((c.flags & 4u) != 0u) {
        col = trace(col, AVERAGE, c.average, p, row);
    }
    if ((c.flags & 8u) != 0u) {
        col = trace(col, MINIMUM, c.minimum, p, row);
    }
    if ((c.flags & 16u) != 0u) {
        col = trace(col, REFERENCE, c.reference, p, row);
    }
    return col;
}
