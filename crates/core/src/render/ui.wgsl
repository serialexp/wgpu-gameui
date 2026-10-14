// wgpu-gameui main UI shader.
//
// Entry points share an ortho-projection uniform (group 0); the textured paths
// also bind the atlas (group 1):
//   - `vs_color`/`fs_color`: colored quads (DrawList vertices); supports per-vertex
//     scissor via the (clip, clip_enabled) attributes.
//   - `vs_analytic`/`fs_analytic`: one tagged, ordered instance stream for SDF
//     rounded-rect chrome, full-affine analytic shadows, stripe fills and
//     stroke segments (lines and polylines).
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
    // `xy`: the logical position of the target's top-left corner; `z`: logical
    // px per physical px. Maps a fragment's position on the target back to
    // logical (world) space exactly, with no interpolation.
    view: vec4<f32>,
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
    if (in.kind == 3u) {
        // A stroke segment from `p2.xy` to `p2.zw`: a quad along it, past each
        // end by that end's reach (`p5.zw`) and `p1.w` to either side, plus
        // room for the anti-aliasing ramp. `local` is (along, across) from
        // `p2.xy`.
        let a = in.p2.xy;
        let span = in.p2.zw - a;
        let seg_len = max(length(span), 1e-6);
        let along = span / seg_len;
        let across = vec2<f32>(-along.y, along.x);
        var pad = 1.0;
        if (any(in.p0 != vec4<f32>(1.0, 0.0, 0.0, 1.0))) {
            // Two screen px in local units however the transform squashes:
            // |M| / |det M| bounds one over its smallest stretch.
            let det = in.p0.x * in.p0.w - in.p0.y * in.p0.z;
            pad = 2.0 * length(in.p0) / max(abs(det), 1e-8);
        }
        let uv = vec2<f32>(
            mix(-in.p5.z - pad, seg_len + in.p5.w + pad, in.corner.x),
            mix(-in.p1.w - pad, in.p1.w + pad, in.corner.y));
        local = a + along * uv.x + across * uv.y;
        out.local = uv;
    } else if (in.kind != 1u) {
        // Chrome and stripes. The edge's anti-aliasing ramp reaches up to
        // ~0.7px past the rect, so pad the quad or the outer half of that ramp
        // is never rasterized: a fractional edge then loses its outside pixel.
        // Translate-only records pad 1px (on whole pixels those pixels come
        // out empty and are discarded); under rotation/scale, 2 screen px
        // converted to local units per axis. `local` is from the rect's
        // top-left corner.
        var pad = vec2<f32>(1.0);
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

// Edges are anti-aliased from a distance *and its gradient* (`vec3(d, grad)`,
// local units): the ramp is one pixel wide across the edge, the pixel's
// footprint taken from the local position's screen derivatives, which are
// constant across an affine quad. `fwidth` of the distance itself collapses
// where the field folds (it is flat across a box corner's 2x2 block, leaving
// corners hard), and a two-pixel smoothstep left a whole-pixel edge at 84%.

// Coverage of a pixel by the inside of an edge (`sdf` as above), with the
// screen pixel pulled back to local space as `pixel_x`/`pixel_y`. A pixel
// whose centre is half a pixel inside a whole-pixel edge is solid.
fn edge_coverage(sdf: vec3<f32>, pixel_x: vec2<f32>, pixel_y: vec2<f32>) -> f32 {
    let footprint = max(abs(dot(pixel_x, sdf.yz)) + abs(dot(pixel_y, sdf.yz)), 1e-4);
    return clamp(0.5 - sdf.x / footprint, 0.0, 1.0);
}

// A box's distance and gradient from `q`, the offset `abs(p - centre) - half`
// grown by a circular corner `radius`. The gradient's signs are dropped; only
// its size across the pixel matters.
fn box_sdf(q: vec2<f32>, radius: f32) -> vec3<f32> {
    if (q.x > 0.0 && q.y > 0.0) {
        let l = length(q);
        return vec3<f32>(l - radius, q / l);
    }
    let across = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), q.x > q.y);
    return vec3<f32>(max(q.x, q.y) - radius, across);
}

fn chrome_rounded_rect_sdf(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> vec3<f32> {
    let half = size * 0.5;
    let radius = corner_radius(p, size, radii);
    return box_sdf(abs(p - half) - half + vec2<f32>(radius), radius);
}

// Distance to a rectangle whose corner arcs may be elliptical. This is used for
// the padding edge: unequal adjacent border widths turn a circular outer corner
// into an elliptical inner corner rather than overlapping rectangular bands.
fn inner_sdf(p: vec2<f32>, size: vec2<f32>, outer_radii: vec4<f32>, widths: vec4<f32>) -> vec3<f32> {
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
    // Offset from the unrounded box's corner, and from the corner arc's centre.
    let o = abs(p - half) - half;
    let q = o + radius;
    // Beside a straight edge, anywhere inside, or at a square corner, the
    // plain box distance is exact. It must be used deep inside too: the arc's
    // estimate bottoms out at -radius there, so a quadrant with a radius would
    // disagree with a square neighbour, and read the jump as an edge (a dark
    // seam through the fill).
    if (q.x <= 0.0 || q.y <= 0.0 || radius.x <= 1e-4 || radius.y <= 1e-4) {
        return box_sdf(o, 0.0);
    }
    // In the corner box: the (possibly elliptical) arc's estimate, scaled to
    // about a pixel a pixel, and its gradient.
    let scale = min(radius.x, radius.y);
    let l = length(q / radius);
    return vec3<f32>((l - 1.0) * scale, q / (radius * radius) / l * scale);
}

fn shade_chrome(in: AnalyticVsOut) -> vec4<f32> {
    if (in.p1.z > 0.5 && (in.world.x < in.p8.x || in.world.x > in.p8.x + in.p8.z
        || in.world.y < in.p8.y || in.world.y > in.p8.y + in.p8.w)) { discard; }
    let size = in.p2.zw;
    let pixel_x = dpdx(in.local);
    let pixel_y = dpdy(in.local);
    let outer = edge_coverage(chrome_rounded_rect_sdf(in.local, size, in.p6), pixel_x, pixel_y);
    let inner_origin = vec2<f32>(in.p7.w, in.p7.x);
    let inner_size = size - vec2<f32>(in.p7.w + in.p7.y, in.p7.x + in.p7.z);
    let inner_d = inner_sdf(in.local - inner_origin, inner_size, in.p6, in.p7);
    var inner = edge_coverage(inner_d, pixel_x, pixel_y);
    if (inner_size.x <= 0.0 || inner_size.y <= 0.0) { inner = 0.0; }
    // The fill runs along the unit direction (p1.w, p9.w) across the quad's
    // extent in that direction, centred on it: CSS's `linear-gradient` line.
    // Must match `GradientAxis::position`.
    let gradient_dir = vec2<f32>(in.p1.w, in.p9.w);
    let gradient_span = max(abs(size.x * gradient_dir.x) + abs(size.y * gradient_dir.y), 1e-4);
    let gradient_coord = dot(in.local - size * 0.5, gradient_dir) / gradient_span + 0.5;
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
    // The distance is linear in screen space bar the very centre, so `fwidth`
    // is the pixel's footprint across the edge; the ramp is one pixel wide,
    // like chrome's.
    let aa = max(fwidth(dist), 1e-4);

    var alpha: f32;
    if (thickness <= 0.0) {
        // Filled disc: coverage inside the edge.
        alpha = clamp(0.5 - dist / aa, 0.0, 1.0);
    } else {
        // Ring centered on the radius path, spanning ±thickness/2.
        let half = thickness * 0.5;
        let outer = clamp(0.5 - (dist - half) / aa, 0.0, 1.0);
        let inner = clamp(0.5 - (dist + half) / aa, 0.0, 1.0);
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

// Stripes (kind 2): bands `p4.w` wide every `p4.z` along the unit normal
// `p4.xy`, from the rect's top-left corner moved `p5.x` along it. Smooth bands
// (`p5.y` 0) take the share of the pixel's footprint across the bands that
// they cover, so a 1 px band on whole pixels paints solid, a diagonal one stays
// smooth, and bands too dense to tell apart fade to their average. The rect's
// edge is smoothed like chrome's. Crisp bands (`p5.y` 1) paint a pixel fully
// when its centre is in a band and the rect, like an unsmoothed 1 px line.
fn shade_stripes(in: AnalyticVsOut) -> vec4<f32> {
    if (in.p1.z > 0.5 && (in.world.x < in.p8.x || in.world.x > in.p8.x + in.p8.z
        || in.world.y < in.p8.y || in.world.y > in.p8.y + in.p8.w)) { discard; }
    let period = in.p4.z;
    let half_width = in.p4.w * 0.5;
    let t = dot(in.local, in.p4.xy) - in.p5.x - half_width;
    // Signed distance from the nearest band's centre line, and the pixel's
    // footprint across the bands.
    let d = t - period * round(t / period);
    let pixel_x = dpdx(in.local);
    let pixel_y = dpdy(in.local);
    let aa = max(abs(dot(pixel_x, in.p4.xy)) + abs(dot(pixel_y, in.p4.xy)), 1e-4);
    let half_size = in.p2.zw * 0.5;
    let edge = box_sdf(abs(in.local - half_size) - half_size, 0.0);
    var coverage: f32;
    if (in.p5.y > 0.5) {
        coverage = select(0.0, 1.0, abs(d) <= half_width && edge.x <= 0.0);
    } else {
        // The footprint against the nearest band and one either side.
        let lo = d - aa * 0.5;
        let hi = d + aa * 0.5;
        var covered = 0.0;
        for (var k = -1; k <= 1; k++) {
            let centre = f32(k) * period;
            covered += max(min(hi, centre + half_width) - max(lo, centre - half_width), 0.0);
        }
        let average = min(2.0 * half_width / period, 1.0);
        let across = mix(min(covered / aa, 1.0), average, smoothstep(0.5 * period, period, aa));
        coverage = across * edge_coverage(edge, pixel_x, pixel_y);
    }
    let alpha = coverage * in.p3.a;
    if (alpha <= 0.0) { discard; }
    return vec4<f32>(in.p3.rgb, alpha);
}

// ---- Stroke segments (kind 3) ---------------------------------------------
//
// One record per segment of a line or polyline (`widgets/stroke.rs`):
//   p0 the linear part, p1 [tx, ty, clip_enabled, half width], p2 [a, b],
//   p3 the colour, p4 [the point before a, the point after b],
//   p5 [start style, end style, start reach, end reach],
//   p6 the dashes [on, off, offset, the stroke's length],
//   p7 [the stroke's length before a, dash cap, closed, 0], p8 the clip.
// End styles: 0 butt, 1 square, 2 round cap (open ends); 3 round join,
// 4 miter, 5 bevel (corners). Dash caps: 0 butt, 1 square, 2 round.
//
// Near a corner both segments reach into the same pixels. Each pixel goes to
// whichever of this segment and its two neighbours is nearest. Where two are
// equally near (past the corner's outside, where both are nearest at the
// corner point) the line halving the corner's angle splits them. Neighbours
// compute these from the same points and the same exact pixel position, so
// they agree on every pixel: a translucent stroke paints each pixel once.

fn segment_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let t = dot(p - a, ab) / dot(ab, ab);
    // Past an end, the distance to that point, computed alike by both
    // segments that share it.
    if (t <= 0.0) { return length(p - a); }
    if (t >= 1.0) { return length(p - b); }
    return length(p - a - ab * t);
}

// Whether `p` is on the side the segment leaving the corner `at` (towards
// `next`) owns, of the line halving the corner's angle.
fn owns_corner_side(p: vec2<f32>, prev: vec2<f32>, at: vec2<f32>, next: vec2<f32>) -> bool {
    let incoming = normalize(at - prev);
    let outgoing = normalize(next - at);
    var split = incoming + outgoing;
    if (dot(split, split) < 1e-12) {
        // Turned right back: split along the line itself.
        split = vec2<f32>(-outgoing.y, outgoing.x);
    }
    return dot(p - at, split) >= 0.0;
}

// The larger of two distances (the intersection of their shapes). Outside
// both, with `corner`, the distance to the corner where two perpendicular
// edges meet.
fn sdf_max(a: vec3<f32>, b: vec3<f32>, corner: bool) -> vec3<f32> {
    if (corner && a.x > 0.0 && b.x > 0.0) {
        let l = length(vec2<f32>(a.x, b.x));
        return vec3<f32>(l, (a.yz * a.x + b.yz * b.x) / l);
    }
    return select(b, a, a.x > b.x);
}

// The stroke's distance past one end of the segment, `q` = (how far past the
// end along the segment, across it), in the (along, across) frame. `bevel` is
// the corner's outward direction in that frame (towards past the end along
// `q.x`) and the cosine of half its turn, for a bevel.
fn segment_end_sdf(style: f32, q: vec2<f32>, half: f32, bevel: vec3<f32>) -> vec3<f32> {
    let band = vec3<f32>(abs(q.y) - half, 0.0, 1.0);
    if (style < 0.5) { return box_sdf(vec2<f32>(q.x, abs(q.y) - half), 0.0); }
    if (style < 1.5) { return box_sdf(vec2<f32>(q.x - half, abs(q.y) - half), 0.0); }
    if (style < 3.5) {
        let l = max(length(q), 1e-6);
        return vec3<f32>(l - half, q / l);
    }
    if (style < 4.5) { return band; }
    if (dot(bevel.xy, bevel.xy) < 1e-12) { return band; }
    let cut = vec3<f32>(dot(q, bevel.xy) - half * bevel.z, bevel.xy);
    return sdf_max(band, cut, false);
}

// For a bevel at the corner between `incoming` and `outgoing` (unit, local
// space): the outward direction in the frame (`frame_along`, `frame_across`),
// with its along part measured from the corner past the end in question
// (`sign` -1 at the start, 1 at the end), and the cosine of half the turn.
fn bevel_of(incoming: vec2<f32>, outgoing: vec2<f32>, frame_along: vec2<f32>,
            frame_across: vec2<f32>, sign: f32) -> vec3<f32> {
    let outward = incoming - outgoing;
    let l = length(outward);
    if (l < 1e-6) { return vec3<f32>(0.0); }
    let w = outward / l;
    return vec3<f32>(sign * dot(w, frame_along), dot(w, frame_across),
                     0.5 * length(incoming + outgoing));
}

// Signed distance along the stroke from `s` to the nearest dash (negative
// inside one): dashes run `on` from every `period`, the pattern `offset` in.
// An open stroke's dashes are cut to its `length`, and one starting at its
// very end is not drawn (as Chromium and Skia do).
fn dash_distance(s: f32, on: f32, period: f32, offset: f32, length: f32, open: bool) -> f32 {
    let first = floor((s + offset) / period) * period - offset;
    var nearest = 1e30;
    for (var k = -1; k <= 1; k++) {
        var lo = first + f32(k) * period;
        var hi = lo + on;
        if (open) {
            if (lo >= length || hi < 0.0) { continue; }
            lo = max(lo, 0.0);
            hi = min(hi, length);
        }
        nearest = min(nearest, max(lo - s, s - hi));
    }
    return nearest;
}

// Coverage of a pixel by the inside of a stroke's edge (`sdf` as for
// `edge_coverage`): a ramp one screen pixel wide along the edge's normal.
// Narrower than `edge_coverage`'s across a diagonal edge (which spans the
// whole pixel's extent), and closer to how Chromium draws SVG strokes.
fn stroke_coverage(sdf: vec3<f32>, pixel_x: vec2<f32>, pixel_y: vec2<f32>) -> f32 {
    let footprint = max(length(vec2<f32>(dot(pixel_x, sdf.yz), dot(pixel_y, sdf.yz))), 1e-4);
    return clamp(0.5 - sdf.x / footprint, 0.0, 1.0);
}

fn shade_segment(in: AnalyticVsOut) -> vec4<f32> {
    if (in.p1.z > 0.5 && (in.world.x < in.p8.x || in.world.x > in.p8.x + in.p8.z
        || in.world.y < in.p8.y || in.world.y > in.p8.y + in.p8.w)) { discard; }
    // The pixel centre in the stroke's space, from its position on the target
    // rather than an interpolated value, so neighbouring segments agree.
    let scale = uniforms.view.z;
    let world = uniforms.view.xy + in.clip_position.xy * scale;
    let m = in.p0;
    let inv = vec4<f32>(m.w, -m.y, -m.z, m.x) / (m.x * m.w - m.y * m.z);
    let rel = world - in.p1.xy;
    let p = vec2<f32>(inv.x * rel.x + inv.y * rel.y, inv.z * rel.x + inv.w * rel.y);

    let a = in.p2.xy;
    let b = in.p2.zw;
    let start = in.p5.x;
    let end = in.p5.y;
    let own = segment_distance(p, a, b);
    if (start > 2.5) {
        let other = segment_distance(p, in.p4.xy, a);
        if (other < own || (other == own && !owns_corner_side(p, in.p4.xy, a, b))) { discard; }
    }
    if (end > 2.5) {
        let other = segment_distance(p, b, in.p4.zw);
        if (other < own || (other == own && owns_corner_side(p, a, b, in.p4.zw))) { discard; }
    }

    // The (along, across) frame from `a`, and the screen pixel in it.
    let half = in.p1.w;
    let seg_len = length(b - a);
    let e = (b - a) / seg_len;
    let n = vec2<f32>(-e.y, e.x);
    let uv = vec2<f32>(dot(p - a, e), dot(p - a, n));
    let screen_x = vec2<f32>(inv.x, inv.z) * scale;
    let screen_y = vec2<f32>(inv.y, inv.w) * scale;
    let pixel_x = vec2<f32>(dot(screen_x, e), dot(screen_x, n));
    let pixel_y = vec2<f32>(dot(screen_y, e), dot(screen_y, n));

    let on = in.p6.x;
    let period = on + in.p6.y;
    let dashed = period > 0.0;
    // Past an open end of a dashed stroke the dashes draw the caps.
    var sdf = vec3<f32>(abs(uv.y) - half, 0.0, 1.0);
    if (uv.x < 0.0 && (start > 2.5 || !dashed)) {
        var bevel = vec3<f32>(0.0);
        if (start > 4.5) { bevel = bevel_of(normalize(a - in.p4.xy), e, e, n, -1.0); }
        sdf = segment_end_sdf(start, vec2<f32>(-uv.x, uv.y), half, bevel);
    } else if (uv.x > seg_len && (end > 2.5 || !dashed)) {
        var bevel = vec3<f32>(0.0);
        if (end > 4.5) { bevel = bevel_of(e, normalize(in.p4.zw - b), e, n, 1.0); }
        sdf = segment_end_sdf(end, vec2<f32>(uv.x - seg_len, uv.y), half, bevel);
    }
    let solid = stroke_coverage(sdf, pixel_x, pixel_y);
    var coverage = solid;
    if (dashed) {
        // How far along the stroke the pixel is. Past a corner both segments
        // hold the corner's position, so the pattern meets itself there.
        var along = uv.x;
        if (start > 2.5) { along = max(along, 0.0); }
        if (end > 2.5) { along = min(along, seg_len); }
        let s = in.p7.x + along;
        let d = dash_distance(s, on, period, in.p6.z, in.p6.w, in.p7.z < 0.5);
        let cap = in.p7.y;
        var dash_sdf: vec3<f32>;
        var cap_reach = 0.0;
        if (cap < 1.5) {
            cap_reach = select(0.0, half, cap > 0.5);
            dash_sdf = sdf_max(sdf, vec3<f32>(d - cap_reach, 1.0, 0.0), true);
        } else {
            cap_reach = half;
            dash_sdf = sdf;
            if (d > 0.0) {
                let q = vec2<f32>(d, max(sdf.x + half, 0.0));
                let l = max(length(q), 1e-6);
                dash_sdf = vec3<f32>(l - half, q / l);
            }
        }
        // Dashes too close together to tell apart fade to their average.
        let footprint = abs(pixel_x.x) + abs(pixel_y.x);
        let duty = clamp((on + 2.0 * cap_reach) / period, 0.0, 1.0);
        coverage = mix(stroke_coverage(dash_sdf, pixel_x, pixel_y), solid * duty,
                       smoothstep(0.5 * period, period, footprint));
    }
    let alpha = coverage * in.p3.a;
    if (alpha <= 0.0) { discard; }
    return vec4<f32>(in.p3.rgb, alpha);
}

@fragment
fn fs_analytic(in: AnalyticVsOut) -> @location(0) vec4<f32> {
    // The kind is flat, so every 2x2 derivative quad follows one uniform branch;
    // fwidth/dpdx/dpdy remain well-defined in every analytic implementation.
    if (in.kind == 0u) { return shade_chrome(in); }
    if (in.kind == 2u) { return shade_stripes(in); }
    if (in.kind == 3u) { return shade_segment(in); }
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
