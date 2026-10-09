//! The scatter plot: readings placed inside the well's insets on a nice
//! axis with headroom, x over 0..1 or the readings' span, flagged readings
//! amber and on top, the reference, hover within reach, the empty state
//! under the minimum, the x labels, and the tooltip above the reading.
use super::*;
use crate::Theme;

const TEAL: [f32; 4] = [0.0, 0.5, 0.5, 1.0];
const WIDTH: f32 = 300.0;
const VIEWPORT: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 800.0,
    height: 600.0,
};

fn point(x: f64, y: f64) -> ScatterPoint<'static> {
    ScatterPoint {
        x,
        y,
        flagged: false,
        tip: None,
    }
}

fn format(value: f64) -> String {
    format!("{value}")
}

fn away() -> InputState {
    InputState {
        mouse_x: -100.0,
        mouse_y: -100.0,
        ..Default::default()
    }
}

fn at((x, y): (f32, f32)) -> InputState {
    InputState {
        mouse_x: x,
        mouse_y: y,
        ..Default::default()
    }
}

/// Draw `plot` at the origin, [`WIDTH`] wide.
fn paint(
    plot: &ScatterPlot<'_>,
    s: &StyleResolver,
    input: &InputState,
) -> (DrawList, ScatterPlotOutput) {
    let mut list = DrawList::new();
    let out = plot.draw(0.0, 0.0, WIDTH, &mut list, s, input);
    (list, out)
}

fn layout_of(plot: &ScatterPlot<'_>, s: &StyleResolver) -> Layout {
    plot.layout(Rect::new(0.0, 0.0, WIDTH, plot.height), s)
}

/// The filled dots painted in `color`: `[x, y, radius]`.
fn dots_of(list: &DrawList, color: [f32; 4]) -> Vec<[f32; 3]> {
    list.circle_instances
        .iter()
        .filter(|c| c.color == color && c.center[3] <= 0.0)
        .map(|c| [c.center[0], c.center[1], c.center[2]])
        .collect()
}

fn shows(list: &DrawList, text: &str) -> bool {
    list.texts.iter().any(|t| t.content == text)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn readings_sit_inside_the_insets_on_an_axis_with_headroom() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let points = [point(0.0, 0.0), point(0.5, 50.0), point(1.0, 100.0)];
    let plot = ScatterPlot::new(&points, &format).color(TEAL);
    let layout = layout_of(&plot, &s);
    assert_eq!(layout.max, nice_max(108.0));
    let (list, out) = paint(&plot, &s, &away());
    assert_eq!(out.height, SCATTER_HEIGHT);
    let dots = dots_of(&list, TEAL);
    assert_eq!(dots.len(), 3);
    let inner = layout.inner;
    let (left, right) = (inner.x + PAD + Y_LABELS, inner.right() - PAD - Y_LABELS);
    assert!(close(dots[0][0], left) && close(dots[2][0], right));
    assert!(close(dots[1][0], (left + right) * 0.5));
    assert!(close(dots[0][1], inner.bottom() - PAD), "zero on the floor");
    assert!(dots[2][1] > inner.y + PAD, "headroom over the highest");
    assert!(dots.iter().all(|d| d[2] == DOT));
    // The axis: its maximum and zero, on plates.
    assert!(shows(&list, &format(layout.max)) && shows(&list, "0"));
}

#[test]
fn x_runs_over_the_readings_span_when_they_leave_the_unit_range() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let points = [
        point(1_000.0, 1.0),
        point(3_000.0, 2.0),
        point(2_000.0, 3.0),
    ];
    let plot = ScatterPlot::new(&points, &format);
    let layout = layout_of(&plot, &s);
    assert_eq!((layout.x0, layout.x1), (1_000.0, 3_000.0));
    let inner = layout.inner;
    assert!(close(
        layout.at(2_000.0, 0.0).0,
        inner.x + inner.width * 0.5
    ));
    // One x: the middle.
    let one = [point(5.0, 1.0); 3];
    let layout = layout_of(&ScatterPlot::new(&one, &format), &s);
    assert!(close(layout.at(5.0, 0.0).0, inner.x + inner.width * 0.5));
}

#[test]
fn flagged_readings_are_amber_and_drawn_over_the_rest() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let mut points = [point(0.2, 1.0), point(0.4, 2.0), point(0.6, 3.0)];
    points[0].flagged = true;
    let plot = ScatterPlot::new(&points, &format).color(TEAL);
    let (list, _) = paint(&plot, &s, &away());
    let amber = s.color(StyleKey::WarnMeta);
    let order: Vec<[f32; 4]> = list
        .circle_instances
        .iter()
        .filter(|c| c.center[3] <= 0.0)
        .map(|c| c.color)
        .collect();
    assert_eq!(order, vec![TEAL, TEAL, amber], "the flagged one last");
}

#[test]
fn the_reference_runs_across_with_its_label_on_the_right() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let points = [point(0.2, 10.0), point(0.4, 20.0), point(0.6, 30.0)];
    let plot = ScatterPlot::new(&points, &format).reference(200.0);
    let layout = layout_of(&plot, &s);
    assert_eq!(layout.max, nice_max(216.0), "the reference raises the axis");
    let (list, _) = paint(&plot, &s, &away());
    let label = list
        .texts
        .iter()
        .find(|t| t.content == "median 200")
        .expect("labelled");
    assert!(label.x > WIDTH * 0.5);
}

#[test]
fn hover_takes_the_nearest_reading_within_reach() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let points = [point(0.2, 10.0), point(0.25, 10.0), point(0.8, 30.0)];
    let plot = ScatterPlot::new(&points, &format).color(TEAL);
    let layout = layout_of(&plot, &s);
    let (x1, y1) = layout.at(0.25, 10.0);
    let (list, out) = paint(&plot, &s, &at((x1 + 2.0, y1 + 1.0)));
    assert_eq!(out.hovered, Some(1));
    let tip = out.tooltip.unwrap();
    assert!(tip.rightward && close(tip.at.0, x1));
    // Drawn larger, ringed, and last.
    let lit = dots_of(&list, TEAL);
    assert_eq!(lit.last().unwrap()[2], HOVER_DOT);
    let rings = list
        .circle_instances
        .iter()
        .filter(|c| c.color == s.ink(Ink::Value));
    assert_eq!(rings.count(), 1);

    let (x2, y2) = layout.at(0.8, 30.0);
    let (_, out) = paint(&plot, &s, &at((x2, y2)));
    assert!(!out.tooltip.unwrap().rightward);
    assert_eq!(
        paint(&plot, &s, &at((x2, y2 + 20.0))).1.hovered,
        None,
        "out of reach"
    );

    // Lit from a table: the dot, but no tooltip.
    let (list, out) = paint(&plot.hovered(Some(2)), &s, &away());
    assert_eq!((out.hovered, out.tooltip), (None, None));
    assert_eq!(dots_of(&list, TEAL).last().unwrap()[2], HOVER_DOT);
}

#[test]
fn too_few_readings_say_so_over_faint_dots_and_dont_hover() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let points = [point(0.2, 10.0), point(0.6, 20.0)];
    let plot = ScatterPlot::new(&points, &format).color(TEAL);
    let layout = layout_of(&plot, &s);
    let (list, out) = paint(&plot, &s, &at(layout.at(0.2, 10.0)));
    assert_eq!(out.hovered, None);
    assert!(shows(&list, "NOT ENOUGH READINGS YET"));
    assert!(shows(&list, "2 of 3 readings"));
    let faint = [TEAL[0], TEAL[1], TEAL[2], TOO_FEW];
    assert_eq!(dots_of(&list, faint).len(), 2);
    assert!(!shows(&list, "0"), "no axis");

    let plot = plot
        .empty_labels("Calibrating", Some("one more run"))
        .min_points(4);
    let (list, _) = paint(&plot, &s, &away());
    assert!(shows(&list, "CALIBRATING") && shows(&list, "one more run"));

    let none: [ScatterPoint<'_>; 0] = [];
    let (list, _) = paint(&ScatterPlot::new(&none, &format), &s, &away());
    assert!(shows(&list, "0 of 3 readings"));
}

#[test]
fn x_labels_spread_from_edge_to_edge_under_the_well() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let points = [point(0.2, 10.0); 3];
    let labels = ["14 d ago", "7 d", "now"];
    let plot = ScatterPlot::new(&points, &format).x_labels(&labels);
    let (mut list, out) = paint(&plot, &s, &away());
    assert_eq!(out.height, SCATTER_HEIGHT + X_LABEL_GAP + X_LABEL_ROW);
    assert_eq!(out.height, plot.measure_height());
    let find = |text: &str| {
        list.texts
            .iter()
            .find(|t| t.content == text)
            .cloned()
            .unwrap()
    };
    let (first, middle, last) = (find("14 d ago"), find("7 d"), find("now"));
    assert!(close(first.x, X_LABEL_PAD));
    let (w, _) = list.measure_block(&last);
    assert!(close(last.x + w, WIDTH - X_LABEL_PAD));
    assert!(first.y > SCATTER_HEIGHT && first.x < middle.x && middle.x < last.x);
}

#[test]
fn the_tooltip_sits_above_the_reading_with_its_flag_and_tip() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let mut points = [point(0.2, 10.0), point(0.5, 20.0), point(0.8, 30.0)];
    points[1].flagged = true;
    points[1].tip = Some("Tue 14:02");
    let plot = ScatterPlot::new(&points, &format);
    let layout = layout_of(&plot, &s);
    let dot = layout.at(0.5, 20.0);
    let (_, out) = paint(&plot, &s, &at(dot));
    let tip = out.tooltip.unwrap();
    let mut list = DrawList::new();
    let rect = plot.draw_tooltip(&tip, VIEWPORT, &mut list, &s).unwrap();
    assert!(
        rect.bottom() <= dot.1 - TIP_GAP + 0.5,
        "above the dot: {rect:?}"
    );
    for text in ["20", "EXCLUDED FROM FIT", "Tue 14:02"] {
        assert!(shows(&list, text), "{text}");
    }
    // Never above the well's top: a reading near it pushes the tooltip down
    // to start there, then the viewport keeps it in.
    let high = ScatterTooltip {
        at: (100.0, 5.0),
        ..tip
    };
    let rect = plot
        .draw_tooltip(&high, VIEWPORT, &mut DrawList::new(), &s)
        .unwrap();
    assert_eq!(rect.y, 0.0);
    let stale = ScatterTooltip { index: 7, ..tip };
    assert_eq!(
        plot.draw_tooltip(&stale, VIEWPORT, &mut DrawList::new(), &s),
        None
    );
}

#[test]
fn readings_that_arent_finite_are_left_out() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let points = [
        point(0.2, 10.0),
        point(f64::NAN, 1.0),
        point(0.5, f64::INFINITY),
        point(0.6, 3.0),
    ];
    let plot = ScatterPlot::new(&points, &format).color(TEAL);
    assert_eq!(layout_of(&plot, &s).max, nice_max(10.0 * HEADROOM));
    let (list, _) = paint(&plot, &s, &away());
    assert_eq!(dots_of(&list, TEAL).len(), 2);
    for width in [0.0, 10.0] {
        plot.draw(0.0, 0.0, width, &mut DrawList::new(), &s, &at((5.0, 50.0)));
    }
}
