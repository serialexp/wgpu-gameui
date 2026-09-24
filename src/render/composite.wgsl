// Composite the UI's offscreen layer onto a host target that stores linear
// light (an `*Srgb` or float format).
//
// The UI is drawn into a non-sRGB offscreen texture cleared to transparent, with
// straight `ALPHA_BLENDING`. Starting from transparent black that produces
// premultiplied, sRGB-encoded colour plus a correct "over" alpha — i.e. every
// blend happened in sRGB space, the way a browser does it. This pass
// un-premultiplies, decodes to linear light, and blends the result over the
// host's existing contents.

@group(0) @binding(0) var layer: texture_2d<f32>;

// Fullscreen triangle: 0=(-1,-1), 1=(3,-1), 2=(-1,3).
@vertex
fn vs_composite(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vi & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_composite(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = textureLoad(layer, vec2<i32>(pos.xy), 0);
    if (p.a <= 0.0) {
        // Untouched pixel: a zero-alpha source leaves the host unchanged.
        return vec4<f32>(0.0);
    }
    let straight = clamp(p.rgb / p.a, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(srgb_to_linear(straight), p.a);
}
