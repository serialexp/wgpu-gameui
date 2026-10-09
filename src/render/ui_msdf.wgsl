// wgpu-gameui MSDF text shader.
//
// Renders glyphs from a multi-channel signed distance field atlas. The fill mask
// is reconstructed as `median(r, g, b)` (Chlumský) and anti-aliased in screen
// space via the `screenPxRange()` technique (msdf-atlas-gen): the field's pixel
// range is converted to a screen-pixel width using the uv derivatives, so AA is
// correct at any display size or transform scale.
//
// Effects (one quad, one draw):
//   * fill    — coverage of the glyph interior (`dist >= 0`).
//   * outline — the glyph grown outward by `outline_width` screen px, composited
//     *under* the fill. Disabled when `outline.a == 0`.
//   * softness — extra AA spread (screen px). 0 = crisp text; > 0 spreads the edge
//     for soft drop-shadows / glow. Shadow and glow are emitted as separate,
//     offset, behind quads by the CPU side (see text.rs) using these same fields.
//
// Per-vertex scissor clipping matches the other UI pipelines (ui.wgsl).
//
// Crisp edges (softness 0) are corrected for their colour so light text on a
// dark background looks as heavy as dark text on a light one; see
// `corrected_coverage` and `render/text_contrast.rs`.
//
// NOTE: effect reach is bounded by the field's valid range — about
// `(px_range / 2) * (font_size / ref_px)` screen px around the edge. Past that the
// field saturates, so very thick outlines / wide blur at tiny font sizes clip.
// `MsdfGlyphAtlas` picks `ref_px`/`px_range` to give a few px of headroom at UI sizes.

struct Uniforms {
    view_proj: mat4x4<f32>,
    // `TextContrast`: the gamma ratios, and the enhanced contrast in `x`.
    gamma_ratios: vec4<f32>,
    contrast: vec4<f32>,
};
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

@group(1) @binding(0) var atlas_tex: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

struct VsIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) fill: vec4<f32>,
    @location(3) clip: vec4<f32>,
    @location(4) clip_enabled: f32,
    // Distance-ramp width of the field in *atlas* texels (constant per atlas).
    @location(5) px_range: f32,
    @location(6) outline: vec4<f32>,
    @location(7) outline_width: f32,
    @location(8) softness: f32,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) fill: vec4<f32>,
    @location(2) clip: vec4<f32>,
    @location(3) clip_enabled: f32,
    @location(4) frag_pos: vec2<f32>,
    @location(5) px_range: f32,
    @location(6) outline: vec4<f32>,
    @location(7) outline_width: f32,
    @location(8) softness: f32,
};

@vertex
fn vs_msdf(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip_position = uniforms.view_proj * vec4<f32>(in.position, 0.0, 1.0);
    out.uv = in.uv;
    out.fill = in.fill;
    out.clip = in.clip;
    out.clip_enabled = in.clip_enabled;
    out.frag_pos = in.position;
    out.px_range = in.px_range;
    out.outline = in.outline;
    out.outline_width = in.outline_width;
    out.softness = in.softness;
    return out;
}

fn median(r: f32, g: f32, b: f32) -> f32 {
    return max(min(r, g), min(max(r, g), b));
}

// Contrast and gamma correction adapted from Windows Terminal's dwrite.hlsl
// (Copyright (c) Microsoft Corporation, MIT licence), by way of gpui.
// The CPU twin is `corrected_coverage` in render/text_contrast.rs; keep them
// equal. `rgb` is the sRGB-encoded text colour.
fn corrected_coverage(coverage: f32, rgb: vec3<f32>) -> f32 {
    let g = uniforms.gamma_ratios;
    let b = dot(rgb, vec3<f32>(0.30, 0.59, 0.11));
    // Enhanced contrast thickens text darker than 75% brightness.
    let k = uniforms.contrast.x * saturate(4.0 * (0.75 - b));
    let a = coverage * (k + 1.0) / (coverage * k + 1.0);
    // Gamma moves partial coverage by a·(1-a)·correction.
    let correction = (g.x * b + g.y) * a + (g.z * b + g.w);
    return saturate(a + a * (1.0 - a) * correction);
}

@fragment
fn fs_msdf(in: VsOut) -> @location(0) vec4<f32> {
    if (in.clip_enabled > 0.5) {
        let p = in.frag_pos;
        if (p.x < in.clip.x || p.x > in.clip.x + in.clip.z
            || p.y < in.clip.y || p.y > in.clip.y + in.clip.w) {
            discard;
        }
    }

    let msd = textureSample(atlas_tex, atlas_sampler, in.uv).rgb;
    let sd = median(msd.r, msd.g, msd.b);

    // Convert the atlas-space distance range to a screen-pixel range using the uv
    // derivatives. Signed distance in *screen px*, positive inside the glyph.
    let tex_size = vec2<f32>(textureDimensions(atlas_tex));
    let unit_range = vec2<f32>(in.px_range) / tex_size;
    let screen_tex_size = vec2<f32>(1.0) / fwidth(in.uv);
    let screen_px_range = max(0.5 * dot(unit_range, screen_tex_size), 1.0);
    let dist = screen_px_range * (sd - 0.5);

    // AA half-width in screen px; `softness` widens it for shadows / glow.
    let aa = 0.5 + in.softness;
    var fill_cov = clamp(dist / (2.0 * aa) + 0.5, 0.0, 1.0);
    var outline_cov = clamp((dist + in.outline_width) / (2.0 * aa) + 0.5, 0.0, 1.0);
    // Only crisp edges are anti-aliasing; a soft edge is a shadow or glow's
    // falloff and keeps its shape.
    if (in.softness <= 0.0) {
        fill_cov = corrected_coverage(fill_cov, in.fill.rgb);
        outline_cov = corrected_coverage(outline_cov, in.outline.rgb);
    }

    let fill_a = in.fill.a * fill_cov;
    let outline_a = in.outline.a * outline_cov;

    // Composite fill OVER outline in premultiplied space, then un-premultiply so
    // AA edges never darken toward black when there is no outline.
    let premul = in.fill.rgb * fill_a + in.outline.rgb * (outline_a * (1.0 - fill_a));
    let a = fill_a + outline_a * (1.0 - fill_a);
    if (a <= 0.0) {
        discard;
    }
    return vec4<f32>(premul / a, a);
}
