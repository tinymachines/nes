// The picture on the GPU: ntsc-crt's three-line comb decode and its
// five CRT stages as compute passes, each the CPU stage's arithmetic
// in the CPU stage's order, held to it in tests/gpu.rs. Every constant
// arrives in `Params` from the decoder and CrtParams instances.

struct Params {
    n: u32,        // samples per line
    nd: u32,       // decimated samples per line
    d: u32,        // decimation
    rows: u32,     // decoded rows
    row0: u32,     // first decoded line
    width: u32,    // decoded samples per row
    taps: u32,     // uv lowpass taps
    uv_half: u32,
    out_w: u32,    // 256 * scale
    out_h: u32,    // 240 * scale
    scale: u32,
    flags: u32,    // 1 mask, 2 geometry, 4 persistence has state
    black: f32,
    scale_y: f32,
    amp_k: f32,
    gamma: f32,
    r_from_v: f32,
    g_from_u: f32,
    g_from_v: f32,
    b_from_u: f32,
    comb: vec4<f32>,
    beam_sigma: f32,
    beam_ratio: f32,
    beam_reach: i32,
    scan_base: f32,
    scan_bloom: f32,
    scan_reach: i32,
    mask_pitch: u32,
    mask_off: f32,
    decay: vec4<f32>,
    barrel_k: f32,
    corner_r: f32,
    pad0: f32,
    pad1: f32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> samples: array<f32>;
@group(0) @binding(2) var<storage, read> lines: array<vec4<u32>>;   // per row: line, start, p0, 0
@group(0) @binding(3) var<storage, read> sincos: array<vec4<f32>>;  // 12: sin, cos
@group(0) @binding(4) var<storage, read> taps: array<f32>;
@group(0) @binding(5) var<storage, read_write> dec: array<vec2<f32>>;
@group(0) @binding(6) var<storage, read_write> lp: array<vec2<f32>>;
@group(0) @binding(7) var<storage, read_write> grid: array<vec4<f32>>;
@group(1) @binding(0) var<storage, read_write> beam: array<vec4<f32>>;
@group(1) @binding(1) var<storage, read_write> scan: array<vec4<f32>>;
@group(1) @binding(2) var<storage, read_write> state: array<vec4<f32>>;
@group(1) @binding(3) var<storage, read_write> pers: array<vec4<f32>>;
@group(1) @binding(4) var<storage, read_write> masked: array<vec4<f32>>;
@group(1) @binding(5) var<storage, read_write> final_out: array<vec4<f32>>;

fn comb_luma(line: u32, i: u32) -> f32 {
    let prev = samples[(line - 1u) * p.n + i];
    let cur = samples[line * p.n + i];
    let next = samples[(line + 1u) * p.n + i];
    return p.comb.x * prev + p.comb.y * cur + p.comb.z * next;
}

// Pass 1: the comb's chroma, demodulated at the line's phase, block
// averaged by the decimation.
@compute @workgroup_size(64)
fn decimate(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = gid.x;
    if idx >= p.rows * p.nd { return; }
    let row = idx / p.nd;
    let j = idx % p.nd;
    let line = lines[row].x;
    let p0 = lines[row].z;
    let a0 = j * p.d;
    let b0 = min(a0 + p.d, p.n);
    var u = 0.0;
    var v = 0.0;
    for (var i = a0; i < b0; i = i + 1u) {
        let cur = samples[line * p.n + i];
        let chroma = cur - comb_luma(line, i);
        let a = chroma * p.amp_k;
        let sc = sincos[(p0 + i) % 12u];
        u = u + a * sc.x;
        v = v + a * sc.y;
    }
    let cnt = f32(b0 - a0);
    dec[idx] = vec2<f32>(u / cnt, v / cnt);
}

// Pass 2: the decimated lowpass, edges replicated.
@compute @workgroup_size(64)
fn uv_filter(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = gid.x;
    if idx >= p.rows * p.nd { return; }
    let row = idx / p.nd;
    let j = i32(idx % p.nd);
    var acc = vec2<f32>(0.0, 0.0);
    for (var k = 0u; k < p.taps; k = k + 1u) {
        let src = clamp(j + i32(k) - i32(p.uv_half), 0, i32(p.nd) - 1);
        acc = acc + taps[k] * dec[row * p.nd + u32(src)];
    }
    lp[idx] = acc;
}

fn catmull(row: u32, x: f32) -> vec2<f32> {
    let jf = floor(x);
    let t = x - jf;
    let j = i32(jf);
    let last = i32(p.nd) - 1;
    let p0 = lp[row * p.nd + u32(clamp(j - 1, 0, last))];
    let p1 = lp[row * p.nd + u32(clamp(j, 0, last))];
    let p2 = lp[row * p.nd + u32(clamp(j + 1, 0, last))];
    let p3 = lp[row * p.nd + u32(clamp(j + 2, 0, last))];
    return 0.5 * (2.0 * p1
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t * t * t);
}

// Pass 3: luma scaled, chroma interpolated back, the matrix, the clamp,
// the display gamma: linear RGB on the sample grid.
@compute @workgroup_size(64)
fn rgb(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = gid.x;
    if idx >= p.rows * p.width { return; }
    let row = idx / p.width;
    let x = idx % p.width;
    let line = lines[row].x;
    let start = lines[row].y;
    let s = start + x;
    let y = (comb_luma(line, s) - p.black) * p.scale_y;
    let uv = catmull(row, f32(s) / f32(p.d));
    let r = y + p.r_from_v * uv.y;
    let g = y + p.g_from_u * uv.x + p.g_from_v * uv.y;
    let b = y + p.b_from_u * uv.x;
    let c = vec3<f32>(pow(clamp(r, 0.0, 1.0), p.gamma), pow(clamp(g, 0.0, 1.0), p.gamma), pow(clamp(b, 0.0, 1.0), p.gamma));
    grid[idx] = vec4<f32>(c, 1.0);
}

// Pass 4: the beam. Gaussian columns of the grid into output pixels.
@compute @workgroup_size(64)
fn beam_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = gid.x;
    if idx >= p.rows * p.out_w { return; }
    let row = idx / p.out_w;
    let x = idx % p.out_w;
    let center = (f32(x) + 0.5) * p.beam_ratio - 0.5;
    let c0 = i32(floor(center + 0.5));
    var acc = vec3<f32>(0.0, 0.0, 0.0);
    var wsum = 0.0;
    let s2 = 2.0 * p.beam_sigma * p.beam_sigma;
    for (var i = c0 - p.beam_reach; i <= c0 + p.beam_reach; i = i + 1) {
        let dd = f32(i) - center;
        let wt = exp(-dd * dd / s2);
        let j = u32(clamp(i, 0, i32(p.width) - 1));
        acc = acc + wt * grid[row * p.width + j].xyz;
        wsum = wsum + wt;
    }
    beam[idx] = vec4<f32>(acc / wsum, 1.0);
}

// Pass 5: scanlines, gathered: every input line whose Gaussian reaches
// this output row, in line order, the CPU's 1e-4 cutoff kept.
@compute @workgroup_size(64)
fn scanlines(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = gid.x;
    if idx >= p.out_h * p.out_w { return; }
    let y = i32(idx / p.out_w);
    let x = idx % p.out_w;
    let s = i32(p.scale);
    var acc = vec3<f32>(0.0, 0.0, 0.0);
    let lo = max((y - p.scan_reach) / s - 1, 0);
    let hi = min((y + p.scan_reach) / s + 1, i32(p.rows) - 1);
    for (var line = lo; line <= hi; line = line + 1) {
        let center = (f32(line) + 0.5) * f32(s) - 0.5;
        let y0 = i32(floor(center + 0.5));
        if y < y0 - p.scan_reach || y > y0 + p.scan_reach { continue; }
        let px = beam[u32(line) * p.out_w + x].xyz;
        let v = clamp(0.299 * px.x + 0.587 * px.y + 0.114 * px.z, 0.0, 1.0);
        let sigma = p.scan_base * (1.0 + p.scan_bloom * v);
        let dd = f32(y) - center;
        let wt = exp(-dd * dd / (2.0 * sigma * sigma));
        if wt < 1e-4 { continue; }
        acc = acc + wt * px;
    }
    scan[idx] = vec4<f32>(acc, 1.0);
}

// Pass 6: persistence. The screen holds the larger of the excitation
// and the decayed previous frame; the state is what it held.
@compute @workgroup_size(64)
fn persist(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = gid.x;
    if idx >= p.out_h * p.out_w { return; }
    var v = scan[idx].xyz;
    if (p.flags & 4u) != 0u {
        let held = state[idx].xyz * p.decay.xyz;
        v = max(v, held);
    }
    state[idx] = vec4<f32>(v, 1.0);
    pers[idx] = vec4<f32>(v, 1.0);
}

// Pass 7: the mask, when on.
@compute @workgroup_size(64)
fn mask(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = gid.x;
    if idx >= p.out_h * p.out_w { return; }
    var v = pers[idx].xyz;
    if (p.flags & 1u) != 0u {
        let x = idx % p.out_w;
        let on = (x / p.mask_pitch) % 3u;
        if on != 0u { v.x = v.x * p.mask_off; }
        if on != 1u { v.y = v.y * p.mask_off; }
        if on != 2u { v.z = v.z * p.mask_off; }
    }
    masked[idx] = vec4<f32>(v, 1.0);
}

// Pass 8: geometry, when on: barrel and corners, bilinear.
@compute @workgroup_size(64)
fn geometry(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = gid.x;
    if idx >= p.out_h * p.out_w { return; }
    if (p.flags & 2u) == 0u {
        final_out[idx] = masked[idx];
        return;
    }
    let w = f32(p.out_w);
    let h = f32(p.out_h);
    let cx = w / 2.0;
    let cy = h / 2.0;
    let half_diag = sqrt(cx * cx + cy * cy);
    let x = f32(idx % p.out_w);
    let y = f32(idx / p.out_w);
    let dx = x + 0.5 - cx;
    let dy = y + 0.5 - cy;
    let r2 = (dx * dx + dy * dy) / (half_diag * half_diag);
    let sx = cx + dx * (1.0 + p.barrel_k * r2) - 0.5;
    let sy = cy + dy * (1.0 + p.barrel_k * r2) - 0.5;
    let ex = max(abs(dx) - (cx - p.corner_r), 0.0);
    let ey = max(abs(dy) - (cy - p.corner_r), 0.0);
    if sqrt(ex * ex + ey * ey) > p.corner_r || sx < 0.0 || sy < 0.0 || sx > w - 1.0 || sy > h - 1.0 {
        final_out[idx] = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        return;
    }
    let x0 = u32(sx);
    let y0 = u32(sy);
    let x1 = min(x0 + 1u, p.out_w - 1u);
    let y1 = min(y0 + 1u, p.out_h - 1u);
    let tx = sx - f32(x0);
    let ty = sy - f32(y0);
    let f00 = masked[y0 * p.out_w + x0].xyz;
    let f10 = masked[y0 * p.out_w + x1].xyz;
    let f01 = masked[y1 * p.out_w + x0].xyz;
    let f11 = masked[y1 * p.out_w + x1].xyz;
    let top = f00 + (f10 - f00) * tx;
    let bot = f01 + (f11 - f01) * tx;
    final_out[idx] = vec4<f32>(top + (bot - top) * ty, 1.0);
}
