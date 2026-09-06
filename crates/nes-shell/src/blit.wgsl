// The window's blit: a full-screen triangle sampling the picture's
// final buffer, gamma-encoded the way ntsc-crt's to_rgba8 does (display
// gamma 2.2), letterboxed at the integer scale.

struct Blit {
    out_w: u32,
    out_h: u32,
    win_w: u32,
    win_h: u32,
}

@group(0) @binding(0) var<uniform> b: Blit;
@group(0) @binding(1) var<storage, read> picture: array<vec4<f32>>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    out.pos = vec4<f32>(x, -y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (y + 1.0) * 0.5);
    return out;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    // Centre the picture at one window pixel per output pixel.
    let px = in.uv * vec2<f32>(f32(b.win_w), f32(b.win_h));
    let ox = (f32(b.win_w) - f32(b.out_w)) * 0.5;
    let oy = (f32(b.win_h) - f32(b.out_h)) * 0.5;
    let x = px.x - ox;
    let y = px.y - oy;
    if x < 0.0 || y < 0.0 || x >= f32(b.out_w) || y >= f32(b.out_h) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let c = picture[u32(y) * b.out_w + u32(x)].xyz;
    let g = pow(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2));
    return vec4<f32>(g, 1.0);
}
