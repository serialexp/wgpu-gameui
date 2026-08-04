//! A deliberately tiny gallery for the near-miss alignment lint.
//!
//! The full widget gallery is 5600px tall and 1000 nodes deep, which makes it a
//! terrible place to reason about *why* an alignment finding fired. This frame
//! holds six cases and nothing else — each one a `Group` (so it declares a real
//! container) holding four buttons, with the verdict it expects written into its
//! title. Two are meant to be caught; four are the false positives the lint used
//! to produce and must now stay silent.
//!
//! ```text
//! cargo test --test alignment_gallery                        # the verdicts
//! DISPLAY=:0 cargo test --test alignment_gallery -- --ignored --nocapture
//! ```
//! The `--ignored` run needs a GPU and writes `test_output/alignment_gallery.png`
//! plus `.debug.txt`, so the fixture can be *looked at* as well as asserted on.

use wgpu_gameui::debug::{DebugReport, Problem};
use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    Button, DrawContext, DrawList, FocusState, Group, InputState, StyleKey, StyleResolver, Theme,
};

const W: f32 = 720.0;
/// Gap between stacked case groups.
const GAP: f32 = 16.0;
/// Margin around the whole frame.
const MARGIN: f32 = 20.0;

/// One case: a titled group, the buttons in it, and the verdict it expects.
struct Case {
    title: &'static str,
    /// `(label, rect)` per button, in the group's **content-local** coordinates,
    /// so each case reads as a little layout of its own.
    items: Vec<(&'static str, Rect)>,
    expect: Expect,
}

enum Expect {
    /// No finding at all.
    Silent,
    /// Exactly one finding, on this edge, with this delta.
    One { edge: &'static str, delta: f32 },
}

fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(x, y, w, h)
}

fn cases() -> Vec<Case> {
    vec![
        // ---- Meant to be caught ----
        Case {
            title: "column: one stray",
            items: vec![
                ("Alpha", r(0.0, 0.0, 120.0, 28.0)),
                ("Bravo", r(0.0, 36.0, 120.0, 28.0)),
                // 3px right of the column everyone else agrees on.
                ("Charlie", r(3.0, 72.0, 120.0, 28.0)),
                ("Delta", r(0.0, 108.0, 120.0, 28.0)),
            ],
            expect: Expect::One {
                edge: "left",
                delta: 3.0,
            },
        },
        Case {
            title: "row: one dropped",
            items: vec![
                ("One", r(0.0, 0.0, 90.0, 28.0)),
                ("Two", r(98.0, 0.0, 90.0, 28.0)),
                // 4px below the row everyone else agrees on. Its bottom and
                // centre are equally wrong, but that is one bug, so one line.
                ("Three", r(196.0, 4.0, 90.0, 28.0)),
                ("Four", r(294.0, 0.0, 90.0, 28.0)),
            ],
            expect: Expect::One {
                edge: "top",
                delta: 4.0,
            },
        },
        // ---- Meant to stay silent ----
        Case {
            title: "column: clean",
            items: vec![
                ("Alpha", r(0.0, 0.0, 120.0, 28.0)),
                ("Bravo", r(0.0, 36.0, 120.0, 28.0)),
                ("Charlie", r(0.0, 72.0, 120.0, 28.0)),
                ("Delta", r(0.0, 108.0, 120.0, 28.0)),
            ],
            expect: Expect::Silent,
        },
        Case {
            title: "row: mixed widths",
            items: vec![
                // Widths differ by a few px, so their lefts, rights and centres
                // all land within near-miss range of one another. They are a
                // row: those numbers were never meant to match.
                ("One", r(0.0, 0.0, 90.0, 28.0)),
                ("Two", r(94.0, 0.0, 86.0, 28.0)),
                ("Three", r(184.0, 0.0, 92.0, 28.0)),
                ("Four", r(280.0, 0.0, 88.0, 28.0)),
            ],
            expect: Expect::Silent,
        },
        Case {
            title: "row: mixed heights, top-aligned",
            items: vec![
                // Tops agree exactly, so bottoms and centres cannot. Aligning to
                // one edge per axis is the whole idea of aligning.
                ("One", r(0.0, 0.0, 90.0, 24.0)),
                ("Two", r(98.0, 0.0, 90.0, 30.0)),
                ("Three", r(196.0, 0.0, 90.0, 26.0)),
                ("Four", r(294.0, 0.0, 90.0, 32.0)),
            ],
            expect: Expect::Silent,
        },
        Case {
            title: "column: mixed widths, left-aligned",
            items: vec![
                // The same idea on the other axis.
                ("Alpha", r(0.0, 0.0, 120.0, 28.0)),
                ("Bravo", r(0.0, 36.0, 126.0, 28.0)),
                ("Charlie", r(0.0, 72.0, 114.0, 28.0)),
                ("Delta", r(0.0, 108.0, 122.0, 28.0)),
            ],
            expect: Expect::Silent,
        },
    ]
}

impl Case {
    /// The content box the buttons need.
    fn content_size(&self) -> (f32, f32) {
        let (mut w, mut h) = (0.0f32, 0.0f32);
        for (_, r) in &self.items {
            w = w.max(r.right());
            h = h.max(r.bottom());
        }
        (w, h)
    }
}

/// Draw the fixture and return `(report, frame height)`.
///
/// Group rects are derived from what each case needs rather than hand-picked:
/// `Group` reserves a title header whose height is a theme detail, so a guessed
/// rect is exactly how a button ends up outside its own group — which is what
/// the first draft of this file did.
fn build(list: &mut DrawList) -> (DebugReport, f32) {
    let theme = Theme::default();
    let style = StyleResolver::new(&theme);
    let mut focus = FocusState::new();
    let input = InputState::default();

    // Probe the group chrome once to learn its insets.
    let probe = Rect::new(0.0, 0.0, 400.0, 400.0);
    let inner = Group::new("probe").content_rect(probe, &style);
    let (ins_x, ins_y) = (inner.x - probe.x, inner.y - probe.y);
    let (pad_x, pad_y) = (probe.right() - inner.right(), probe.bottom() - inner.bottom());

    let mut y = MARGIN;
    for case in cases() {
        let (cw, ch) = case.content_size();
        // The title sits in the header, so a group narrower than its own title
        // overflows itself — which the report duly reports, drowning the case.
        let title_w = list
            .measure_text(case.title, style.scalar(StyleKey::FontSize), None)
            .0;
        let w = cw.max(title_w) + ins_x + pad_x;
        let rect = Rect::new(MARGIN, y, w, ch + ins_y + pad_y);
        let content = Group::new(case.title).draw(rect, list, &style);
        for (label, r) in &case.items {
            let placed = Rect::new(content.x + r.x, content.y + r.y, r.width, r.height);
            let mut ctx = DrawContext::new(list, &mut focus, &theme, &input, W, y + rect.height);
            Button::new(*label).draw(placed, &mut ctx);
        }
        y = rect.bottom() + GAP;
    }

    let h = y - GAP + MARGIN;
    (DebugReport::measured(list, Rect::new(0.0, 0.0, W, h)), h)
}

/// Alignment findings, as `(container name, node name, edge, delta)`.
fn findings(report: &DebugReport) -> Vec<(String, String, &'static str, f32)> {
    report
        .problems
        .iter()
        .filter_map(|p| match p {
            Problem::NearMissAlignment {
                node, edge, delta, ..
            } => {
                let n = &report.nodes[*node];
                let group = n
                    .parent
                    .map(|p| report.nodes[p].name.clone())
                    .unwrap_or_else(|| "<root>".into());
                Some((group, n.name.clone(), *edge, *delta))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn every_case_gets_the_verdict_it_advertises() {
    let mut list = DrawList::new();
    let (report, _) = build(&mut list);
    let found = findings(&report);

    for case in cases() {
        let mine: Vec<_> = found
            .iter()
            .filter(|(group, ..)| group.contains(case.title))
            .collect();
        match case.expect {
            Expect::Silent => assert!(
                mine.is_empty(),
                "\"{}\" should be silent, got {mine:?}\n\n{}",
                case.title,
                report.to_text()
            ),
            Expect::One { edge, delta } => {
                assert_eq!(
                    mine.len(),
                    1,
                    "\"{}\" should yield exactly one finding, got {mine:?}\n\n{}",
                    case.title,
                    report.to_text()
                );
                let (_, _, got_edge, got_delta) = mine[0];
                assert_eq!(*got_edge, edge, "\"{}\" edge", case.title);
                assert!(
                    (got_delta - delta).abs() < 0.01,
                    "\"{}\" delta: want {delta}, got {got_delta}",
                    case.title
                );
            }
        }
    }

    let expected = cases()
        .iter()
        .filter(|c| matches!(c.expect, Expect::One { .. }))
        .count();
    assert_eq!(
        found.len(),
        expected,
        "unexpected extra findings: {found:?}\n\n{}",
        report.to_text()
    );
}

/// Every case group must be intact, or a "silent" verdict could be silence about
/// a button that fell out of the group rather than a lint working correctly.
#[test]
fn each_group_actually_contains_its_four_buttons() {
    let mut list = DrawList::new();
    let (report, _) = build(&mut list);
    for case in cases() {
        let group = report
            .nodes
            .iter()
            .find(|n| n.name == format!("Group {:?}", case.title))
            .unwrap_or_else(|| panic!("no group named {:?}", case.title));
        let buttons = report
            .nodes
            .iter()
            .filter(|n| n.parent == Some(group.id) && n.name.starts_with("Button"))
            .count();
        assert_eq!(
            buttons,
            case.items.len(),
            "\"{}\" lost a button out of its group\n\n{}",
            case.title,
            report.to_text()
        );
    }
}

/// The same stray column, drawn outside any scope. Alignment is a claim about
/// intent — that one piece of layout code placed these together — and nothing in
/// an unscoped draw says so, so the stray goes unreported.
#[test]
fn an_unscoped_column_is_not_analysed() {
    let theme = Theme::default();
    let mut focus = FocusState::new();
    let input = InputState::default();
    let mut list = DrawList::new();

    for (i, x) in [20.0f32, 20.0, 23.0, 20.0].into_iter().enumerate() {
        let rect = Rect::new(x, 20.0 + i as f32 * 36.0, 120.0, 28.0);
        let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, W, 300.0);
        Button::new("Item").draw(rect, &mut ctx);
    }

    let report = DebugReport::measured(&mut list, Rect::new(0.0, 0.0, W, 300.0));
    assert!(
        findings(&report).is_empty(),
        "no declared region means no alignment claim\n\n{}",
        report.to_text()
    );
}

/// Render the fixture so it can be eyeballed alongside the dump.
#[test]
#[ignore = "needs a GPU adapter"]
fn render_alignment_gallery() {
    use wgpu_gameui::{HeadlessGpu, write_png};

    let Some(mut gpu) = HeadlessGpu::new() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };
    let mut list = gpu.draw_list();
    let (report, h) = build(&mut list);

    std::fs::create_dir_all("test_output").unwrap();
    std::fs::write("test_output/alignment_gallery.debug.txt", report.to_text())
        .expect("write dump");
    eprintln!(
        "wrote test_output/alignment_gallery.debug.txt — {} nodes, {} problems",
        report.nodes.len(),
        report.problems.len()
    );
    for (group, node, edge, delta) in findings(&report) {
        eprintln!("  {group} / {node}: {edge} off by {delta:+.2}px");
    }

    let size = (W as u32, h.ceil() as u32);
    let pixels = gpu.capture_on(
        &list,
        size,
        wgpu::Color {
            r: 0.05,
            g: 0.06,
            b: 0.08,
            a: 1.0,
        },
    );
    write_png("test_output/alignment_gallery.png", &pixels, size).expect("write png");
    eprintln!(
        "wrote test_output/alignment_gallery.png ({}x{})",
        size.0, size.1
    );
}
