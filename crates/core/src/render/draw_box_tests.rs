use super::*;
use crate::affine::Affine2;
use crate::widgets::DrawList;

/// 100×100 at scale 1 from the origin: a margin of 8·ε·100 ≈ 1e-4 px, too
/// small to move any bound below off its pixel.
fn plain() -> Target {
    Target::new((100, 100), 1.0, [0.0, 0.0])
}

fn pixels(x0: u32, y0: u32, x1: u32, y1: u32) -> PixelBox {
    PixelBox { x0, y0, x1, y1 }
}

const IDENTITY: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

/// An analytic record of `kind` with the shared fields set and the rest zero.
fn record(kind: u32, linear: [f32; 4], translation: [f32; 2], rect: [f32; 4]) -> AnalyticInstance {
    let mut payload = [[0.0; 4]; 10];
    payload[0] = linear;
    payload[1] = [translation[0], translation[1], 0.0, 0.0];
    payload[2] = rect;
    AnalyticInstance {
        payload,
        kind,
        padding: [0; 3],
    }
}

fn clipped(mut r: AnalyticInstance, clip: [f32; 4]) -> AnalyticInstance {
    r.payload[1][2] = 1.0;
    r.payload[8] = clip;
    r
}

/// A segment record from `a` to `b`, `half` wide to either side, each end
/// reaching `reach` past its point.
fn segment(linear: [f32; 4], a: [f32; 2], b: [f32; 2], half: f32, reach: f32) -> AnalyticInstance {
    let mut r = record(3, linear, [0.0, 0.0], [a[0], a[1], b[0], b[1]]);
    r.payload[1][3] = half;
    r.payload[5] = [0.0, 0.0, reach, reach];
    r
}

fn circle_at(centre: [f32; 2], radius: f32, thickness: f32) -> CircleInstance {
    CircleInstance {
        center: [centre[0], centre[1], radius, thickness],
        color: [1.0; 4],
        clip: [0.0; 4],
        params: [0.0; 4],
    }
}

fn icon_at(corners: [[f32; 2]; 4], clip: Option<Rect>) -> IconDraw {
    IconDraw {
        corners,
        sprite: None,
        icon_key: String::new(),
        tint: [1.0; 4],
        clip,
        src: None,
        wrap: false,
    }
}

// ---- Turning a bound into pixels ------------------------------------------

#[test]
fn a_clip_keeps_the_pixels_whose_centres_are_on_its_edges() {
    let t = plain();
    // Centres 2.5 and 3.5 sit exactly on this clip's edges.
    assert_eq!(
        clip_bound(&t, [2.5, 2.5, 1.0, 1.0]).pixels(&t),
        pixels(2, 2, 4, 4)
    );
    // Only centre 2.5 is inside this one.
    assert_eq!(
        clip_bound(&t, [2.0, 2.0, 1.0, 1.0]).pixels(&t),
        pixels(2, 2, 3, 3)
    );
}

#[test]
fn a_clip_of_zero_or_negative_size_holds_nothing() {
    let t = plain();
    for clip in [
        [5.5, 5.5, 0.0, 3.0],
        [5.5, 5.5, 3.0, 0.0],
        [5.5, 5.5, -1.0, 3.0],
    ] {
        assert_eq!(clip_bound(&t, clip).pixels(&t), PixelBox::EMPTY, "{clip:?}");
    }
}

#[test]
fn a_clip_that_is_not_a_number_discards_nothing_as_the_shader_does() {
    let t = plain();
    let mut c = circle_at([20.0, 20.0], 5.0, 0.0);
    let unclipped = circle(&t, &c);
    c.params[0] = 1.0;
    c.clip = [f32::NAN, 0.0, 10.0, 10.0];
    assert_eq!(circle(&t, &c), unclipped);
}

#[test]
fn a_position_that_is_not_finite_covers_the_whole_target() {
    let t = plain();
    let whole = pixels(0, 0, 100, 100);
    assert_eq!(circle(&t, &circle_at([f32::NAN, 20.0], 5.0, 0.0)), whole);
    assert_eq!(
        circle(&t, &circle_at([20.0, 20.0], f32::INFINITY, 0.0)),
        whole
    );
    let r = record(0, IDENTITY, [0.0, 0.0], [10.0, f32::NAN, 5.0, 5.0]);
    assert_eq!(analytic(&t, &r), whole);
}

#[test]
fn content_off_the_target_has_an_empty_box_and_content_across_its_edge_is_cut() {
    let t = plain();
    assert!(circle(&t, &circle_at([-50.0, 20.0], 5.0, 0.0)).is_empty());
    assert!(circle(&t, &circle_at([20.0, 150.0], 5.0, 0.0)).is_empty());
    // Centre on the right edge: from 93 to the edge.
    assert_eq!(
        circle(&t, &circle_at([100.0, 20.0], 5.0, 0.0)),
        pixels(93, 13, 100, 27)
    );
}

#[test]
fn a_scale_factor_that_is_not_positive_is_taken_as_one() {
    assert_eq!(Target::new((10, 10), 0.0, [0.0, 0.0]).scale, 1.0);
    assert_eq!(Target::new((10, 10), -2.0, [0.0, 0.0]).scale, 1.0);
}

#[test]
fn the_float_margin_grows_with_the_largest_coordinate_involved() {
    let t = plain();
    assert_eq!(float_margin(&t, 0.0), 8.0 * f32::EPSILON * 100.0);
    assert_eq!(float_margin(&t, 1e5), 8.0 * f32::EPSILON * 1e5);
    let far = Target::new((100, 100), 2.0, [0.0, -3e5]);
    assert_eq!(float_margin(&far, 10.0), 8.0 * f32::EPSILON * 3e5 * 2.0);
}

#[test]
fn boxes_overlap_only_when_they_share_a_pixel() {
    let a = pixels(0, 0, 10, 10);
    assert!(a.overlaps(&pixels(9, 9, 20, 20)));
    assert!(
        !a.overlaps(&pixels(10, 0, 20, 10)),
        "touching edges share none"
    );
    assert!(!a.overlaps(&PixelBox::EMPTY));
    assert!(!PixelBox::EMPTY.overlaps(&PixelBox::EMPTY));
    assert_eq!(a.union(&PixelBox::EMPTY), a);
    assert_eq!(a.union(&pixels(20, 5, 30, 8)), pixels(0, 0, 30, 10));
}

#[test]
fn a_box_holds_its_pixels_up_to_but_not_including_its_end() {
    let a = pixels(2, 3, 5, 7);
    assert!(a.contains(2, 3) && a.contains(4, 6));
    assert!(!a.contains(5, 3) && !a.contains(2, 7) && !a.contains(1, 3));
    assert!(!PixelBox::EMPTY.contains(0, 0));
}

// ---- Chrome and stripes ------------------------------------------------------

#[test]
fn untransformed_chrome_reaches_0_61_px_past_its_rect() {
    let t = plain();
    // 10..30 by 20..30, widened 0.61: centres from 9.5 to 30.5.
    let r = record(0, IDENTITY, [0.0, 0.0], [10.0, 20.0, 20.0, 10.0]);
    assert_eq!(analytic(&t, &r), pixels(9, 19, 31, 31));
    // Moved 0.3 right, the left bound is 9.69: centre 9.5 is out.
    let r = record(0, IDENTITY, [0.3, 0.0], [10.0, 20.0, 20.0, 10.0]);
    assert_eq!(analytic(&t, &r), pixels(10, 19, 31, 31));
    // Stripes use the same quad and edge.
    let r = record(2, IDENTITY, [0.0, 0.0], [10.0, 20.0, 20.0, 10.0]);
    assert_eq!(analytic(&t, &r), pixels(9, 19, 31, 31));
}

#[test]
fn chrome_reaches_0_61_physical_px_at_any_scale() {
    // At scale 2 the rect is 20..60 by 40..60 physical.
    let t = Target::new((200, 200), 2.0, [0.0, 0.0]);
    let r = record(0, IDENTITY, [0.0, 0.0], [10.0, 20.0, 20.0, 10.0]);
    assert_eq!(analytic(&t, &r), pixels(19, 39, 61, 61));
}

#[test]
fn chrome_with_a_large_radius_at_a_fractional_position_keeps_the_rect_bound() {
    let t = plain();
    let mut r = record(0, IDENTITY, [0.0, 0.0], [10.4, 20.6, 40.0, 30.0]);
    r.payload[6] = [15.0; 4];
    // 9.79..51.01 by 19.99..51.21.
    assert_eq!(analytic(&t, &r), pixels(10, 20, 51, 51));
}

/// How far past its rect an untransformed chrome record draws, in px at
/// scale 1, by `edge_coverage(chrome_rounded_rect_sdf(..))` from `ui.wgsl`
/// copied over: the furthest a point with any coverage lies outside the rect
/// along either axis, over a grid of 1/1024 px around a corner. The GPU test
/// reads 8-bit pixels and can't see coverage below 1/510; this sees all of
/// it. The field scales with the pixel, so the reach in physical px is the
/// same at any scale factor.
fn chrome_reach(size: [f32; 2], radius: f32) -> f32 {
    fn box_sdf(q: [f32; 2], radius: f32) -> (f32, [f32; 2]) {
        if q[0] > 0.0 && q[1] > 0.0 {
            let l = q[0].hypot(q[1]);
            return (l - radius, [q[0] / l, q[1] / l]);
        }
        let across = if q[0] > q[1] { [1.0, 0.0] } else { [0.0, 1.0] };
        (q[0].max(q[1]) - radius, across)
    }
    let half = [size[0] * 0.5, size[1] * 0.5];
    let mut reach = 0.0f32;
    // From 1 px outside the top-left corner to half a px inside it; the far
    // corners are its mirror images.
    let step = |i: i32| i as f32 / 1024.0;
    for i in -1024..=512 {
        for j in -1024..=512 {
            let p = [step(i), step(j)];
            let q = [
                (p[0] - half[0]).abs() - half[0] + radius,
                (p[1] - half[1]).abs() - half[1] + radius,
            ];
            let (d, g) = box_sdf(q, radius);
            // The pixel steps are (1, 0) and (0, 1) local px.
            let footprint = (g[0].abs() + g[1].abs()).max(1e-4);
            let coverage = (0.5 - d / footprint).clamp(0.0, 1.0);
            if coverage > 0.0 {
                reach = reach.max(-p[0]).max(-p[1]);
            }
        }
    }
    reach
}

#[test]
fn chrome_draws_no_further_than_its_reach_for_any_radius_of_zero_or_more() {
    for size in [[20.0f32, 12.0], [0.4, 0.3], [3.0, 1.0]] {
        let most = 0.5 * size[0].min(size[1]);
        for radius in [0.0f32, 0.05, 0.25, 1.0, 4.0, 100.0] {
            let reach = chrome_reach(size, radius.min(most));
            assert!(
                reach <= CHROME_REACH,
                "{size:?} radius {radius}: reaches {reach}"
            );
        }
    }
    // At radius 0 the corner takes it to (1 + √2) / 4 ≈ 0.6036, a quarter px
    // to the side of the corner: the bound is tight.
    let r0 = chrome_reach([20.0, 12.0], 0.0);
    assert!(r0 > 0.603, "{r0}");
}

#[test]
fn a_negative_radius_draws_further_so_its_box_is_the_padded_quad() {
    // The reason for the fallback: the field reaches past 0.61.
    assert!(chrome_reach([20.0, 12.0], -0.6) > CHROME_REACH);
    let t = plain();
    let mut r = record(0, IDENTITY, [0.0, 0.0], [10.0, 20.0, 20.0, 10.0]);
    r.payload[6] = [0.0, 0.0, -0.6, 0.0];
    // Padded 2 px: 8..32 by 18..32.
    assert_eq!(analytic(&t, &r), pixels(8, 18, 32, 32));
    // Stripes don't read the radii.
    let mut r = record(2, IDENTITY, [0.0, 0.0], [10.0, 20.0, 20.0, 10.0]);
    r.payload[6] = [-0.6; 4];
    assert_eq!(analytic(&t, &r), pixels(9, 19, 31, 31));
}

#[test]
fn a_field_that_is_not_finite_makes_the_box_the_padded_quad() {
    let t = plain();
    // A NaN colour turns the ring's alpha into NaN, which isn't discarded.
    for (slot, lane) in [(3, 3), (5, 0), (7, 1), (9, 2), (1, 3)] {
        for kind in [0, 2] {
            let mut r = record(kind, IDENTITY, [0.0, 0.0], [10.0, 20.0, 20.0, 10.0]);
            r.payload[slot][lane] = f32::NAN;
            assert_eq!(
                analytic(&t, &r),
                pixels(8, 18, 32, 32),
                "kind {kind}, p{slot}[{lane}]"
            );
            r.payload[slot][lane] = f32::INFINITY;
            assert_eq!(analytic(&t, &r), pixels(8, 18, 32, 32));
        }
    }
}

#[test]
fn transformed_chrome_is_its_padded_quad() {
    let t = plain();
    // Scaled 2 by 0.5: padded 1 and 4 local px, so 9..21 by 6..24 local,
    // 18..42 by 3..12 on the target.
    let r = record(
        0,
        [2.0, 0.0, 0.0, 0.5],
        [0.0, 0.0],
        [10.0, 10.0, 10.0, 10.0],
    );
    assert_eq!(analytic(&t, &r), pixels(18, 3, 42, 12));
    // A quarter turn (x' = 50 - y, y' = x), padded 2: 8..22 local both ways.
    let r = record(
        0,
        [0.0, -1.0, 1.0, 0.0],
        [50.0, 0.0],
        [10.0, 10.0, 10.0, 10.0],
    );
    assert_eq!(analytic(&t, &r), pixels(28, 8, 42, 22));
}

#[test]
fn a_record_is_cut_by_its_clip_only_when_its_clip_is_on() {
    let t = plain();
    let r = record(0, IDENTITY, [0.0, 0.0], [10.0, 20.0, 20.0, 10.0]);
    let mut off = r;
    off.payload[8] = [15.0, 0.0, 5.0, 100.0];
    assert_eq!(analytic(&t, &off), pixels(9, 19, 31, 31));
    assert_eq!(
        analytic(&t, &clipped(r, [15.0, 0.0, 5.0, 100.0])),
        pixels(15, 19, 20, 31)
    );
    assert!(analytic(&t, &clipped(r, [15.0, 0.0, 0.0, 100.0])).is_empty());
}

// ---- Shadows and stroke segments ----------------------------------------------

#[test]
fn a_shadow_is_its_raster_rect() {
    let t = plain();
    let r = record(1, IDENTITY, [0.0, 0.0], [5.0, 6.0, 20.0, 10.0]);
    assert_eq!(analytic(&t, &r), pixels(5, 6, 25, 16));
    let r = record(1, [2.0, 0.0, 0.0, 2.0], [1.0, 1.0], [5.0, 6.0, 20.0, 10.0]);
    assert_eq!(analytic(&t, &r), pixels(11, 13, 51, 33));
}

#[test]
fn a_segment_reaches_its_half_width_its_end_reach_and_one_px_more() {
    let t = plain();
    // 10..30 along y = 10, 1 to either side: padded 1, 9..31 by 8..12.
    let butt = segment(IDENTITY, [10.0, 10.0], [30.0, 10.0], 1.0, 0.0);
    assert_eq!(analytic(&t, &butt), pixels(9, 8, 31, 12));
    let square = segment(IDENTITY, [10.0, 10.0], [30.0, 10.0], 1.0, 1.0);
    assert_eq!(analytic(&t, &square), pixels(8, 8, 32, 12));
    // Vertical: the frame turns with it.
    let down = segment(IDENTITY, [10.0, 10.0], [10.0, 30.0], 1.0, 0.0);
    assert_eq!(analytic(&t, &down), pixels(8, 9, 12, 31));
}

#[test]
fn a_transformed_segment_pads_two_screen_px() {
    let t = plain();
    // Scale 2: pad 2·√8 / 4 = √2 local px, so 8.586..31.414 by
    // 7.586..12.414 local, doubled on the target.
    let r = segment([2.0, 0.0, 0.0, 2.0], [10.0, 10.0], [30.0, 10.0], 1.0, 0.0);
    assert_eq!(analytic(&t, &r), pixels(17, 15, 63, 25));
}

// ---- Circles, sprites, nine-slices ---------------------------------------------

#[test]
fn a_circle_is_its_quad() {
    let t = plain();
    // Radius 5 + 2: 13..27.
    assert_eq!(
        circle(&t, &circle_at([20.0, 20.0], 5.0, 0.0)),
        pixels(13, 13, 27, 27)
    );
    // A ring 2 thick reaches 1 further.
    assert_eq!(
        circle(&t, &circle_at([20.0, 20.0], 5.0, 2.0)),
        pixels(12, 12, 28, 28)
    );
    let mut c = circle_at([20.0, 20.0], 5.0, 0.0);
    c.params[0] = 1.0;
    c.clip = [15.0, 15.0, 10.0, 10.0];
    assert_eq!(circle(&t, &c), pixels(15, 15, 25, 25));
}

#[test]
fn a_sprite_is_the_box_around_its_corners() {
    let t = plain();
    let corners = [[10.0, 10.0], [20.0, 15.0], [15.0, 25.0], [5.0, 20.0]];
    assert_eq!(icon(&t, &icon_at(corners, None)), pixels(5, 10, 20, 25));
    let clip = Some(Rect::new(0.0, 0.0, 12.0, 100.0));
    assert_eq!(icon(&t, &icon_at(corners, clip)), pixels(5, 10, 12, 25));
    let empty = Some(Rect::new(0.0, 0.0, 0.0, 100.0));
    assert!(icon(&t, &icon_at(corners, empty)).is_empty());
}

#[test]
fn a_nine_slice_is_its_rect_through_its_transform() {
    let t = plain();
    let draw = NineSliceDraw {
        local: Rect::new(10.0, 10.0, 20.0, 10.0),
        transform: Affine2::translation(5.0, 5.0),
        nine_slice: None,
        texture_key: String::new(),
        tint: [1.0; 4],
        clip: None,
    };
    assert_eq!(nine_slice(&t, &draw), pixels(15, 15, 35, 25));
}

// ---- Soup ------------------------------------------------------------------------

#[test]
fn soup_is_the_union_of_its_triangles() {
    let t = plain();
    let mut list = DrawList::new();
    list.triangle((10.0, 10.0), (15.0, 10.0), (10.0, 15.0), [1.0; 4]);
    list.triangle((40.0, 50.0), (45.0, 55.0), (40.0, 55.0), [1.0; 4]);
    assert_eq!(
        soup(&t, &list.vertices, &list.indices),
        pixels(10, 10, 45, 55)
    );
}

#[test]
fn soup_is_cut_by_its_clip() {
    let t = plain();
    let mut list = DrawList::new();
    list.push_clip(Rect::new(12.0, 0.0, 100.0, 100.0));
    list.triangle((10.0, 10.0), (15.0, 10.0), (15.0, 15.0), [1.0; 4]);
    list.pop_clip();
    assert_eq!(
        soup(&t, &list.vertices, &list.indices),
        pixels(12, 10, 15, 15)
    );
}

#[test]
fn soup_inside_clips_that_miss_each_other_has_an_empty_box() {
    let t = plain();
    let mut list = DrawList::new();
    list.push_clip(Rect::new(50.0, 50.0, 4.0, 4.0));
    list.push_clip(Rect::new(2.5, 2.5, 10.0, 10.0));
    list.triangle((1.5, 1.5), (4.5, 1.5), (1.5, 4.5), [1.0; 4]);
    list.pop_clip();
    list.pop_clip();
    assert!(!list.indices.is_empty());
    assert!(soup(&t, &list.vertices, &list.indices).is_empty());
}

#[test]
fn a_triangle_whose_vertices_clip_differently_is_cut_by_the_box_around_their_clips() {
    let t = plain();
    let at = |x: f32, y: f32, clip: Option<Rect>| Vertex::new(x, y, [1.0; 4]).with_clip(clip);
    let left = Some(Rect::new(0.0, 0.0, 20.0, 100.0));
    let right = Some(Rect::new(30.0, 0.0, 20.0, 100.0));
    let vertices = [
        at(10.0, 10.0, left),
        at(80.0, 10.0, right),
        at(10.0, 40.0, left),
    ];
    // The clips span 0..50: the triangle's 10..80 is cut at 50.
    assert_eq!(soup(&t, &vertices, &[0, 1, 2]), pixels(10, 10, 50, 40));
    // One vertex unclipped: the shader interpolates its flag, so the
    // triangle is left uncut.
    let vertices = [
        at(10.0, 10.0, left),
        at(80.0, 10.0, None),
        at(10.0, 40.0, left),
    ];
    assert_eq!(soup(&t, &vertices, &[0, 1, 2]), pixels(10, 10, 80, 40));
}

#[test]
fn a_triangle_between_different_clips_all_without_width_holds_nothing() {
    let t = plain();
    let at = |x: f32, y: f32, clip: [f32; 4]| {
        Vertex::new(x, y, [1.0; 4]).with_clip(Some(Rect::new(clip[0], clip[1], clip[2], clip[3])))
    };
    // Every pixel's clip is a mean of widths 0, 0 and -3: none.
    let vertices = [
        at(10.0, 10.0, [5.0, 5.0, 0.0, 50.0]),
        at(80.0, 10.0, [40.0, 5.0, 0.0, 50.0]),
        at(10.0, 40.0, [5.0, 30.0, -3.0, 20.0]),
    ];
    assert!(soup(&t, &vertices, &[0, 1, 2]).is_empty());
    // One width that isn't a number: the shader's test fails and keeps the
    // pixel, so the box can't be empty.
    let vertices = [
        at(10.0, 10.0, [5.0, 5.0, 0.0, 50.0]),
        at(80.0, 10.0, [40.0, 5.0, f32::NAN, 50.0]),
        at(10.0, 40.0, [5.0, 30.0, 0.0, 20.0]),
    ];
    assert!(!soup(&t, &vertices, &[0, 1, 2]).is_empty());
}

#[test]
fn a_flipped_record_is_its_padded_quad() {
    let t = plain();
    // Mirrored (x' = 50 - x), padded 2: 8..22 local, 28..42 on the target.
    let r = record(
        0,
        [-1.0, 0.0, 0.0, 1.0],
        [50.0, 0.0],
        [10.0, 10.0, 10.0, 10.0],
    );
    assert_eq!(analytic(&t, &r), pixels(28, 8, 42, 22));
}

#[test]
fn a_rect_of_negative_size_is_bound_by_its_corners_either_way() {
    let t = plain();
    let r = record(0, IDENTITY, [0.0, 0.0], [30.0, 20.0, -20.0, 10.0]);
    assert_eq!(analytic(&t, &r), pixels(9, 19, 31, 31));
}

#[test]
fn a_diagonal_segment_is_its_turned_quad() {
    let t = plain();
    // From (10, 10) to (20, 20), half width 1, no reach, padded 1: from
    // 10 - (1 + 2)/√2 ≈ 7.88 to 20 + 3/√2 ≈ 22.12 both ways.
    let r = segment(IDENTITY, [10.0, 10.0], [20.0, 20.0], 1.0, 0.0);
    assert_eq!(analytic(&t, &r), pixels(8, 8, 22, 22));
}

#[test]
fn crisp_stripes_have_the_same_box_as_smooth_ones() {
    let t = plain();
    let mut r = record(2, IDENTITY, [0.0, 0.0], [10.0, 20.0, 20.0, 10.0]);
    r.payload[5][1] = 1.0;
    assert_eq!(analytic(&t, &r), pixels(9, 19, 31, 31));
}

#[test]
fn a_soup_index_past_its_vertices_covers_the_whole_target() {
    let t = plain();
    let vertices = [Vertex::new(1.0, 1.0, [1.0; 4]); 3];
    assert_eq!(soup(&t, &vertices, &[0, 1, 7]), pixels(0, 0, 100, 100));
}

// ---- The projection ---------------------------------------------------------------

#[test]
fn a_view_origin_moves_every_box_with_it() {
    let at_origin = analytic(
        &plain(),
        &record(0, IDENTITY, [0.0, 0.0], [10.0, 20.0, 20.0, 10.0]),
    );
    let moved = Target::new((100, 100), 1.0, [100.0, 50.0]);
    let r = record(0, IDENTITY, [100.0, 50.0], [10.0, 20.0, 20.0, 10.0]);
    assert_eq!(analytic(&moved, &r), at_origin);
}

#[test]
fn coordinates_around_1e5_widen_a_box_by_their_float_margin() {
    // The margin there is about 0.095 px: 9.29..30.71, still 9..31.
    let far = Target::new((100, 100), 1.0, [1e5, 1e5]);
    let r = record(0, IDENTITY, [1e5, 1e5], [10.0, 20.0, 20.0, 10.0]);
    assert_eq!(analytic(&far, &r), pixels(9, 19, 31, 31));
    // A circle whose quad ends at 20.40625 (exact in f32 at 1e5, where the
    // step is 1/128): its hard bound, 20.46875, misses centre 20.5 near the
    // origin, and takes it in with the margin at 1e5.
    let right_edge = |c: &CircleInstance, t: &Target| circle(t, c).x1;
    let near = circle_at([11.40625, 50.0], 7.0, 0.0);
    assert_eq!(right_edge(&near, &plain()), 20);
    let far_circle = circle_at([1e5 + 11.40625, 1e5 + 50.0], 7.0, 0.0);
    assert_eq!(right_edge(&far_circle, &far), 21);
}

/// Text and MSDF icons prepared on the CPU, as `plain()` would draw them.
#[cfg(feature = "bundled-font")]
mod msdf_boxes {
    use super::*;
    use crate::text::{
        GlyphSnap, MsdfCpu, PixelGrid, TextBlock, register_bundled_fonts, shared_font_system,
    };

    /// `texts` prepared as one command.
    fn prepare(texts: &[TextBlock]) -> MsdfPrepared {
        let fs = shared_font_system();
        register_bundled_fonts(&fs).expect("register bundled fonts");
        let mut cpu = MsdfCpu::new(fs);
        let mut out = MsdfPrepared::default();
        let grid = PixelGrid::new(GlyphSnap::default(), 1.0, [0.0, 0.0], true);
        cpu.prepare_texts(
            &mut out,
            grid,
            texts,
            std::iter::once(0..texts.len() as u32),
        );
        out
    }

    fn label(text: &str, x: f32, y: f32) -> TextBlock {
        TextBlock::new(text, x, y).with_size(14.0)
    }

    #[test]
    fn a_text_block_is_its_glyph_quads_cut_by_its_clip() {
        let t = plain();
        let free = text_block(&t, &prepare(&[label("Agy", 20.0, 30.0)]), 0);
        assert!(!free.is_empty());
        // The tiles reach left of the pen; the line has room above its glyphs,
        // so they start below its top.
        assert!(free.x0 < 20 && free.y0 >= 30);
        // Clipped to x 25..30, inclusive: pixel centres 25.5 to 29.5.
        let clipped = label("Agy", 20.0, 30.0).with_clip(Rect::new(25.0, 0.0, 5.0, 100.0));
        let cut = text_block(&t, &prepare(&[clipped]), 0);
        assert_eq!(cut, pixels(25, free.y0, 30, free.y1));
        // A clip of zero size holds nothing.
        let none = label("Agy", 20.0, 30.0).with_clip(Rect::new(25.0, 0.0, 0.0, 100.0));
        assert!(text_block(&t, &prepare(&[none]), 0).is_empty());
    }

    #[test]
    fn a_shadow_widens_its_blocks_box_by_its_offset() {
        let t = plain();
        let plain_box = text_block(&t, &prepare(&[label("Agy", 20.0, 30.0)]), 0);
        let shadowed = label("Agy", 20.0, 30.0).with_shadow(0, 0, 0, 255, 6.0, 0.0, 0.0);
        let shadow_box = text_block(&t, &prepare(&[shadowed]), 0);
        assert_eq!(
            shadow_box,
            pixels(plain_box.x0, plain_box.y0, plain_box.x1 + 6, plain_box.y1)
        );
    }

    #[test]
    fn each_block_of_a_shadowed_unit_keeps_a_box_of_its_own() {
        let t = plain();
        let texts = [
            label("Top", 10.0, 5.0).with_shadow(0, 0, 0, 255, 1.0, 1.0, 0.0),
            label("Low", 60.0, 70.0),
        ];
        let prepared = prepare(&texts);
        assert_eq!(prepared.text_units.len(), 1);
        let (top, low) = (text_block(&t, &prepared, 0), text_block(&t, &prepared, 1));
        // Alone, each is its own block's box; together they would cover the
        // space between.
        assert_eq!(top, text_block(&t, &prepare(&texts[..1]), 0));
        assert_eq!(low, text_block(&t, &prepare(&texts[1..]), 0));
        assert!(!top.overlaps(&low));
    }

    #[cfg(feature = "phosphor-icons")]
    #[test]
    fn an_msdf_icon_is_its_fitted_quad_past_its_rect() {
        use crate::render::PhosphorIcon;
        use crate::widgets::DrawList;
        let t = plain();
        let mut list = DrawList::new();
        list.phosphor_icon(
            Rect::new(20.0, 20.0, 16.0, 16.0),
            PhosphorIcon::Check,
            [1.0; 4],
        );
        list.push_clip(Rect::new(0.0, 0.0, 28.0, 100.0));
        list.phosphor_icon(
            Rect::new(20.0, 50.0, 16.0, 16.0),
            PhosphorIcon::Check,
            [1.0; 4],
        );
        list.pop_clip();
        let fs = shared_font_system();
        let mut cpu = MsdfCpu::new(fs);
        let mut prepared = MsdfPrepared::default();
        cpu.prepare_icons(&mut prepared, &list.icons_msdf);
        let whole = icon_msdf(&t, &prepared, 0);
        // The padded tile is fitted into the rect along its tighter axis, here
        // the height, and reaches past the rect along the other.
        assert_eq!((whole.y0, whole.y1), (20, 36));
        assert!(whole.x0 < 20 && whole.x1 > 36);
        // The clip cuts the second at x 28: pixel centres up to 27.5.
        let cut = icon_msdf(&t, &prepared, 1);
        assert_eq!((cut.x0, cut.x1), (whole.x0, 28));
        assert_eq!(cut.y1 - cut.y0, whole.y1 - whole.y0);
    }
}

/// Draws each kind alone on a GPU and checks that no pixel outside its box
/// changed: the boxes above are only as good as their match with the shaders.
#[cfg(feature = "headless")]
mod gpu {
    use super::*;
    use crate::HeadlessGpu;
    use crate::render::{NineSliceId, Placement, SpriteId, YuvColour, YuvPlanes, chroma_size};
    use crate::shadow::{BoxShadow, CornerRadii};
    use crate::text::TextBlock;
    use crate::widgets::{Cap, Join, Stripes, Stroke};

    const SIZE: (u32, u32) = (96, 96);
    const WHITE: [f32; 4] = [1.0; 4];
    /// Below 1 a logical px of padding is less than the 0.61 physical px of
    /// reach, so the quad, not the reach, bounds what draws.
    const SCALES: [f32; 5] = [0.75, 1.0, 1.25, 1.5, 2.0];
    const ORIGINS: [[f32; 2]; 2] = [[0.0, 0.0], [3.0, -5.0]];
    /// Where in a pixel the scene starts, swept across it. The last ones move
    /// x and y apart, so a corner's furthest reach (0.6 px out along one axis,
    /// a quarter px along the other) lands near a pixel centre.
    const OFFSETS: [[f32; 2]; 10] = [
        [0.0, 0.0],
        [0.125, 0.875],
        [0.3, 0.7],
        [0.5, 0.5],
        [0.71, 0.29],
        [0.9375, 0.0625],
        [0.1, 0.25],
        [0.25, 0.1],
        [0.9, 0.75],
        [0.75, 0.9],
    ];

    /// Draws one primitive with its top-left near `at`, in about 24×16
    /// logical px.
    type Scene = Box<dyn Fn(&mut DrawList, [f32; 2])>;

    struct Ids {
        sprite: SpriteId,
        yuv: SpriteId,
        nine: NineSliceId,
    }

    fn load(gpu: &mut HeadlessGpu) -> Ids {
        let ui = gpu.renderer();
        let sprite = ui
            .load_sprite_rgba8("box", 8, 8, vec![255u8; 8 * 8 * 4], Placement::Own)
            .expect("sprite");
        let (cw, ch) = chroma_size(8, 8);
        let planes = YuvPlanes::packed(
            8,
            vec![200u8; 64],
            vec![128u8; (cw * ch) as usize],
            vec![128u8; (cw * ch) as usize],
        );
        let yuv = ui
            .create_yuv_sprite("box yuv", 8, 8, planes, YuvColour::assumed_for(8, 8))
            .expect("yuv sprite");
        let nine = ui.register_nine_slice("box nine", sprite, [2, 2, 2, 2]);
        Ids { sprite, yuv, nine }
    }

    /// Each kind, plain and under a transform: rotation, uneven scale.
    fn scenes(ids: &Ids) -> Vec<(&'static str, Scene)> {
        let (sprite, yuv, nine) = (ids.sprite, ids.yuv, ids.nine);
        let r = |at: [f32; 2], w: f32, h: f32| Rect::new(at[0], at[1], w, h);
        // Runs `draw` turned and squashed about `at`.
        fn turned(list: &mut DrawList, at: [f32; 2], draw: impl Fn(&mut DrawList)) {
            list.push_transform();
            list.translate(at[0] + 8.0, at[1] + 6.0);
            list.rotate(0.3);
            list.scale(1.4, 0.7);
            draw(list);
            list.pop_transform();
        }
        // Runs `draw` mirrored left to right about `at`: a negative
        // determinant.
        fn flipped(list: &mut DrawList, at: [f32; 2], draw: impl Fn(&mut DrawList)) {
            list.push_transform();
            list.translate(at[0] + 8.0, at[1] + 6.0);
            list.scale(-1.0, 1.0);
            draw(list);
            list.pop_transform();
        }
        let shadow = BoxShadow {
            offset: [2.0, 3.0],
            blur: 4.0,
            spread: 1.0,
            color: WHITE,
            inset: false,
        };
        let mut scenes: Vec<(&'static str, Scene)> = vec![
            (
                "chrome",
                Box::new(move |l: &mut DrawList, at| {
                    l.chrome_rect(r(at, 20.0, 12.0), 0.0, 1.0, WHITE, WHITE)
                }),
            ),
            (
                "chrome r0.25",
                Box::new(move |l: &mut DrawList, at| {
                    l.chrome_rect(r(at, 20.0, 12.0), 0.25, 1.0, WHITE, WHITE)
                }),
            ),
            (
                "chrome r4",
                Box::new(move |l: &mut DrawList, at| {
                    l.chrome_rect(r(at, 20.0, 12.0), 4.0, 2.0, WHITE, WHITE)
                }),
            ),
            (
                "chrome large radius",
                Box::new(move |l: &mut DrawList, at| {
                    l.chrome_rect(r(at, 24.0, 16.0), 14.0, 0.0, WHITE, WHITE)
                }),
            ),
            (
                "chrome tiny",
                Box::new(move |l: &mut DrawList, at| {
                    l.chrome_rect(r(at, 0.4, 0.3), 0.0, 0.0, WHITE, WHITE)
                }),
            ),
            (
                "chrome turned",
                Box::new(move |l: &mut DrawList, at| {
                    turned(l, at, |l| {
                        l.chrome_rect(Rect::new(-6.0, -4.0, 12.0, 8.0), 3.0, 1.0, WHITE, WHITE)
                    })
                }),
            ),
            (
                "chrome flipped",
                Box::new(move |l: &mut DrawList, at| {
                    flipped(l, at, |l| {
                        l.chrome_rect(Rect::new(-6.0, -4.0, 12.0, 8.0), 3.0, 1.0, WHITE, WHITE)
                    })
                }),
            ),
            (
                "stripes",
                Box::new(move |l: &mut DrawList, at| {
                    let s = Stripes {
                        normal: [1.0, 0.5],
                        period: 4.0,
                        width: 1.5,
                        offset: 0.0,
                        color: WHITE,
                        crisp: false,
                    };
                    l.stripes(r(at, 20.0, 12.0), s)
                }),
            ),
            (
                "stripes crisp",
                Box::new(move |l: &mut DrawList, at| {
                    l.stripes(r(at, 20.0, 12.0), Stripes::hatch(3.0, WHITE))
                }),
            ),
            (
                "stripes turned",
                Box::new(move |l: &mut DrawList, at| {
                    turned(l, at, |l| {
                        l.stripes(Rect::new(-6.0, -4.0, 12.0, 8.0), Stripes::hatch(3.0, WHITE))
                    })
                }),
            ),
            (
                "shadow",
                Box::new(move |l: &mut DrawList, at| {
                    l.box_shadow_outset(
                        r([at[0] + 4.0, at[1] + 3.0], 12.0, 8.0),
                        CornerRadii::uniform(3.0),
                        shadow,
                    )
                }),
            ),
            (
                "shadow crisp",
                Box::new(move |l: &mut DrawList, at| {
                    l.box_shadow_outset(
                        r([at[0] + 2.0, at[1] + 2.0], 12.0, 8.0),
                        CornerRadii::uniform(0.0),
                        BoxShadow {
                            blur: 0.0,
                            ..shadow
                        },
                    )
                }),
            ),
            (
                "shadow inset",
                Box::new(move |l: &mut DrawList, at| {
                    l.box_shadow_inset(
                        r(at, 20.0, 12.0),
                        CornerRadii::uniform(2.0),
                        BoxShadow {
                            inset: true,
                            ..shadow
                        },
                    )
                }),
            ),
            (
                "shadow turned",
                Box::new(move |l: &mut DrawList, at| {
                    turned(l, at, |l| {
                        l.box_shadow_outset(
                            Rect::new(-4.0, -3.0, 8.0, 6.0),
                            CornerRadii::uniform(2.0),
                            shadow,
                        )
                    })
                }),
            ),
            (
                "line",
                Box::new(move |l: &mut DrawList, at| {
                    l.line(
                        [at[0], at[1] + 4.0],
                        [at[0] + 20.0, at[1] + 9.0],
                        1.0,
                        WHITE,
                    )
                }),
            ),
            (
                "line round",
                Box::new(move |l: &mut DrawList, at| {
                    l.stroke_line(
                        [at[0] + 2.0, at[1] + 2.0],
                        [at[0] + 18.0, at[1] + 12.0],
                        &Stroke::new(3.0).cap(Cap::Round),
                        WHITE,
                    )
                }),
            ),
            (
                "line dashed square",
                Box::new(move |l: &mut DrawList, at| {
                    l.stroke_line(
                        [at[0] + 2.0, at[1] + 6.0],
                        [at[0] + 22.0, at[1] + 6.0],
                        &Stroke::new(2.0).cap(Cap::Square).dashed(3.0, 2.0),
                        WHITE,
                    )
                }),
            ),
            (
                "line dashed round",
                Box::new(move |l: &mut DrawList, at| {
                    l.stroke_line(
                        [at[0] + 2.0, at[1] + 3.0],
                        [at[0] + 20.0, at[1] + 10.0],
                        &Stroke::new(2.5).cap(Cap::Round).dashed(2.0, 3.0),
                        WHITE,
                    )
                }),
            ),
            (
                "closed round join",
                Box::new(move |l: &mut DrawList, at| {
                    let p = [
                        [at[0] + 2.0, at[1] + 2.0],
                        [at[0] + 20.0, at[1] + 4.0],
                        [at[0] + 10.0, at[1] + 12.0],
                    ];
                    l.stroke_closed(&p, &Stroke::new(2.0).join(Join::Round), WHITE)
                }),
            ),
            (
                "polyline miter",
                Box::new(move |l: &mut DrawList, at| {
                    let p = [
                        [at[0], at[1] + 12.0],
                        [at[0] + 8.0, at[1]],
                        [at[0] + 16.0, at[1] + 12.0],
                        [at[0] + 22.0, at[1] + 4.0],
                    ];
                    l.stroke_polyline(&p, &Stroke::new(2.0).join(Join::Miter), WHITE)
                }),
            ),
            (
                "polyline bevel",
                Box::new(move |l: &mut DrawList, at| {
                    let p = [
                        [at[0], at[1] + 12.0],
                        [at[0] + 8.0, at[1] + 1.0],
                        [at[0] + 16.0, at[1] + 12.0],
                    ];
                    l.stroke_polyline(&p, &Stroke::new(3.0).join(Join::Bevel), WHITE)
                }),
            ),
            (
                "line turned",
                Box::new(move |l: &mut DrawList, at| {
                    turned(l, at, |l| {
                        l.stroke_line(
                            [-6.0, -2.0],
                            [6.0, 3.0],
                            &Stroke::new(1.5).cap(Cap::Round),
                            WHITE,
                        )
                    })
                }),
            ),
            (
                "line flipped",
                Box::new(move |l: &mut DrawList, at| {
                    flipped(l, at, |l| {
                        l.stroke_line(
                            [-6.0, -2.0],
                            [6.0, 3.0],
                            &Stroke::new(1.5).cap(Cap::Round),
                            WHITE,
                        )
                    })
                }),
            ),
            (
                "circle",
                Box::new(move |l: &mut DrawList, at| {
                    l.circle((at[0] + 8.0, at[1] + 7.0), 6.0, WHITE)
                }),
            ),
            (
                "circle outline",
                Box::new(move |l: &mut DrawList, at| {
                    l.circle_outline((at[0] + 8.0, at[1] + 7.0), 6.0, 2.0, WHITE)
                }),
            ),
            (
                "circle tiny",
                Box::new(move |l: &mut DrawList, at| {
                    l.circle((at[0] + 1.0, at[1] + 1.0), 0.3, WHITE)
                }),
            ),
            (
                "soup triangle",
                Box::new(move |l: &mut DrawList, at| {
                    l.triangle(
                        (at[0], at[1]),
                        (at[0] + 20.0, at[1] + 3.0),
                        (at[0] + 6.0, at[1] + 12.0),
                        WHITE,
                    )
                }),
            ),
            (
                "soup turned",
                Box::new(move |l: &mut DrawList, at| {
                    turned(l, at, |l| l.quad(-6.0, -4.0, 12.0, 8.0, WHITE))
                }),
            ),
            (
                "soup flipped",
                Box::new(move |l: &mut DrawList, at| {
                    flipped(l, at, |l| l.quad(-6.0, -4.0, 12.0, 8.0, WHITE))
                }),
            ),
            (
                "radial gradient",
                Box::new(move |l: &mut DrawList, at| {
                    l.radial_gradient(r(at, 20.0, 12.0), WHITE, WHITE, 12)
                }),
            ),
            (
                "sprite",
                Box::new(move |l: &mut DrawList, at| l.image(sprite, r(at, 20.0, 12.0), WHITE)),
            ),
            (
                "sprite turned",
                Box::new(move |l: &mut DrawList, at| {
                    turned(l, at, |l| {
                        l.image(sprite, Rect::new(-6.0, -4.0, 12.0, 8.0), WHITE)
                    })
                }),
            ),
            (
                "sprite flipped",
                Box::new(move |l: &mut DrawList, at| {
                    flipped(l, at, |l| {
                        l.image(sprite, Rect::new(-6.0, -4.0, 12.0, 8.0), WHITE)
                    })
                }),
            ),
            (
                "yuv",
                Box::new(move |l: &mut DrawList, at| l.image(yuv, r(at, 20.0, 12.0), WHITE)),
            ),
            (
                "nine-slice",
                Box::new(move |l: &mut DrawList, at| {
                    l.nine_slice_id(nine, at[0], at[1], 20.0, 12.0, WHITE)
                }),
            ),
            (
                "nine-slice turned",
                Box::new(move |l: &mut DrawList, at| {
                    turned(l, at, |l| {
                        l.nine_slice_id(nine, -6.0, -4.0, 12.0, 8.0, WHITE)
                    })
                }),
            ),
        ];
        // Text with each effect: its quads reach past the glyph by the
        // field's ramp, and the effects' further still.
        let text = |at: [f32; 2]| TextBlock::new("Agy", at[0], at[1]).with_size(14.0);
        scenes.extend([
            (
                "text",
                Box::new(move |l: &mut DrawList, at: [f32; 2]| l.text(text(at))) as Scene,
            ),
            (
                "text outline",
                Box::new(move |l: &mut DrawList, at| {
                    l.text(text(at).with_outline(255, 255, 255, 255, 1.5))
                }),
            ),
            (
                "text shadow",
                Box::new(move |l: &mut DrawList, at| {
                    l.text(text(at).with_shadow(255, 255, 255, 255, 2.0, 2.0, 2.0))
                }),
            ),
            (
                "text glow",
                Box::new(move |l: &mut DrawList, at| {
                    l.text(text(at).with_glow(255, 255, 255, 255, 3.0))
                }),
            ),
            (
                "text scaled",
                Box::new(move |l: &mut DrawList, at| {
                    l.push_transform();
                    l.translate(at[0], at[1]);
                    l.scale(1.5, 1.5);
                    l.text(text([0.0, 0.0]).with_glow(255, 255, 255, 255, 2.0));
                    l.pop_transform();
                }),
            ),
        ]);
        #[cfg(feature = "phosphor-icons")]
        {
            use crate::render::PhosphorIcon;
            scenes.extend([
                (
                    "icon",
                    Box::new(move |l: &mut DrawList, at: [f32; 2]| {
                        l.phosphor_icon(r(at, 16.0, 16.0), PhosphorIcon::Check, WHITE)
                    }) as Scene,
                ),
                (
                    "icon turned",
                    Box::new(move |l: &mut DrawList, at| {
                        turned(l, at, |l| {
                            l.phosphor_icon(
                                Rect::new(-6.0, -6.0, 12.0, 12.0),
                                PhosphorIcon::Plus,
                                WHITE,
                            )
                        })
                    }),
                ),
            ]);
        }
        scenes
    }

    /// The union of every recorded primitive's box, with text and MSDF icons
    /// as the renderer prepared them.
    fn list_box(t: &Target, list: &DrawList, prepared: &MsdfPrepared) -> PixelBox {
        let mut b = soup(t, &list.vertices, &list.indices);
        for block in 0..list.texts.len() {
            b = b.union(&text_block(t, prepared, block));
        }
        #[cfg(feature = "phosphor-icons")]
        for i in 0..list.icons_msdf.len() as u32 {
            b = b.union(&icon_msdf(t, prepared, i));
        }
        for r in &list.analytic_instances {
            b = b.union(&analytic(t, r));
        }
        for c in &list.circle_instances {
            b = b.union(&circle(t, c));
        }
        for d in &list.icons {
            b = b.union(&icon(t, d));
        }
        for d in &list.nine_slices {
            b = b.union(&nine_slice(t, d));
        }
        b
    }

    /// Renders `list` onto `t` and returns how many pixels changed, failing if
    /// any of them is outside the list's box.
    fn changed_inside_box(
        gpu: &mut HeadlessGpu,
        list: &DrawList,
        t: &Target,
        label: &str,
    ) -> usize {
        gpu.renderer().set_view_origin(t.origin[0], t.origin[1]);
        let pixels = gpu.capture_scaled(list, t.size, t.scale);
        gpu.renderer().set_view_origin(0.0, 0.0);
        let b = list_box(t, list, gpu.renderer().text_prepared());
        let mut changed = 0;
        let mut outside = Vec::new();
        for (i, p) in pixels.chunks_exact(4).enumerate() {
            if p.iter().all(|&c| c == 0) {
                continue;
            }
            changed += 1;
            let (x, y) = (i as u32 % t.size.0, i as u32 / t.size.0);
            if !b.contains(x, y) {
                outside.push((x, y, [p[0], p[1], p[2], p[3]]));
            }
        }
        assert!(
            outside.is_empty(),
            "{label}: {} pixels changed outside {b:?}, first {:?}",
            outside.len(),
            &outside[..outside.len().min(5)]
        );
        changed
    }

    fn draw(scene: &Scene, gpu: &HeadlessGpu, at: [f32; 2]) -> DrawList {
        let mut list = gpu.draw_list();
        scene(&mut list, at);
        list
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn no_primitive_changes_a_pixel_outside_its_box() {
        let Some(mut gpu) = HeadlessGpu::new() else {
            eprintln!("no GPU adapter; skipping");
            return;
        };
        let ids = load(&mut gpu);
        for (name, scene) in scenes(&ids) {
            let mut drew = false;
            for scale in SCALES {
                for origin in ORIGINS {
                    let t = Target::new(SIZE, scale, origin);
                    for offset in OFFSETS {
                        let at = [origin[0] + 8.0 + offset[0], origin[1] + 8.0 + offset[1]];
                        let list = draw(&scene, &gpu, at);
                        let label = format!("{name} at {at:?}, scale {scale}, origin {origin:?}");
                        drew |= changed_inside_box(&mut gpu, &list, &t, &label) > 0;
                    }
                }
            }
            // A tiny shape can leave nothing an 8-bit target shows at some
            // offsets, but not at all of them.
            assert!(drew, "{name}: nothing drew");
        }
    }

    /// Clip edges on pixel centres, cutting through each primitive, and clips
    /// that miss each other (a clip of zero size).
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn clipped_primitives_change_no_pixel_outside_their_box() {
        let Some(mut gpu) = HeadlessGpu::new() else {
            eprintln!("no GPU adapter; skipping");
            return;
        };
        let ids = load(&mut gpu);
        for (name, scene) in scenes(&ids) {
            let mut drew = false;
            for scale in SCALES {
                let origin = ORIGINS[1];
                let t = Target::new(SIZE, scale, origin);
                let at = [origin[0] + 8.3, origin[1] + 8.6];
                // From the first pixel centre at or past `at`, 18 by 8
                // physical px: through the middle of every scene.
                let first = |p: f32, o: f32| o + (((p - o) * scale).floor() + 0.5) / scale;
                let clip = Rect::new(
                    first(at[0], origin[0]),
                    first(at[1], origin[1]),
                    18.0 / scale,
                    8.0 / scale,
                );
                let mut list = gpu.draw_list();
                list.push_clip_exact(clip);
                scene(&mut list, at);
                list.pop_clip();
                let label = format!("{name} clipped, scale {scale}");
                drew |= changed_inside_box(&mut gpu, &list, &t, &label) > 0;

                let mut list = gpu.draw_list();
                list.push_clip(Rect::new(origin[0] + 60.0, origin[1] + 60.0, 4.0, 4.0));
                list.push_clip(Rect::new(at[0] + 2.0, at[1] + 2.0, 10.0, 10.0));
                scene(&mut list, at);
                list.pop_clip();
                list.pop_clip();
                let label = format!("{name} in clips that miss, scale {scale}");
                assert_eq!(
                    changed_inside_box(&mut gpu, &list, &t, &label),
                    0,
                    "{label}"
                );
                let b = list_box(&t, &list, gpu.renderer().text_prepared());
                assert!(b.is_empty(), "{label}");
            }
            assert!(drew, "{name}: nothing drew inside the clip");
        }
    }

    /// A target whose size doesn't divide by the scale, with each primitive
    /// near its far corner; and positions and a view origin around 1e5.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn odd_targets_and_far_coordinates_change_no_pixel_outside_the_box() {
        let Some(mut gpu) = HeadlessGpu::new() else {
            eprintln!("no GPU adapter; skipping");
            return;
        };
        let ids = load(&mut gpu);
        for (name, scene) in scenes(&ids) {
            let odd = Target::new((101, 61), 1.25, [0.0, 0.0]);
            // 80.8 by 48.8 logical: the scene's far corner a little past it.
            let mut drew_odd = false;
            for offset in OFFSETS {
                let at = [58.3 + offset[0], 34.7 + offset[1]];
                let list = draw(&scene, &gpu, at);
                let label = format!("{name} at {at:?} on 101×61 at 1.25");
                drew_odd |= changed_inside_box(&mut gpu, &list, &odd, &label) > 0;
            }
            assert!(drew_odd, "{name}: nothing drew on the odd target");

            let mut drew_far = false;
            for scale in SCALES {
                let origin = [1e5, 1e5 + 3.0];
                let t = Target::new(SIZE, scale, origin);
                for offset in OFFSETS {
                    let at = [origin[0] + 8.0 + offset[0], origin[1] + 8.0 + offset[1]];
                    let list = draw(&scene, &gpu, at);
                    let label = format!("{name} at {at:?}, scale {scale}");
                    drew_far |= changed_inside_box(&mut gpu, &list, &t, &label) > 0;
                }
            }
            assert!(drew_far, "{name}: nothing drew around 1e5");
        }
    }
}
