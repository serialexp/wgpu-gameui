//! `DrawList::append` against recording: a frame recorded in pieces, each
//! piece into its own list and the pieces appended in order, must be the list
//! the same calls make when recorded into one list, record for record. This
//! is what lets a retained piece of a frame be drawn again by appending it.
//!
//! `DrawList::append_translated` likewise: a piece recorded at its own origin
//! and placed with an offset and a clip must be the list its calls make when
//! recorded inside `push_clip(clip)` and `translate(offset)`. Coordinates here
//! are quarter pixels and offsets whole ones, so every sum is exact and the
//! comparison can be too.

use crate::layout::Rect;
use crate::render::draw_box::Target;
use crate::render::draw_order::tests_support::Rng;
use crate::render::draw_plan::tests_support::Lists;
use crate::render::frame_diff::{Change, ContextKey, FrameSnapshot};
use crate::text::TextBlock;
use crate::text_select::TextKey;
use crate::widgets::{DrawList, PlacedLog, RecordCounts, RecordSpan, Stroke};

/// One drawing call, of every kind of record a list holds.
#[derive(Clone, Copy, Debug)]
enum Op {
    Triangle,
    Quad,
    Chrome,
    Circle,
    Line,
    Shadow,
    Text,
    TextWithShadow,
    SelectableText,
    Icon,
    NineSlice,
    #[cfg(feature = "phosphor-icons")]
    Phosphor,
    ClipPush,
    ClipPop,
    Repaint,
    Stripes,
    Polyline,
    Underlined,
    Tinted,
    Scaled,
    ViewportClip,
}

const OPS: &[Op] = &[
    Op::Triangle,
    Op::Quad,
    Op::Chrome,
    Op::Circle,
    Op::Line,
    Op::Shadow,
    Op::Text,
    Op::TextWithShadow,
    Op::SelectableText,
    Op::Icon,
    Op::NineSlice,
    #[cfg(feature = "phosphor-icons")]
    Op::Phosphor,
    Op::ClipPush,
    Op::ClipPop,
    Op::Repaint,
    Op::Stripes,
    Op::Polyline,
    Op::Underlined,
    Op::Tinted,
    Op::Scaled,
    Op::ViewportClip,
];

impl Op {
    fn pushes_clip(self) -> bool {
        matches!(self, Op::ClipPush | Op::ViewportClip)
    }
}

/// A call and where it draws.
#[derive(Clone, Copy, Debug)]
struct Call {
    op: Op,
    at: [f32; 2],
    n: u32,
}

impl Call {
    fn draw(&self, list: &mut DrawList) {
        let [x, y] = self.at;
        let colour = [0.2 + (self.n % 7) as f32 / 10.0, 0.5, 0.7, 0.9];
        let r = Rect::new(x, y, 40.0 + (self.n % 30) as f32, 20.0);
        match self.op {
            Op::Triangle => list.triangle((x, y), (x + 30.0, y), (x, y + 20.0), colour),
            Op::Quad => list.quad(x, y, 30.0, 12.0, colour),
            Op::Chrome => list.chrome_rect(r, 3.0, 1.0, colour, [0.5; 4]),
            Op::Circle => list.circle((x, y), 8.0, colour),
            Op::Line => list.line([x, y], [x + 50.0, y + 9.0], 2.0, colour),
            Op::Shadow => list.box_shadow_outset(
                r,
                crate::shadow::CornerRadii::uniform(4.0),
                crate::shadow::BoxShadow {
                    offset: [0.0, 2.0],
                    blur: 6.0,
                    spread: 0.0,
                    color: [0.0, 0.0, 0.0, 0.5],
                    inset: false,
                },
            ),
            Op::Text => {
                list.text(TextBlock::new(format!("label {}", self.n), x, y).with_size(12.0))
            }
            Op::TextWithShadow => list.text(
                TextBlock::new(format!("shadowed {}", self.n), x, y)
                    .with_size(13.0)
                    .with_shadow(0, 0, 0, 180, 1.0, 1.0, 1.0),
            ),
            Op::SelectableText => list.text(
                TextBlock::new(format!("pick {}", self.n), x, y)
                    .with_size(12.0)
                    .selectable(TextKey::new(1, self.n as u64)),
            ),
            Op::Icon => list.icon("missing", x, y, 16.0, 16.0),
            Op::NineSlice => list.nine_slice(x, y, 60.0, 30.0, "panel"),
            #[cfg(feature = "phosphor-icons")]
            Op::Phosphor => list.phosphor_icon(r, crate::PhosphorIcon::Gear, colour),
            Op::ClipPush => list.push_clip(Rect::new(x, y, 200.0, 120.0)),
            Op::ClipPop => list.pop_clip(),
            Op::Repaint => list.repaint_after((self.n % 5) as f32 * 0.25),
            Op::Stripes => list.hatch(r, 6.0, colour),
            // Axis-aligned and a whole pixel wide, so snapped to the grid,
            // with a join whose neighbours are real points.
            Op::Polyline => list.stroke_polyline(
                &[[x, y], [x + 40.0, y], [x + 40.0, y + 25.0]],
                &Stroke::new(1.0),
                colour,
            ),
            Op::Underlined => list.text(
                TextBlock::new(format!("under {}", self.n), x, y)
                    .with_size(14.0)
                    .with_style_ranges(vec![crate::TextStyleRange {
                        range: 0..5,
                        color: None,
                        underline: crate::Underline::Color(colour),
                    }]),
            ),
            Op::Tinted => {
                list.push_tint();
                list.multiply_tint([0.5, 1.0, 0.75, 0.5]);
                list.chrome_rect(r, 2.0, 1.0, colour, [0.25; 4]);
                list.quad(x, y, 10.0, 10.0, colour);
                list.pop_tint();
            }
            // Every record kind that keeps a transform rather than baking a
            // translation: a scale whose products stay exact.
            Op::Scaled => {
                list.push_transform();
                list.translate(x, y);
                list.scale(2.0, 0.5);
                list.chrome_rect(Rect::new(0.0, 0.0, 20.0, 30.0), 3.0, 1.0, colour, [0.5; 4]);
                list.line([0.0, 0.0], [25.0, 8.0], 2.0, colour);
                list.stroke_polyline(
                    &[[0.0, 0.0], [20.0, 0.0], [20.0, 16.0]],
                    &Stroke::new(1.5),
                    colour,
                );
                list.hatch(Rect::new(0.0, 0.0, 30.0, 20.0), 5.0, colour);
                list.box_shadow_outset(
                    Rect::new(0.0, 0.0, 20.0, 20.0),
                    crate::shadow::CornerRadii::uniform(2.0),
                    crate::shadow::BoxShadow {
                        offset: [1.0, 1.0],
                        blur: 4.0,
                        spread: 0.0,
                        color: [0.0, 0.0, 0.0, 0.5],
                        inset: false,
                    },
                );
                list.icon("missing", 0.0, 0.0, 8.0, 8.0);
                list.nine_slice(0.0, 0.0, 16.0, 16.0, "panel");
                #[cfg(feature = "phosphor-icons")]
                list.phosphor_icon(
                    Rect::new(0.0, 0.0, 12.0, 12.0),
                    crate::PhosphorIcon::Gear,
                    colour,
                );
                list.triangle((0.0, 0.0), (8.0, 0.0), (0.0, 8.0), colour);
                list.pop_transform();
            }
            Op::ViewportClip => list.push_clip_viewport(Rect::new(x, y, 160.0, 90.0)),
        }
    }
}

fn random_calls(rng: &mut Rng, len: usize) -> Vec<Call> {
    (0..len)
        .map(|_| Call {
            op: OPS[rng.below(OPS.len() as u32) as usize],
            at: [rng.below(560) as f32, rng.below(420) as f32],
            n: rng.below(1000),
        })
        .collect()
}

/// `calls` recorded into one list.
fn recorded(lists: &Lists, calls: &[Call]) -> DrawList {
    let mut list = lists.list();
    for call in calls {
        call.draw(&mut list);
    }
    list
}

/// `calls` cut at `cuts`, the first piece recorded into the list and every
/// later one into a list of its own and appended. A clip is a call's own
/// state, so a piece that pops a clip pushed in an earlier one pops nothing:
/// pieces are cut only where no clip is open, as a retained subtree is.
fn appended(lists: &Lists, calls: &[Call], cuts: &[usize]) -> DrawList {
    let mut list = lists.list();
    let mut start = 0;
    for (i, &end) in cuts.iter().chain([&calls.len()]).enumerate() {
        if i == 0 {
            for call in &calls[start..end] {
                call.draw(&mut list);
            }
        } else {
            list.append(&recorded(lists, &calls[start..end]));
        }
        start = end;
    }
    list
}

/// Places where no clip is open: where a sequence can be cut into pieces.
fn closed_points(calls: &[Call]) -> Vec<usize> {
    let mut depth = 0i32;
    let mut points = Vec::new();
    for (i, call) in calls.iter().enumerate() {
        if depth == 0 {
            points.push(i);
        }
        match call.op {
            op if op.pushes_clip() => depth += 1,
            Op::ClipPop => depth = (depth - 1).max(0),
            _ => {}
        }
    }
    if depth == 0 {
        points.push(calls.len());
    }
    points
}

fn assert_same(direct: &DrawList, pieces: &DrawList, what: &str) {
    assert_eq!(direct.difference(pieces), None, "{what}: difference");
    assert_eq!(direct.paint_cmds, pieces.paint_cmds, "{what}: paint stream");
    assert_eq!(
        direct.soup_committed_indices, pieces.soup_committed_indices,
        "{what}: trailing soup"
    );
    assert_eq!(direct.vertices, pieces.vertices, "{what}: vertices");
    assert_eq!(direct.indices, pieces.indices, "{what}: indices");
    assert_eq!(
        direct.analytic_instances, pieces.analytic_instances,
        "{what}: analytic"
    );
    assert_eq!(
        direct.circle_instances, pieces.circle_instances,
        "{what}: circles"
    );
    assert_eq!(direct.prim_counts(), pieces.prim_counts(), "{what}: counts");
    assert_eq!(
        (
            direct.chrome_instance_count(),
            direct.shadow_instance_count(),
            direct.stripe_instance_count(),
            direct.segment_instance_count()
        ),
        (
            pieces.chrome_instance_count(),
            pieces.shadow_instance_count(),
            pieces.stripe_instance_count(),
            pieces.segment_instance_count()
        ),
        "{what}: analytic kinds"
    );
    let selectable = |list: &DrawList| -> Vec<(TextKey, f32, f32)> {
        list.selectable_texts()
            .map(|(key, block)| (key, block.x, block.y))
            .collect()
    };
    assert_eq!(selectable(direct), selectable(pieces), "{what}: selectable");
    assert_eq!(
        direct.next_repaint(),
        pieces.next_repaint(),
        "{what}: repaint"
    );
    assert_eq!(
        direct.dropped_degenerate(),
        pieces.dropped_degenerate(),
        "{what}: dropped"
    );
    assert_eq!(
        direct.viewport_clips(),
        pieces.viewport_clips(),
        "{what}: viewport clips"
    );
    let texts = |list: &DrawList| -> Vec<(f32, f32, Option<Rect>)> {
        list.texts.iter().map(|t| (t.x, t.y, t.clip)).collect()
    };
    assert_eq!(texts(direct), texts(pieces), "{what}: text places");
    let icons = |list: &DrawList| -> Vec<([[f32; 2]; 4], Option<Rect>)> {
        list.icons.iter().map(|i| (i.corners, i.clip)).collect()
    };
    assert_eq!(icons(direct), icons(pieces), "{what}: icons");
    let nine_slices = |list: &DrawList| -> Vec<(crate::Affine2, Option<Rect>)> {
        list.nine_slices
            .iter()
            .map(|n| (n.transform, n.clip))
            .collect()
    };
    assert_eq!(
        nine_slices(direct),
        nine_slices(pieces),
        "{what}: nine-slices"
    );
    #[cfg(feature = "phosphor-icons")]
    assert_eq!(direct.icons_msdf, pieces.icons_msdf, "{what}: msdf icons");
    // Every other record (text blocks, icons, nine-slices), compared the way
    // the renderer compares frames: identical means the plan, the instances
    // and the draws come out the same.
    let context = test_context();
    let mut snapshot = FrameSnapshot::default();
    snapshot.keep(direct, &context);
    let diff = snapshot.compare(pieces, &context);
    assert!(diff.identical(), "{what}: {diff:?}");
}

#[test]
fn difference_names_the_first_record_that_differs() {
    let lists = Lists::new();
    let draw = |list: &mut DrawList, colour: f32, text: &str, x: f32| {
        // A quad (analytic instance 0), the chrome (1), the text.
        list.quad(0.0, 0.0, 8.0, 8.0, [1.0, 0.0, 0.0, 1.0]);
        list.chrome_rect(
            Rect::new(x, 2.0, 20.0, 10.0),
            3.0,
            1.0,
            [colour, 0.5, 0.5, 1.0],
            [0.0; 4],
        );
        list.text(TextBlock::new(text, 4.0, 4.0));
    };
    let mut a = lists.list();
    draw(&mut a, 0.25, "a", 2.0);
    let mut same = lists.list();
    draw(&mut same, 0.25, "a", 2.0);
    assert_eq!(a.difference(&same), None);

    let mut recoloured = lists.list();
    draw(&mut recoloured, 0.75, "a", 2.0);
    let found = a.difference(&recoloured).expect("a colour differs");
    assert!(found.starts_with("analytic instance 1"), "{found}");

    let mut moved = lists.list();
    draw(&mut moved, 0.25, "a", 3.0);
    let found = a.difference(&moved).expect("a place differs");
    assert!(found.starts_with("analytic instance 1"), "{found}");

    let mut retext = lists.list();
    draw(&mut retext, 0.25, "b", 2.0);
    let found = a.difference(&retext).expect("a text differs");
    assert!(found.starts_with("text 0"), "{found}");

    let mut longer = lists.list();
    draw(&mut longer, 0.25, "a", 2.0);
    longer.repaint_after(1.0);
    let found = a.difference(&longer).expect("a repaint differs");
    assert!(found.starts_with("repaint"), "{found}");

    let mut more = lists.list();
    draw(&mut more, 0.25, "a", 2.0);
    more.quad(0.0, 0.0, 8.0, 8.0, [1.0, 0.0, 0.0, 1.0]);
    assert!(a.difference(&more).is_some());
}

#[test]
fn a_list_appended_in_pieces_is_the_list_recorded_in_one() {
    let lists = Lists::new();
    let mut rng = Rng(17);
    for round in 0..200 {
        let len = 4 + rng.below(14) as usize;
        let calls = random_calls(&mut rng, len);
        let direct = recorded(&lists, &calls);
        let points = closed_points(&calls);
        // Some cut points, in order, from where pieces may start.
        let cuts: Vec<usize> = points
            .iter()
            .copied()
            .filter(|_| rng.below(2) == 0)
            .collect();
        let pieces = appended(&lists, &calls, &cuts);
        assert_same(
            &direct,
            &pieces,
            &format!("round {round}, cuts {cuts:?}, {calls:?}"),
        );
    }
}

#[test]
fn every_way_of_cutting_one_sequence_gives_the_same_list() {
    let lists = Lists::new();
    // Soup before and after commands, runs of one family across a cut, and
    // a piece that is only soup.
    let at = |i: usize| [10.0 + 30.0 * i as f32, 20.0 + 11.0 * i as f32];
    let ops = [
        Op::Triangle,
        Op::Quad,
        Op::Chrome,
        Op::Chrome,
        Op::Triangle,
        Op::Text,
        Op::Text,
        Op::Circle,
        Op::Triangle,
        Op::Triangle,
        Op::Icon,
        Op::Triangle,
    ];
    let calls: Vec<Call> = ops
        .iter()
        .enumerate()
        .map(|(i, &op)| Call {
            op,
            at: at(i),
            n: i as u32,
        })
        .collect();
    let direct = recorded(&lists, &calls);
    let inner = calls.len() - 1;
    for mask in 0u32..(1 << inner) {
        let cuts: Vec<usize> = (0..inner)
            .filter(|bit| mask & (1 << bit) != 0)
            .map(|bit| bit + 1)
            .collect();
        let pieces = appended(&lists, &calls, &cuts);
        assert_same(&direct, &pieces, &format!("cuts {cuts:?}"));
    }
}

#[test]
fn appending_an_empty_list_changes_nothing() {
    let lists = Lists::new();
    let calls: Vec<Call> = [Op::Chrome, Op::Triangle]
        .iter()
        .map(|&op| Call {
            op,
            at: [5.0, 5.0],
            n: 3,
        })
        .collect();
    let direct = recorded(&lists, &calls);
    let mut list = recorded(&lists, &calls);
    list.append(&lists.list());
    assert_same(&direct, &list, "empty appended");
    let mut empty = lists.list();
    empty.append(&direct);
    assert_same(&direct, &empty, "appended to empty");
}
/// Calls at quarter-pixel coordinates either side of the origin, with every
/// clip closed (see [`closed`]).
fn random_placed_calls(rng: &mut Rng, len: usize) -> Vec<Call> {
    let calls = (0..len)
        .map(|_| Call {
            op: OPS[rng.below(OPS.len() as u32) as usize],
            at: [
                (rng.below(2400) as f32 - 600.0) * 0.25,
                (rng.below(1800) as f32 - 400.0) * 0.25,
            ],
            n: rng.below(1000),
        })
        .collect();
    closed(calls)
}

/// `calls` with every clip closed, nested or not: a pop with no clip open
/// becomes a quad, and the clips left open are popped at the end. A placed
/// list's records are placed under the clip it was placed with, so a piece
/// must not pop what it did not push.
fn closed(calls: Vec<Call>) -> Vec<Call> {
    let mut open = 0u32;
    let mut calls: Vec<Call> = calls
        .into_iter()
        .map(|mut call| {
            if call.op.pushes_clip() {
                open += 1;
            } else if matches!(call.op, Op::ClipPop) {
                if open == 0 {
                    call.op = Op::Quad;
                } else {
                    open -= 1;
                }
            }
            call
        })
        .collect();
    for _ in 0..open {
        calls.push(Call {
            op: Op::ClipPop,
            at: [0.0; 2],
            n: 0,
        });
    }
    calls
}

fn random_offset(rng: &mut Rng) -> [f32; 2] {
    [rng.below(600) as f32 - 300.0, rng.below(600) as f32 - 300.0]
}

fn random_clip(rng: &mut Rng) -> Option<Rect> {
    (rng.below(3) != 0).then(|| {
        Rect::new(
            rng.below(500) as f32 - 100.0,
            rng.below(400) as f32 - 100.0,
            20.0 + rng.below(400) as f32,
            20.0 + rng.below(300) as f32,
        )
    })
}

/// A piece placed by recording: `body` inside `push_clip(clip)` and
/// `translate(offset)`, between `before` and `after`.
fn recorded_placed(
    lists: &Lists,
    [before, body, after]: [&[Call]; 3],
    offset: [f32; 2],
    clip: Option<Rect>,
) -> DrawList {
    let mut list = recorded(lists, before);
    if let Some(clip) = clip {
        list.push_clip(clip);
    }
    list.push_transform();
    list.translate(offset[0], offset[1]);
    for call in body {
        call.draw(&mut list);
    }
    list.pop_transform();
    if clip.is_some() {
        list.pop_clip();
    }
    for call in after {
        call.draw(&mut list);
    }
    list
}

/// The same piece recorded at its own origin and placed with
/// `append_translated`.
fn appended_placed(
    lists: &Lists,
    [before, body, after]: [&[Call]; 3],
    offset: [f32; 2],
    clip: Option<Rect>,
) -> DrawList {
    let mut list = recorded(lists, before);
    list.append_translated(&recorded(lists, body), offset, clip);
    for call in after {
        call.draw(&mut list);
    }
    list
}

#[test]
fn a_placed_list_is_the_list_recorded_inside_translate_and_clip() {
    let lists = Lists::new();
    let mut rng = Rng(23);
    let mut shadows_dropped = 0;
    for round in 0..300 {
        let parts: Vec<Vec<Call>> = [3, 14, 3]
            .iter()
            .map(|&most| {
                let len = rng.below(most + 1) as usize;
                random_placed_calls(&mut rng, len)
            })
            .collect();
        let parts = [&parts[0][..], &parts[1][..], &parts[2][..]];
        let offset = random_offset(&mut rng);
        let clip = random_clip(&mut rng);
        let direct = recorded_placed(&lists, parts, offset, clip);
        let placed = appended_placed(&lists, parts, offset, clip);
        shadows_dropped += recorded(&lists, parts[1]).shadow_instance_count()
            - (placed.shadow_instance_count()
                - recorded(&lists, parts[0]).shadow_instance_count()
                - recorded(&lists, parts[2]).shadow_instance_count());
        assert_same(
            &direct,
            &placed,
            &format!("round {round}, offset {offset:?}, clip {clip:?}, {parts:?}"),
        );
    }
    // The clips did cut shadows, so dropping them was checked too.
    assert!(shadows_dropped > 0);
}

#[test]
fn a_shadow_the_clip_hides_is_dropped_and_the_soup_around_it_stays_one_run() {
    let lists = Lists::new();
    let call = |op| Call {
        op,
        at: [0.0, 0.0],
        n: 1,
    };
    let body = [
        call(Op::Triangle),
        call(Op::Shadow),
        call(Op::Triangle),
        call(Op::Chrome),
        call(Op::Triangle),
        call(Op::Shadow),
    ];
    assert_eq!(recorded(&lists, &body).shadow_instance_count(), 2);
    let clip = Some(Rect::new(1000.0, 1000.0, 50.0, 50.0));
    for offset in [[3.0, 4.0], [-7.0, 0.0]] {
        let parts = [&[][..], &body[..], &[][..]];
        let direct = recorded_placed(&lists, parts, offset, clip);
        let placed = appended_placed(&lists, parts, offset, clip);
        assert_eq!(placed.shadow_instance_count(), 0);
        assert_same(&direct, &placed, &format!("offset {offset:?}"));
    }
}

/// `span` cut in two, each kind of record at its middle.
fn halves(span: RecordSpan) -> [RecordSpan; 2] {
    let (s, e) = (span.start, span.end);
    let mid = |a: u32, b: u32| a + (b - a) / 2;
    let middle = RecordCounts {
        vertices: mid(s.vertices, e.vertices),
        indices: mid(s.indices, e.indices),
        analytic: mid(s.analytic, e.analytic),
        circles: mid(s.circles, e.circles),
        nine_slices: mid(s.nine_slices, e.nine_slices),
        icons: mid(s.icons, e.icons),
        icons_msdf: mid(s.icons_msdf, e.icons_msdf),
        texts: mid(s.texts, e.texts),
        selectable: mid(s.selectable, e.selectable),
        viewport_clips: mid(s.viewport_clips, e.viewport_clips),
    };
    [
        RecordSpan {
            start: s,
            end: middle,
        },
        RecordSpan {
            start: middle,
            end: e,
        },
    ]
}

/// A placed piece recorded again with the shape it had, and patched where it
/// was placed, is the list that placing the new piece makes, whether patched
/// whole or in two spans. A patch the placement cannot take is declined.
#[test]
fn a_piece_patched_where_it_was_placed_is_the_new_piece_placed() {
    let lists = Lists::new();
    let mut rng = Rng(41);
    let (mut patched, mut declined, mut reshaped) = (0, 0, 0);
    for round in 0..400 {
        let before = random_placed_calls(&mut rng, 3);
        let after = random_placed_calls(&mut rng, 3);
        let body = random_placed_calls(&mut rng, 12);
        // The same calls, some recoloured or resized, some moved by quarter
        // pixels: most keep the shape they had.
        let mut next = body.clone();
        for call in &mut next {
            if rng.below(2) == 0 {
                call.n = rng.below(1000);
            }
            if rng.below(3) == 0 {
                call.at[0] += (rng.below(80) as f32 - 40.0) * 0.25;
                call.at[1] += (rng.below(80) as f32 - 40.0) * 0.25;
            }
        }
        let offset = random_offset(&mut rng);
        let clip = random_clip(&mut rng);

        let old = recorded(&lists, &body);
        let mut list = recorded(&lists, &before);
        let mut log = PlacedLog::default();
        let piece = log.place(&mut list, &old, offset, clip);
        let placed = *log.placed(piece).expect("logged");
        for call in &after {
            call.draw(&mut list);
        }
        assert!(log.fits(piece, &old), "round {round}: a piece fits itself");
        let new = recorded(&lists, &next);
        if !log.fits(piece, &new) {
            reshaped += 1;
            continue;
        }
        let all = RecordSpan::all_of(&new);
        let spans = if round % 2 == 0 {
            vec![all]
        } else {
            halves(all).to_vec()
        };
        if !spans
            .iter()
            .all(|span| list.patch_translated(&placed, &new, span))
        {
            declined += 1;
            continue;
        }
        patched += 1;
        let fresh = appended_placed(&lists, [&before, &next, &after], offset, clip);
        assert_same(
            &fresh,
            &list,
            &format!("round {round}, offset {offset:?}, clip {clip:?}, {body:?} → {next:?}"),
        );
    }
    assert!(
        patched > 100 && declined > 0 && reshaped > 0,
        "{patched} patched, {declined} declined, {reshaped} of another shape"
    );
}

/// The target the oracles here compare frames on.
fn test_context() -> ContextKey {
    let target = Target::new((640, 480), 1.0, [0.0, 0.0]);
    ContextKey {
        grid: Lists::grid(&target),
        viewport: target.size,
        fonts: 0,
        resolution: 0,
    }
}

/// A list kept and patched in place, reporting its edits, against the
/// comparison it saves: every record the comparison finds changed is
/// reported changed at least as much, and a report is answered exactly when
/// it is relative to the list drawn last (renders skipped, lists recorded
/// again and patches declined included).
#[test]
fn reported_edits_cover_what_a_comparison_finds() {
    let lists = Lists::new();
    let context = test_context();
    let mut rng = Rng(59);
    let (mut answered, mut unanswered, mut looks, mut places) = (0, 0, 0u32, 0u32);
    for round in 0..60 {
        let mut bodies: Vec<Vec<Call>> = (0..6)
            .map(|_| {
                let len = 1 + rng.below(8) as usize;
                random_placed_calls(&mut rng, len)
            })
            .collect();
        let places_of: Vec<([f32; 2], Option<Rect>)> = (0..bodies.len())
            .map(|_| (random_offset(&mut rng), random_clip(&mut rng)))
            .collect();
        let mut list = lists.list();
        list.report_edits(true);
        let mut log = PlacedLog::default();
        let record = |list: &mut DrawList, log: &mut PlacedLog, bodies: &[Vec<Call>]| {
            list.clear();
            log.clear();
            for (body, &(offset, clip)) in bodies.iter().zip(&places_of) {
                log.place(list, &recorded(&lists, body), offset, clip);
            }
        };
        record(&mut list, &mut log, &bodies);
        let mut told = FrameSnapshot::default();
        let mut oracle = FrameSnapshot::default();
        // What the renderer can know: whether a list was drawn, whether the
        // list changed since (a patch or a recording), and whether the last
        // `begin_edits` was called on the list as drawn.
        let (mut drawn, mut changed, mut based) = (false, true, false);
        for step in 0..12 {
            let what = format!("round {round}, step {step}");
            match rng.below(8) {
                // Recorded again, from nothing.
                0 => {
                    record(&mut list, &mut log, &bodies);
                    (changed, based) = (true, false);
                }
                // Untouched.
                1 => {}
                // Some pieces recoloured, resized or moved.
                _ => {
                    for _ in 0..1 + rng.below(3) {
                        let k = rng.below(bodies.len() as u32) as usize;
                        let mut next = bodies[k].clone();
                        for call in &mut next {
                            if rng.below(2) == 0 {
                                call.n = rng.below(1000);
                            }
                            if rng.below(3) == 0 {
                                call.at[0] += (rng.below(80) as f32 - 40.0) * 0.25;
                            }
                        }
                        let new = recorded(&lists, &next);
                        if !log.fits(k as u32, &new) {
                            continue;
                        }
                        let placed = *log.placed(k as u32).expect("logged");
                        bodies[k] = next;
                        changed = true;
                        if !list.patch_translated(&placed, &new, &RecordSpan::all_of(&new)) {
                            // Partly patched: the owner records it again.
                            record(&mut list, &mut log, &bodies);
                            based = false;
                        }
                    }
                }
            }
            // Now and then a list made is not drawn: what is patched next
            // is relative to a list the renderer never saw.
            if rng.below(6) == 0 {
                list.begin_edits();
                based = !changed;
                continue;
            }
            let relative = drawn && (!changed || based);
            let diff = told.reported(&list, &context);
            let found = oracle.compare(&list, &context);
            oracle.keep(&list, &context);
            assert_eq!(diff.answered(), relative, "{what}: answered {diff:?}");
            assert_eq!(told.bytes(), 0, "{what}: a reported list is not kept");
            if relative {
                answered += 1;
                assert!(found.answered() || found.abandoned, "{what}: {found:?}");
                if !found.abandoned {
                    let (t, f) = (&told.changes, &oracle.changes);
                    let families: [(&str, &[Change], &[Change]); 6] = [
                        ("analytic", &t.analytic, &f.analytic),
                        ("circle", &t.circles, &f.circles),
                        ("icon", &t.icons, &f.icons),
                        ("nine-slice", &t.nine_slices, &f.nine_slices),
                        ("text", &t.texts, &f.texts),
                        ("soup", &t.soup, &f.soup),
                    ];
                    for (family, t, f) in families {
                        assert_eq!(t.len(), f.len(), "{what}: {family} counts");
                        for (i, (t, f)) in t.iter().zip(f).enumerate() {
                            assert!(t >= f, "{what}: {family} {i} told {t:?}, found {f:?}");
                        }
                    }
                    #[cfg(feature = "phosphor-icons")]
                    for (i, (t, f)) in t.icons_msdf.iter().zip(&f.icons_msdf).enumerate() {
                        assert!(t >= f, "{what}: MSDF icon {i} told {t:?}, found {f:?}");
                    }
                    assert!(t.tail >= f.tail, "{what}: soup tail");
                    looks += diff.look;
                    places += diff.place;
                }
            } else {
                unanswered += 1;
            }
            list.begin_edits();
            (drawn, changed, based) = (true, false, true);
        }
    }
    assert!(
        answered > 200 && unanswered > 50 && looks > 0 && places > 0,
        "{answered} answered, {unanswered} not, {looks} looks, {places} places"
    );
}

/// A log keeps each piece's shape apart: a piece fits only a list of the
/// shape it had, and a piece it does not hold fits nothing.
#[test]
fn a_placed_log_holds_each_piece_shape() {
    let lists = Lists::new();
    let call = |op, n| Call {
        op,
        at: [0.0, 0.0],
        n,
    };
    let a = recorded(&lists, &[call(Op::Chrome, 1), call(Op::Text, 2)]);
    let b = recorded(&lists, &[call(Op::Text, 3), call(Op::Chrome, 4)]);
    let c = recorded(&lists, &[call(Op::Chrome, 5)]);
    let mut list = lists.list();
    let mut log = PlacedLog::default();
    let pieces = [&a, &b, &c].map(|piece| log.place(&mut list, piece, [2.0, 3.0], None));
    assert_eq!(pieces, [0, 1, 2]);
    assert_eq!(log.len(), 3);
    // `a` and `b` hold the same counts, painted in another order.
    assert_eq!(a.record_counts(), b.record_counts());
    for i in 0..3 {
        for (j, other) in [&a, &b, &c].into_iter().enumerate() {
            assert_eq!(
                log.fits(i as u32, other),
                i == j,
                "piece {i} against list {j}"
            );
        }
    }
    let recoloured = recorded(&lists, &[call(Op::Text, 7), call(Op::Chrome, 8)]);
    assert!(log.fits(1, &recoloured));
    assert!(!log.fits(3, &c));
    assert_eq!(
        log.placed(2).map(|p| p.at),
        Some(b.record_counts().offset_by(a.record_counts()))
    );
    log.clear();
    assert!(log.is_empty() && !log.fits(0, &a));
}

/// A placement whose clip culled a record cannot be patched; nor can a list
/// of other counts.
#[test]
fn a_patch_declines_a_culled_placement_and_other_counts() {
    let lists = Lists::new();
    let call = |op, n| Call {
        op,
        at: [0.0, 0.0],
        n,
    };
    let body = [call(Op::Chrome, 1), call(Op::Shadow, 1)];
    let piece = recorded(&lists, &body);
    let mut list = lists.list();
    let hidden = Some(Rect::new(1000.0, 1000.0, 50.0, 50.0));
    let placed = list.append_translated(&piece, [0.0, 0.0], hidden);
    assert!(!placed.whole);
    assert!(!list.patch_translated(&placed, &piece, &RecordSpan::all_of(&piece)));

    let mut list = lists.list();
    let placed = list.append_translated(&piece, [3.0, 4.0], None);
    assert!(placed.whole);
    let longer = recorded(&lists, &[call(Op::Chrome, 1), call(Op::Chrome, 2)]);
    let more = recorded(
        &lists,
        &[
            call(Op::Chrome, 1),
            call(Op::Chrome, 2),
            call(Op::Circle, 1),
        ],
    );
    assert!(!list.patch_translated(&placed, &more, &RecordSpan::all_of(&more)));
    // Of one shape, the shadow now a chrome rect: an analytic record of
    // another kind is declined.
    assert!(!list.patch_translated(&placed, &longer, &RecordSpan::all_of(&longer)));
}

#[test]
fn placing_without_a_move_or_a_clip_is_appending() {
    let lists = Lists::new();
    let mut rng = Rng(5);
    for round in 0..50 {
        let before = random_placed_calls(&mut rng, 4);
        let body = random_placed_calls(&mut rng, 10);
        let mut appended = recorded(&lists, &before);
        appended.append(&recorded(&lists, &body));
        let mut placed = recorded(&lists, &before);
        placed.append_translated(&recorded(&lists, &body), [0.0, 0.0], None);
        assert_same(&appended, &placed, &format!("round {round}"));
    }
}

#[test]
fn placements_nest_as_recording_nests() {
    let lists = Lists::new();
    let mut rng = Rng(31);
    for round in 0..150 {
        let around = random_placed_calls(&mut rng, 6);
        let points = closed_points(&around);
        let (above, below) = around.split_at(points[rng.below(points.len() as u32) as usize]);
        let inner = random_placed_calls(&mut rng, 10);
        let [outer_offset, inner_offset] = [random_offset(&mut rng), random_offset(&mut rng)];
        let outer_clip = random_clip(&mut rng);
        let inner_clip = random_clip(&mut rng);

        let mut direct = lists.list();
        if let Some(clip) = outer_clip {
            direct.push_clip(clip);
        }
        direct.push_transform();
        direct.translate(outer_offset[0], outer_offset[1]);
        for call in above {
            call.draw(&mut direct);
        }
        if let Some(clip) = inner_clip {
            direct.push_clip(clip);
        }
        direct.push_transform();
        direct.translate(inner_offset[0], inner_offset[1]);
        for call in &inner {
            call.draw(&mut direct);
        }
        direct.pop_transform();
        if inner_clip.is_some() {
            direct.pop_clip();
        }
        for call in below {
            call.draw(&mut direct);
        }
        direct.pop_transform();
        if outer_clip.is_some() {
            direct.pop_clip();
        }

        let mut middle = recorded(&lists, above);
        middle.append_translated(&recorded(&lists, &inner), inner_offset, inner_clip);
        for call in below {
            call.draw(&mut middle);
        }
        let mut placed = lists.list();
        placed.append_translated(&middle, outer_offset, outer_clip);

        assert_same(&direct, &placed, &format!("round {round}"));
    }
}

#[test]
fn a_nested_clip_that_shows_nothing_is_the_same_empty_clip_when_placed() {
    let lists = Lists::new();
    let call = |op, at| Call { op, at, n: 2 };
    // A clip inside another, overlapping it, placed where the outer clip
    // misses both. Recording meets the empty clip at the inner one, placing
    // at the overlap of the two; an empty clip is stored at the origin, so
    // the two agree.
    let body = [
        call(Op::ClipPush, [0.0, 0.0]),
        call(Op::ClipPush, [-50.0, 10.0]),
        call(Op::Chrome, [5.0, 15.0]),
        call(Op::ClipPop, [0.0, 0.0]),
        call(Op::ClipPop, [0.0, 0.0]),
    ];
    let clip = Some(Rect::new(900.0, 900.0, 10.0, 10.0));
    let parts = [&[][..], &body[..], &[][..]];
    let direct = recorded_placed(&lists, parts, [4.0, 4.0], clip);
    let placed = appended_placed(&lists, parts, [4.0, 4.0], clip);
    let clip_of = |list: &DrawList| list.chrome_instances().next().unwrap().clip;
    let [recorded_clip, placed_clip] = [clip_of(&direct), clip_of(&placed)];
    assert_eq!(recorded_clip, [0.0; 4]);
    assert_eq!(placed_clip, [0.0; 4]);
    assert_same(&direct, &placed, "an empty nested clip");
}
