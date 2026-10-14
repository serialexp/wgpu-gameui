// Separable Gaussian backdrop blur — one shader, used for both passes.
//
// Pass A (horizontal): samples the app-provided scene texture over the blur
// region's UV sub-rect, writes the horizontally-blurred result into a (possibly
// downsampled) intermediate texture.
// Pass B (vertical): samples the intermediate over its full 0..1 UV, writes the
// vertically-blurred result into the UI target at the region's NDC rect.
//
// The CPU fills `out_rect` (NDC corners of the output quad) and `in_uv` (UV
// corners to read), so the vertex shader needs no vertex buffer.

struct BlurUniforms {
    // Output quad in NDC: x0, y0(top), x1, y1(bottom).
    out_rect: vec4<f32>,
    // Input read rect in UV: u0, v0(top), u1, v1(bottom).
    in_uv: vec4<f32>,
    // Per-tap UV offset along the blur axis (one source texel).
    dir_step: vec2<f32>,
    // Blur radius in taps (texels) for this pass.
    radius: f32,
    // Output conversion (pass B): 1 = encode linear -> sRGB before the tint,
    // 2 = decode sRGB -> linear after it (the target stores linear light).
    flags: u32,
    // RGBA multiplied into the output, in sRGB space (scrim/darken; white =
    // passthrough).
    tint: vec4<f32>,
};

const FLAG_ENCODE: u32 = 1u;
const FLAG_DECODE: u32 = 2u;

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let lo = x * 12.92;
    let hi = 1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, x <= vec3<f32>(0.0031308));
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let lo = x / 12.92;
    let hi = pow((x + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, x <= vec3<f32>(0.04045));
}

@group(0) @binding(0) var<uniform> u: BlurUniforms;
@group(1) @binding(0) var src_tex: texture_2d<f32>;
@group(1) @binding(1) var src_sampler: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Triangle strip, 4 verts: 0=TL, 1=BL, 2=TR, 3=BR.
@vertex
fn vs_blur(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let left = (vi == 0u || vi == 1u);
    let top = (vi == 0u || vi == 2u);
    let x = select(u.out_rect.z, u.out_rect.x, left);
    let y = select(u.out_rect.w, u.out_rect.y, top);
    let uu = select(u.in_uv.z, u.in_uv.x, left);
    let vv = select(u.in_uv.w, u.in_uv.y, top);
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(uu, vv);
    return out;
}

const MAX_TAPS: i32 = 16;

@fragment
fn fs_blur(in: VsOut) -> @location(0) vec4<f32> {
    let sigma = max(u.radius * 0.5, 0.0001);
    let two_s2 = 2.0 * sigma * sigma;
    var acc = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    var wsum = 0.0;
    // Fixed loop bound keeps texture sampling under uniform control flow; taps
    // beyond the radius are masked to zero weight rather than skipped.
    for (var i: i32 = -MAX_TAPS; i <= MAX_TAPS; i = i + 1) {
        let fi = f32(i);
        let in_radius = select(0.0, 1.0, abs(fi) <= u.radius + 0.5);
        let w = exp(-(fi * fi) / two_s2) * in_radius;
        let uv = in.uv + u.dir_step * fi;
        acc = acc + textureSample(src_tex, src_sampler, uv) * w;
        wsum = wsum + w;
    }
    var color = acc / max(wsum, 0.0001);
    if ((u.flags & FLAG_ENCODE) != 0u) {
        color = vec4<f32>(linear_to_srgb(color.rgb), color.a);
    }
    color = color * u.tint;
    if ((u.flags & FLAG_DECODE) != 0u) {
        color = vec4<f32>(srgb_to_linear(color.rgb), color.a);
    }
    return color;
}
