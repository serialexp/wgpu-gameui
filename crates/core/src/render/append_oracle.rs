//! `DrawList::append` against recording: a frame recorded in pieces, each
//! piece into its own list and the pieces appended in order, must be the list
//! the same calls make when recorded into one list, record for record. This
//! is what lets a retained piece of a frame be drawn again by appending it.

use crate::layout::Rect;
use crate::render::draw_box::Target;
use crate::render::draw_order::tests_support::Rng;
use crate::render::draw_plan::tests_support::Lists;
use crate::render::frame_diff::{ContextKey, FrameSnapshot};
use crate::text::TextBlock;
use crate::text_select::TextKey;
use crate::widgets::DrawList;

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
];

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
            Op::ClipPush => depth += 1,
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
    // Every other record (text blocks, icons, nine-slices), compared the way
    // the renderer compares frames: identical means the plan, the instances
    // and the draws come out the same.
    let target = Target::new((640, 480), 1.0, [0.0, 0.0]);
    let context = ContextKey {
        grid: Lists::grid(&target),
        viewport: target.size,
        fonts: 0,
        resolution: 0,
    };
    let mut snapshot = FrameSnapshot::default();
    snapshot.keep(direct, &context);
    let diff = snapshot.compare(pieces, &context);
    assert!(diff.identical(), "{what}: {diff:?}");
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
