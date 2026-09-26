// wgpu-gameui main UI shader.
//
// Entry points share an ortho-projection uniform (group 0); the textured paths
// also bind the atlas (group 1):
//   - `vs_color`/`fs_color`: colored quads (DrawList vertices); supports per-vertex
//     scissor via the (clip, clip_enabled) attributes.
//   - `vs_analytic`/`fs_analytic`: one tagged, ordered instance stream for SDF
//     rounded-rect chrome and full-affine analytic shadows.
//   - `vs_circle`/`fs_circle`: instanced SDF circle (filled disc + ring outline).
//   - `vs_icon`/`fs_icon`: instanced textured quads (icons, sprites, images);
//     corners baked per-instance, bilinearly interpolated, atlas × tint.
//   - `vs_nine_slice`/`fs_nine_slice`: instanced nine-slice panels; the fragment
//     remaps local coords into the source UV (nine-region piecewise map).
//
// Clipping is done per-pixel against the per-vertex/instance clip rect (matches
// the DrawList::push_clip API). When `clip_enabled <= 0.5`, the rect is ignored.

struct Uniforms {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

// ---- Colored quad path ---------------------------------------------------

struct ColorVsIn {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) clip: vec4<f32>,
    @location(3) clip_enabled: f32,
};

struct ColorVsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) clip: vec4<f32>,
    @location(2) clip_enabled: f32,
    @location(3) frag_pos: vec2<f32>,
};

@vertex
fn vs_color(in: ColorVsIn) -> ColorVsOut {
    var out: ColorVsOut;
    out.clip_position = uniforms.view_proj * vec4<f32>(in.position, 0.0, 1.0);
    out.color = in.color;
    out.clip = in.clip;
    out.clip_enabled = in.clip_enabled;
    out.frag_pos = in.position;
    return out;
}

@fragment
fn fs_color(in: ColorVsOut) -> @location(0) vec4<f32> {
    if (in.clip_enabled > 0.5) {
        let p = in.frag_pos;
        if (p.x < in.clip.x || p.x > in.clip.x + in.clip.z
            || p.y < in.clip.y || p.y > in.clip.y + in.clip.w) {
            discard;
        }
    }
    return in.color;
}

// ---- Instanced chrome path (SDF rounded rect) ----------------------------
//
// One unit-quad base mesh, one instance per button-like "chrome" rect. The
// fragment computes a rounded-rect signed distance, so fill + border + crisp
// anti-aliased corners come from a single instanced draw regardless of size.
// Replaces re-tessellating identical button geometry into the vertex soup every
// frame (see benches/ui_stress.rs / the instancing work).

struct AnalyticVsIn {
    @location(0) corner: vec2<f32>,
    @location(1) p0: vec4<f32>, @location(2) p1: vec4<f32>,
    @location(3) p2: vec4<f32>, @location(4) p3: vec4<f32>,
    @location(5) p4: vec4<f32>, @location(6) p5: vec4<f32>,
    @location(7) p6: vec4<f32>, @location(8) p7: vec4<f32>,
    @location(9) p8: vec4<f32>, @location(10) p9: vec4<f32>,
    @location(11) kind: u32,
};

struct AnalyticVsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) world: vec2<f32>,
    @location(2) @interpolate(flat) p0: vec4<f32>,
    @location(3) @interpolate(flat) p1: vec4<f32>,
    @location(4) @interpolate(flat) p2: vec4<f32>,
    @location(5) @interpolate(flat) p3: vec4<f32>,
    @location(6) @interpolate(flat) p4: vec4<f32>,
    @location(7) @interpolate(flat) p5: vec4<f32>,
    @location(8) @interpolate(flat) p6: vec4<f32>,
    @location(9) @interpolate(flat) p7: vec4<f32>,
    @location(10) @interpolate(flat) p8: vec4<f32>,
    @location(11) @interpolate(flat) p9: vec4<f32>,
    @location(12) @interpolate(flat) kind: u32,
};

@vertex
fn vs_analytic(in: AnalyticVsIn) -> AnalyticVsOut {
    var out: AnalyticVsOut;
    // `p0` is the forward linear part [a, b, c, d] of the affine, row-major
    // like `Affine2` (x' = a·x + b·y + tx, y' = c·x + d·y + ty); `p1.xy` is
    // the translation and `p2` the local rect.
    var local: vec2<f32>;
    if (in.kind == 0u) {
        // Chrome. The edge's anti-aliasing ramp reaches ~1px past the rect, so
        // under rotation/scale pad the quad by 2 screen px (converted to local
        // units per axis) or the outer half of that ramp is never rasterized
        // and the edges look hard. Translate-only chrome keeps its exact quad.
        var pad = vec2<f32>(0.0);
        if (any(in.p0 != vec4<f32>(1.0, 0.0, 0.0, 1.0))) {
            let axis_scale = vec2<f32>(length(in.p0.xz), length(in.p0.yw));
            pad = vec2<f32>(2.0) / max(axis_scale, vec2<f32>(1e-4));
        }
        let cell = in.corner * (in.p2.zw + 2.0 * pad) - pad;
        local = in.p2.xy + cell;
        out.local = cell;
    } else {
        local = in.p2.xy + in.corner * in.p2.zw;
        out.local = local;
    }
    let world = vec2<f32>(in.p0.x * local.x + in.p0.y * local.y + in.p1.x,
                          in.p0.z * local.x + in.p0.w * local.y + in.p1.y);
    out.clip_position = uniforms.view_proj * vec4<f32>(world, 0.0, 1.0);
    out.world = world;
    out.p0 = in.p0; out.p1 = in.p1; out.p2 = in.p2; out.p3 = in.p3;
    out.p4 = in.p4; out.p5 = in.p5; out.p6 = in.p6; out.p7 = in.p7;
    out.p8 = in.p8; out.p9 = in.p9; out.kind = in.kind;
    return out;
}

fn corner_radius(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    if (p.y < size.y * 0.5) {
        return select(radii.y, radii.x, p.x < size.x * 0.5);
    }
    return select(radii.z, radii.w, p.x < size.x * 0.5);
}

fn chrome_rounded_rect_distance(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half = size * 0.5;
    let radius = corner_radius(p, size, radii);
    let q = abs(p - half) - half + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

// Distance to a rectangle whose corner arcs may be elliptical. This is used for
// the padding edge: unequal adjacent border widths turn a circular outer corner
// into an elliptical inner corner rather than overlapping rectangular bands.
fn inner_distance(p: vec2<f32>, size: vec2<f32>, outer_radii: vec4<f32>, widths: vec4<f32>) -> f32 {
    let clamped_size = max(size, vec2<f32>(0.0));
    let half = clamped_size * 0.5;
    let left = p.x < half.x;
    let top = p.y < half.y;
    var outer_radius: f32;
    var inset: vec2<f32>;
    if (top) {
        outer_radius = select(outer_radii.y, outer_radii.x, left);
        inset = vec2<f32>(select(widths.y, widths.w, left), widths.x);
    } else {
        outer_radius = select(outer_radii.z, outer_radii.w, left);
        inset = vec2<f32>(select(widths.y, widths.w, left), widths.z);
    }
    let radius = max(vec2<f32>(outer_radius) - inset, vec2<f32>(0.0));
    let q = abs(p - half) - half + radius;
    let outside = max(q, vec2<f32>(0.0));
    let safe_radius = max(radius, vec2<f32>(1e-4));
    let ellipse = (length(outside / safe_radius) - 1.0) * min(safe_radius.x, safe_radius.y);
    let square = length(outside) + min(max(q.x, q.y), 0.0);
    return select(ellipse, square, radius.x <= 1e-4 || radius.y <= 1e-4);
}

fn shade_chrome(in: AnalyticVsOut) -> vec4<f32> {
    if (in.p1.z > 0.5 && (in.world.x < in.p8.x || in.world.x > in.p8.x + in.p8.z
        || in.world.y < in.p8.y || in.world.y > in.p8.y + in.p8.w)) { discard; }
    let size = in.p2.zw;
    let outer_d = chrome_rounded_rect_distance(in.local, size, in.p6);
    let outer_aa = max(fwidth(outer_d), 1e-4);
    let outer = 1.0 - smoothstep(-outer_aa, outer_aa, outer_d);
    let inner_origin = vec2<f32>(in.p7.w, in.p7.x);
    let inner_size = size - vec2<f32>(in.p7.w + in.p7.y, in.p7.x + in.p7.z);
    let inner_d = inner_distance(in.local - inner_origin, inner_size, in.p6, in.p7);
    let inner_aa = max(fwidth(inner_d), 1e-4);
    var inner = 1.0 - smoothstep(-inner_aa, inner_aa, inner_d);
    if (inner_size.x <= 0.0 || inner_size.y <= 0.0) { inner = 0.0; }
    let gradient_coord = select(in.local.y / max(size.y, 1e-4), in.local.x / max(size.x, 1e-4), in.p1.w > 0.5);
    let fill = mix(in.p3, in.p4, vec4<f32>(clamp(gradient_coord, 0.0, 1.0)));
    let color = mix(in.p5, fill, inner);
    let alpha = outer * color.a;
    if (alpha <= 0.0) { discard; }
    return vec4<f32>(color.rgb, alpha);
}

// ---- Instanced circle path (SDF disc / ring) -----------------------------
//
// One unit-quad base mesh, one instance per circle/circle-outline. The vertex
// shader expands the quad over the circle's bounding box (center ± extent) and
// the fragment computes a signed distance from the center, giving a smooth AA
// filled disc (thickness <= 0) or a ring centered on the radius path
// (thickness > 0). Replaces re-tessellating a 16-64-segment fan into the vertex
// soup every frame. Adapted from citybuilder's sdf_circle.wgsl.

struct CircleVsIn {
    // Base mesh: unit-quad corner in [0,1]^2.
    @location(0) corner: vec2<f32>,
    // Per-instance:
    @location(1) center: vec4<f32>,  // cx, cy, radius, thickness (post-transform world space)
    @location(2) color: vec4<f32>,
    @location(3) clip: vec4<f32>,    // clip rect x, y, w, h
    @location(4) params: vec4<f32>,  // clip_enabled, _pad, _pad, _pad
};

struct CircleVsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) clip: vec4<f32>,
    @location(2) params: vec4<f32>,  // clip_enabled, radius, thickness, _pad
    @location(3) frag_pos: vec2<f32>,
    @location(4) center: vec2<f32>,
};

@vertex
fn vs_circle(in: CircleVsIn) -> CircleVsOut {
    var out: CircleVsOut;
    let center = in.center.xy;
    let radius = in.center.z;
    let thickness = in.center.w;
    // Bounding half-extent covers the outline band plus an AA margin.
    let extent = radius + max(thickness, 0.0) * 0.5 + 2.0;
    let world = center + (in.corner * 2.0 - vec2<f32>(1.0)) * extent;
    out.clip_position = uniforms.view_proj * vec4<f32>(world, 0.0, 1.0);
    out.color = in.color;
    out.clip = in.clip;
    out.params = vec4<f32>(in.params.x, radius, thickness, 0.0);
    out.frag_pos = world;
    out.center = center;
    return out;
}

@fragment
fn fs_circle(in: CircleVsOut) -> @location(0) vec4<f32> {
    // Per-pixel scissor (same convention as fs_color / analytic chrome).
    if (in.params.x > 0.5) {
        let p = in.frag_pos;
        if (p.x < in.clip.x || p.x > in.clip.x + in.clip.z
            || p.y < in.clip.y || p.y > in.clip.y + in.clip.w) {
            discard;
        }
    }

    let radius = in.params.y;
    let thickness = in.params.z;
    // Signed distance to the circle edge (negative inside).
    let dist = length(in.frag_pos - in.center) - radius;
    let aa = max(fwidth(dist), 1e-4);

    var alpha: f32;
    if (thickness <= 0.0) {
        // Filled disc: coverage inside the edge.
        alpha = 1.0 - smoothstep(-aa, aa, dist);
    } else {
        // Ring centered on the radius path, spanning ±thickness/2.
        let half = thickness * 0.5;
        let outer = 1.0 - smoothstep(-aa, aa, dist - half);
        let inner = 1.0 - smoothstep(-aa, aa, dist + half);
        alpha = clamp(outer - inner, 0.0, 1.0);
    }

    let a = alpha * in.color.a;
    if (a <= 0.0) {
        discard;
    }
    return vec4<f32>(in.color.rgb, a);
}

// ---- Full-affine analytic rounded-rectangle shadows -----------------------

fn pick_radius(p: vec2<f32>, center: vec2<f32>, radii: vec4<f32>) -> f32 {
    if (p.y < center.y) { return select(radii.x, radii.y, p.x >= center.x); }
    return select(radii.w, radii.z, p.x >= center.x);
}

fn rounded_rect_distance(p: vec2<f32>, rect: vec4<f32>, radii: vec4<f32>) -> f32 {
    let half = rect.zw * 0.5;
    let center = rect.xy + half;
    let radius = pick_radius(p, center, radii);
    let q = abs(p - center) - half + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

// Integrate the rounded-rectangle indicator over the screen pixel instead of
// treating its SDF as locally linear.  The derivative vectors pull the fixed
// screen-space 4x4 sample grid back through any affine transform, so tiny and
// highly curved shapes retain their area and transformed clips do not acquire
// an axis-aligned approximation.  Keep this fixed-size: it is also used by the
// zero-sigma path, where identical source and element evaluations must cancel
// bit-for-bit.
fn rounded_rect_pixel_coverage(p: vec2<f32>, rect: vec4<f32>, radii: vec4<f32>) -> f32 {
    let pixel_x = dpdx(p);
    let pixel_y = dpdy(p);
    let center_distance = rounded_rect_distance(p, rect, radii);
    // The SDF is 1-Lipschitz. Skip the 16 evaluations whenever the whole
    // pulled-back pixel is provably on one side of the edge; broad shadows then
    // pay supersampling cost only in their narrow clipping boundary band.
    let pixel_radius = 0.5 * (length(pixel_x) + length(pixel_y));
    if (center_distance <= -pixel_radius) { return 1.0; }
    if (center_distance > pixel_radius) { return 0.0; }
    var covered = 0.0;
    for (var iy = 0; iy < 4; iy += 1) {
        let oy = (f32(iy) + 0.5) * 0.25 - 0.5;
        for (var ix = 0; ix < 4; ix += 1) {
            let ox = (f32(ix) + 0.5) * 0.25 - 0.5;
            let sample = p + ox * pixel_x + oy * pixel_y;
            covered += select(0.0, 1.0, rounded_rect_distance(sample, rect, radii) <= 0.0);
        }
    }
    return covered * 0.0625;
}

fn shadow_erf(v: vec2<f32>) -> vec2<f32> {
    let s = sign(v);
    let a = abs(v);
    let r1 = 1.0 + (0.278393 + (0.230389 + (0.000972 + 0.078108 * a) * a) * a) * a;
    let r2 = r1 * r1;
    return s - s / (r2 * r2);
}

fn shadow_gaussian(x: f32, sigma: f32) -> f32 {
    return exp(-(x * x) / (2.0 * sigma * sigma)) / (2.50662827463 * sigma);
}

fn shadow_side_extent(y: f32, radius: f32, half: vec2<f32>) -> f32 {
    let delta = min(half.y - radius - abs(y), 0.0);
    return half.x - radius + sqrt(max(0.0, radius * radius - delta * delta));
}

fn blur_shadow_x(x: f32, y: f32, sigma: f32, radii: vec4<f32>, half: vec2<f32>) -> f32 {
    // Radii are TL, TR, BR, BL. A horizontal source slice can intersect two
    // different corner arcs, so derive each endpoint from its own radius. The
    // sampled source y (not the fragment's quadrant) chooses top versus bottom.
    let left_radius = select(radii.w, radii.x, y < 0.0);
    let right_radius = select(radii.z, radii.y, y < 0.0);
    let left_extent = shadow_side_extent(y, left_radius, half);
    let right_extent = shadow_side_extent(y, right_radius, half);
    let integral = 0.5 + 0.5 * shadow_erf(
        (x + vec2<f32>(-right_extent, left_extent)) * (0.70710678118 / sigma)
    );
    return integral.y - integral.x;
}

fn shade_shadow(in: AnalyticVsOut) -> vec4<f32> {
    if (in.p1.z > 0.5 && (in.world.x < in.p8.x || in.world.x > in.p8.x + in.p8.z
        || in.world.y < in.p8.y || in.world.y > in.p8.y + in.p8.w)) { discard; }

    let sigma_y = in.p9.x;
    let sigma_x_conditional = in.p9.z;
    let conditional_slope = in.p9.w;
    let inset = in.p1.w > 0.5;
    let collapsed = in.p9.y > 0.5;
    var coverage: f32;
    if (sigma_y <= 1e-5 || sigma_x_conditional <= 1e-5) {
        coverage = rounded_rect_pixel_coverage(in.local, in.p3, in.p6);
    } else if (collapsed) {
        coverage = 0.0;
    } else {
        let half = in.p3.zw * 0.5;
        let center = in.p3.xy + half;
        let point = in.local - center;
        let low = point.y - half.y;
        let high = point.y + half.y;
        let start = clamp(-3.0 * sigma_y, low, high);
        let end = clamp(3.0 * sigma_y, low, high);
        let step_size = (end - start) * 0.25;
        coverage = 0.0;
        // For local covariance Σ = sigma² A⁻¹A⁻ᵀ, integrate the y marginal
        // N(0, Σyy). Conditioned on y, x is Gaussian with mean
        // Σxy/Σyy*y and variance Σxx-Σxy²/Σyy. This preserves a fixed loop
        // while making the resulting blur isotropic in browser/screen space.
        for (var i = 0; i < 4; i += 1) {
            let y = start + (f32(i) + 0.5) * step_size;
            coverage += blur_shadow_x(
                point.x - conditional_slope * y,
                point.y - y,
                sigma_x_conditional,
                in.p6,
                half,
            ) * shadow_gaussian(y, sigma_y) * step_size;
        }
    }

    let element_coverage = rounded_rect_pixel_coverage(in.local, in.p4, in.p7);
    if (inset) {
        // The padding-box clips the inverse blurred hole. A collapsed hole is
        // fully covered; otherwise both independently antialiased coverages
        // participate, matching Chromium's inset edge behavior.
        coverage = select(1.0 - coverage, 1.0, collapsed) * element_coverage;
    } else if (sigma_y <= 1e-5 || sigma_x_conditional <= 1e-5) {
        // CSS clips crisp outset shadows by subtracting the border-box shape.
        // In particular, identical zero-blur source and element coverages must
        // cancel exactly rather than leave a multiplied AA fringe.
        coverage = max(coverage - element_coverage, 0.0);
    } else {
        // Chromium's blurred outset edge behaves as independently antialiased
        // shadow coverage clipped by the border box, rather than subtracting
        // the element's fractional edge coverage from the whole blur field.
        coverage *= 1.0 - element_coverage;
    }
    let alpha = clamp(coverage, 0.0, 1.0) * in.p5.a;
    if (alpha <= 0.0) { discard; }
    return vec4<f32>(in.p5.rgb, alpha);
}

@fragment
fn fs_analytic(in: AnalyticVsOut) -> @location(0) vec4<f32> {
    // The kind is flat, so every 2x2 derivative quad follows one uniform branch;
    // fwidth/dpdx/dpdy remain well-defined in both analytic implementations.
    if (in.kind == 0u) { return shade_chrome(in); }
    return shade_shadow(in);
}

// ---- Atlas bindings (shared by the icon + nine-slice paths) --------------

@group(1) @binding(0) var atlas_tex: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

// ---- Instanced icon / image path -----------------------------------------
//
// One unit-quad base mesh, one instance per icon/image (icons, sprites, and
// cropped images all flow through here). The 4 world-space corners are baked
// into the instance (the transform is applied DrawList-side), so the vertex
// shader bilinearly interpolates them by the unit-quad coord — handling
// rotation/scale/shear for free, no fallback. UV is a linear lerp of the
// instance's source rect. Replaces re-tessellating 6 verts/icon into the
// textured soup + re-uploading it every frame.
//
// `flags.y` = tile-wrap: the instance's uv_rect is a span in *tile units*
// (u1/v1 may exceed 1); the fragment shader maps it back into the source
// region modulo its own size, so a single instance repeats the sprite across
// the whole quad (ImageFit::Tile). Nearest-edge texels of the source art are
// sampled at the seams — keep a fully-opaque margin in the art for seamless
// tiling, since atlas neighbors are never sampled (the fract stays inside the
// region's own rect).

struct IconVsIn {
    // Base mesh: unit-quad corner in [0,1]^2.
    @location(0) corner: vec2<f32>,
    // Per-instance:
    @location(1) c_tl_tr: vec4<f32>,  // tl.x, tl.y, tr.x, tr.y (world space)
    @location(2) c_br_bl: vec4<f32>,  // br.x, br.y, bl.x, bl.y (world space)
    @location(3) uv_rect: vec4<f32>,  // u0, v0, u1, v1
    @location(4) tint: vec4<f32>,
    @location(5) clip: vec4<f32>,     // x, y, w, h
    @location(6) flags: vec4<f32>,    // clip_enabled, tile_wrap, tile_span.u, tile_span.v
};

struct IconVsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) clip: vec4<f32>,
    @location(3) clip_enabled: f32,
    @location(4) frag_pos: vec2<f32>,
    @location(5) uv_rect: vec4<f32>,     // copied through for the tile path
    @location(6) flags: vec4<f32>,   // x = tile_wrap, yz = one-tile UV span
};

@vertex
fn vs_icon(in: IconVsIn) -> IconVsOut {
    var out: IconVsOut;
    let tl = in.c_tl_tr.xy;
    let tr = in.c_tl_tr.zw;
    let br = in.c_br_bl.xy;
    let bl = in.c_br_bl.zw;
    let u = in.corner.x;
    let v = in.corner.y;
    // Bilinear interp of the four (possibly rotated/sheared) corners.
    let top = mix(tl, tr, u);
    let bot = mix(bl, br, u);
    let world = mix(top, bot, v);
    out.clip_position = uniforms.view_proj * vec4<f32>(world, 0.0, 1.0);
    out.uv = vec2<f32>(mix(in.uv_rect.x, in.uv_rect.z, u), mix(in.uv_rect.y, in.uv_rect.w, v));
    out.tint = in.tint;
    out.clip = in.clip;
    out.clip_enabled = in.flags.x;
    out.frag_pos = world;
    out.uv_rect = in.uv_rect;
    out.flags = vec4<f32>(in.flags.y, in.flags.z, in.flags.w, 0.0);
    return out;
}

@fragment
fn fs_icon(in: IconVsOut) -> @location(0) vec4<f32> {
    if (in.clip_enabled > 0.5) {
        let p = in.frag_pos;
        if (p.x < in.clip.x || p.x > in.clip.x + in.clip.z
            || p.y < in.clip.y || p.y > in.clip.y + in.clip.w) {
            discard;
        }
    }
    var uv = in.uv;
    if (in.flags.x > 0.5) {
        // Tile: fold the interpolated UV back into the source region modulo
        // ONE tile (flags.yz = the single-tile UV span in atlas units, from
        // the instance builder). Sampling never leaves the region's rect and
        // the sprite repeats across the whole quad (ImageFit::Tile).
        let span = in.flags.yz;
        let rel = (in.uv - in.uv_rect.xy) / span;
        uv = in.uv_rect.xy + fract(rel) * span;
    }
    let sampled = textureSample(atlas_tex, atlas_sampler, uv);
    return sampled * in.tint;
}

// ---- Instanced nine-slice path -------------------------------------------
//
// One unit-quad base mesh, one instance per nine-slice panel. The fragment
// remaps the quad's local coordinates into the source UV with the classic
// piecewise-linear nine-slice map (corners 1:1, edges stretched along one axis,
// center stretched both ways), then samples the atlas. Because the UV math is in
// the instance's LOCAL space, the full affine (incl. rotation/scale) is baked
// into the instance and applied in the vertex shader — so unlike the
// screen-space SDF chrome path, nine-slices need NO immediate-tessellation
// fallback. Replaces re-tessellating 9 quads (54 verts) per panel into the
// textured soup every frame.

struct NineVsIn {
    // Base mesh: unit-quad corner in [0,1]^2.
    @location(0) corner: vec2<f32>,
    // Per-instance:
    @location(1) lin: vec4<f32>,          // affine linear part: a, b, c, d (row-major)
    @location(2) tp: vec4<f32>,           // tx, ty, clip_enabled, _pad
    @location(3) origin_size: vec4<f32>,  // local x, y, w, h (pre-transform)
    @location(4) uv_outer: vec4<f32>,     // u0, v0, u3, v3 (outer edges)
    @location(5) uv_inner: vec4<f32>,     // u1, v1, u2, v2 (inner border seams)
    @location(6) border: vec4<f32>,       // bl, bt, br, bb (screen px)
    @location(7) tint: vec4<f32>,
    @location(8) clip: vec4<f32>,         // x, y, w, h
};

struct NineVsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) cell: vec2<f32>,         // px from the panel's local top-left
    @location(1) size: vec2<f32>,
    @location(2) uv_outer: vec4<f32>,
    @location(3) uv_inner: vec4<f32>,
    @location(4) border: vec4<f32>,
    @location(5) tint: vec4<f32>,
    @location(6) clip: vec4<f32>,
    @location(7) clip_enabled: f32,
    @location(8) frag_pos: vec2<f32>,
};

@vertex
fn vs_nine_slice(in: NineVsIn) -> NineVsOut {
    var out: NineVsOut;
    let cell = in.corner * in.origin_size.zw;
    let local = in.origin_size.xy + cell;
    let wx = in.lin.x * local.x + in.lin.y * local.y + in.tp.x;
    let wy = in.lin.z * local.x + in.lin.w * local.y + in.tp.y;
    out.clip_position = uniforms.view_proj * vec4<f32>(wx, wy, 0.0, 1.0);
    out.cell = cell;
    out.size = in.origin_size.zw;
    out.uv_outer = in.uv_outer;
    out.uv_inner = in.uv_inner;
    out.border = in.border;
    out.tint = in.tint;
    out.clip = in.clip;
    out.clip_enabled = in.tp.z;
    out.frag_pos = vec2<f32>(wx, wy);
    return out;
}

// Map one axis: screen coord `c` in `[0, size]` to source UV, with `b0`/`b1` the
// near/far border widths (screen px) and `o0,i0,i1,o1` the outer/inner/inner/outer
// UV stops. Mirrors the CPU tessellator's per-region linear interpolation,
// including the collapse to the midpoint when the panel is narrower than its
// combined borders.
fn nine_axis(c: f32, size: f32, b0: f32, b1: f32, o0: f32, i0: f32, i1: f32, o1: f32) -> f32 {
    var x1 = b0;
    var x2 = size - b1;
    if (x1 > x2) {
        let m = (x1 + x2) * 0.5;
        x1 = m;
        x2 = m;
    }
    if (c <= x1) {
        let t = select(0.0, c / x1, x1 > 0.0);
        return mix(o0, i0, t);
    } else if (c >= x2) {
        let denom = size - x2;
        let t = select(0.0, (c - x2) / denom, denom > 0.0);
        return mix(i1, o1, t);
    }
    let denom = x2 - x1;
    let t = select(0.0, (c - x1) / denom, denom > 0.0);
    return mix(i0, i1, t);
}

@fragment
fn fs_nine_slice(in: NineVsOut) -> @location(0) vec4<f32> {
    if (in.clip_enabled > 0.5) {
        let p = in.frag_pos;
        if (p.x < in.clip.x || p.x > in.clip.x + in.clip.z
            || p.y < in.clip.y || p.y > in.clip.y + in.clip.w) {
            discard;
        }
    }
    let u = nine_axis(in.cell.x, in.size.x, in.border.x, in.border.z,
                      in.uv_outer.x, in.uv_inner.x, in.uv_inner.z, in.uv_outer.z);
    let v = nine_axis(in.cell.y, in.size.y, in.border.y, in.border.w,
                      in.uv_outer.y, in.uv_inner.y, in.uv_inner.w, in.uv_outer.w);
    let sampled = textureSample(atlas_tex, atlas_sampler, vec2<f32>(u, v));
    return sampled * in.tint;
}
