//! The line chart: points spread between the plot's insets, runs broken at
//! gaps with a lone point as a dot, the area's fade, estimated dashes, hover
//! that snaps to the nearest x, the reduction past two points a pixel, the
//! running point, the legend and its fade, the empty captions, the time
//! axis, the reference line, and the tooltip.
use super::*;
use crate::Theme;
use crate::widgets::chart::{HALO_WIDTH, HOVER_DOT, LINE_WIDTH};

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const WIDTH: f32 = 400.0;
const VIEWPORT: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 800.0,
    height: 600.0,
};

fn line<'a>(name: &'a str, color: [f32; 4], values: &'a [Option<f64>]) -> LineSeries<'a> {
    LineSeries {
        name,
        color,
        values,
        dots: false,
        area: false,
        estimated: false,
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

fn at(x: f32, y: f32) -> InputState {
    InputState {
        mouse_x: x,
        mouse_y: y,
        ..Default::default()
    }
}

/// Draw `chart` at the origin, [`WIDTH`] wide.
fn paint(
    chart: &LineChart<'_>,
    s: &StyleResolver,
    input: &InputState,
) -> (DrawList, LineChartOutput) {
    let mut list = DrawList::new();
    let out = chart.draw(0.0, 0.0, WIDTH, &mut list, s, input);
    (list, out)
}

fn layout_of(chart: &LineChart<'_>, s: &StyleResolver) -> Layout {
    chart.layout(0.0, 0.0, WIDTH, &mut DrawList::new(), s)
}

/// The segments stroked in `color` (`[ax, ay, bx, by]`), and their half
/// widths.
fn segments_of(list: &DrawList, color: [f32; 4]) -> Vec<([f32; 4], f32)> {
    list.segment_instances()
        .filter(|g| g.color == color)
        .map(|g| (g.ends, g.translation[3]))
        .collect()
}

/// The circles painted in `color`: `[x, y, radius, thickness]`.
fn circles_of(list: &DrawList, color: [f32; 4]) -> Vec<[f32; 4]> {
    list.circle_instances
        .iter()
        .filter(|c| c.color == color)
        .map(|c| c.center)
        .collect()
}

fn shows(list: &DrawList, text: &str) -> bool {
    list.texts.iter().any(|t| t.content == text)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn points_spread_between_the_insets_and_rise_with_their_value() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let values = [Some(0.0), Some(15.0), Some(30.0)];
    let series = [line("Input", RED, &values)];
    let chart = LineChart::new(&series, &format).legend(false);
    let layout = layout_of(&chart, &s);
    assert_eq!(layout.max, 30.0);
    let (list, out) = paint(&chart, &s, &away());
    let lines: Vec<_> = segments_of(&list, RED)
        .into_iter()
        .filter(|(_, half)| close(*half, LINE_WIDTH * 0.5))
        .map(|(ends, _)| ends)
        .collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    let (left, right) = (layout.left + INSET, WIDTH - INSET);
    let (base, top) = (PAD + CHART_HEIGHT, PAD);
    assert!(close(lines[0][0], left) && close(lines[0][1], base));
    assert!(close(lines[0][2], (left + right) * 0.5));
    assert!(close(lines[0][3], (base + top) * 0.5));
    assert!(close(lines[1][2], right) && close(lines[1][3], top));
    // A halo under every segment, the line's own width and more.
    let halos = segments_of(&list, HALO);
    assert_eq!(halos.len(), 2);
    assert!(halos.iter().all(|(_, half)| close(*half, HALO_WIDTH * 0.5)));
    assert_eq!(out.height, PAD + CHART_HEIGHT + X_LABELS);
    assert_eq!(out.hovered, None);
}

#[test]
fn a_gap_breaks_the_line_and_a_lone_point_is_a_dot() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let values = [
        Some(1.0),
        Some(2.0),
        None,
        Some(3.0),
        None,
        Some(1.0),
        Some(f64::NAN),
        Some(2.0),
    ];
    let series = [line("Input", RED, &values)];
    let chart = LineChart::new(&series, &format).legend(false);
    let (list, _) = paint(&chart, &s, &away());
    let lines: Vec<_> = segments_of(&list, RED)
        .into_iter()
        .filter(|(_, half)| close(*half, LINE_WIDTH * 0.5))
        .collect();
    assert_eq!(lines.len(), 1, "only the first run has two points");
    // The lone 3, 1 and 2 (NaN counts as missing): small dots.
    let dots = circles_of(&list, RED);
    assert_eq!(dots.len(), 3, "{dots:?}");
    assert!(dots.iter().all(|c| c[2] == LONE_DOT && c[3] <= 0.0));
}

#[test]
fn the_area_fades_from_the_top_to_nothing_at_the_baseline() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let values = [Some(30.0), Some(30.0)];
    let mut series = [line("Input", RED, &values)];
    series[0].area = true;
    let chart = LineChart::new(&series, &format).legend(false);
    let (list, _) = paint(&chart, &s, &away());
    assert_eq!(list.indices.len(), 6, "two triangles a segment");
    let base = PAD + CHART_HEIGHT;
    for v in &list.vertices {
        let expected = if close(v.position[1], base) {
            0.0
        } else {
            assert!(close(v.position[1], PAD), "{:?}", v.position);
            AREA_TOP
        };
        assert!(close(v.color[3], expected), "{v:?}");
    }

    // Estimated: half as strong, and dashed.
    series[0].estimated = true;
    let chart = LineChart::new(&series, &format).legend(false);
    let (list, _) = paint(&chart, &s, &away());
    let strongest = list.vertices.iter().map(|v| v.color[3]).fold(0.0, f32::max);
    assert!(close(strongest, AREA_TOP_ESTIMATED));
    let dashes: Vec<_> = list.segment_instances().map(|g| g.dash[0]).collect();
    assert!(
        !dashes.is_empty() && dashes.iter().all(|&d| d == ESTIMATED_DASH.0),
        "{dashes:?}"
    );
}

#[test]
fn hover_snaps_to_the_nearest_x_and_places_the_tooltip_on_the_roomier_side() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let x = [0.0, 1.0, 2.0, 10.0];
    let values = [Some(1.0), Some(2.0), Some(3.0), Some(4.0)];
    let series = [line("Input", RED, &values)];
    let chart = LineChart::new(&series, &format).x(&x).legend(false);
    let layout = layout_of(&chart, &s);
    let mid_y = PAD + CHART_HEIGHT * 0.5;

    // Between 2 and 10, nearer 2: the third point, though it is a quarter
    // of the way across in index.
    let (list, out) = paint(&chart, &s, &at(layout.px(5.0), mid_y));
    assert_eq!(out.hovered, Some(2));
    let tip = out.tooltip.expect("a tooltip while over the plot");
    assert!(tip.right_of_guide);
    assert!(close(tip.guide, layout.px(2.0)));
    assert_eq!(rects_count(&list, GUIDE), 1);
    // The hovered dot, ringed.
    assert_eq!(circles_of(&list, s.ink(Ink::Value)).len(), 1);
    assert!(circles_of(&list, RED).iter().any(|c| c[2] == HOVER_DOT));

    let (_, out) = paint(&chart, &s, &at(layout.px(9.0), mid_y));
    assert_eq!(out.hovered, Some(3));
    assert!(!out.tooltip.unwrap().right_of_guide);

    // Over the gutter or below the labels: nothing.
    assert_eq!(paint(&chart, &s, &at(1.0, mid_y)).1.hovered, None);
    let below = PAD + CHART_HEIGHT + X_LABELS + 1.0;
    assert_eq!(
        paint(&chart, &s, &at(layout.px(5.0), below)).1.hovered,
        None
    );

    // Lit from a table: the dot, but no guide or tooltip.
    let chart = chart.hovered(Some(1));
    let (list, out) = paint(&chart, &s, &away());
    assert_eq!((out.hovered, out.tooltip), (None, None));
    assert_eq!(rects_count(&list, GUIDE), 0);
    assert_eq!(circles_of(&list, s.ink(Ink::Value)).len(), 1);
}

fn rects_count(list: &DrawList, color: [f32; 4]) -> usize {
    list.chrome_instances().filter(|c| c.bg == color).count()
}

/// The segments and circles of `n` points of a noisy series, with dots.
fn cost_of(n: usize) -> crate::PrimCounts {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let values: Vec<_> = (0..n).map(|i| Some(((i * 7919) % 100) as f64)).collect();
    let mut series = [line("Input", RED, &values)];
    series[0].dots = true;
    let chart = LineChart::new(&series, &format).legend(false);
    paint(&chart, &s, &away()).0.prim_counts()
}

#[test]
fn past_two_points_a_pixel_the_cost_stops_growing() {
    let (many, most) = (cost_of(5_000), cost_of(50_000));
    assert_eq!(many.segment_instances, most.segment_instances);
    assert_eq!(many.texts, most.texts);
    // Each column's lowest and highest: at most two points a column, so at
    // most four segments (halo and line) a column.
    assert!(most.segment_instances <= 4 * WIDTH as usize, "{most:?}");
    assert_eq!(most.circle_instances, 0, "too dense for dots");
}

#[test]
fn the_reduction_keeps_each_columns_extremes_and_its_gaps() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let mut values = vec![Some(10.0); 20_000];
    values[7_001] = Some(100.0);
    values[13_003] = Some(0.0);
    values[15_000] = None;
    let series = [line("Input", RED, &values)];
    let chart = LineChart::new(&series, &format).legend(false);
    let layout = layout_of(&chart, &s);
    let mut points = Vec::new();
    chart.reduce(&series[0], &layout, &mut points);
    assert!(
        points.len() <= 3 * layout.span.ceil() as usize + 3,
        "{}",
        points.len()
    );
    let has = |v: Option<f64>| points.iter().any(|&(_, p)| p == v);
    assert!(
        has(Some(100.0)) && has(Some(0.0)),
        "the spike and the dip survive"
    );
    assert!(has(None), "the gap still breaks the line");
    assert!(points.windows(2).all(|w| w[0].0 <= w[1].0), "in x order");
}

#[test]
fn dots_show_while_there_is_room_for_them() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let values = [Some(1.0), Some(2.0), None, Some(3.0)];
    let mut series = [line("Input", RED, &values)];
    series[0].dots = true;
    let chart = LineChart::new(&series, &format).legend(false);
    let (list, _) = paint(&chart, &s, &away());
    let dots = circles_of(&list, RED);
    // Three dots, and the lone 3's small one under its own.
    assert_eq!(dots.iter().filter(|c| c[2] == DOT).count(), 3, "{dots:?}");
    assert_eq!(circles_of(&list, DOT_RING).len(), 4);
}

#[test]
fn the_running_point_is_hollow_and_says_now_clearing_the_labels_beside_it() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    // Eleven points ~37px apart, labelled every third from the newest:
    // 1, 4, 7 and 10.
    let values: Vec<_> = (0..11).map(|i| Some(f64::from(i))).collect();
    let names: Vec<String> = (0..11).map(|i| format!("p{i}")).collect();
    let labels: Vec<&str> = names.iter().map(String::as_str).collect();
    let series = [line("Input", RED, &values)];
    let chart = LineChart::new(&series, &format)
        .labels(&labels)
        .current(9)
        .legend(false);
    let (list, _) = paint(&chart, &s, &away());
    assert!(shows(&list, "now"));
    assert!(!shows(&list, "p9"), "now names the running point");
    assert!(!shows(&list, "p10"), "too near now");
    assert!(shows(&list, "p7") && shows(&list, "p4") && shows(&list, "p1"));
    let backgrounds = circles_of(&list, s.color(StyleKey::Background));
    assert_eq!(backgrounds.len(), 1);
    assert_eq!(backgrounds[0][2], CURRENT_DOT);
    assert!(
        circles_of(&list, RED)
            .iter()
            .any(|c| c[2] == CURRENT_DOT && c[3] == CURRENT_STROKE)
    );

    // Hovered, the running point takes the hovered dot instead.
    let (list, _) = paint(&chart.hovered(Some(9)), &s, &away());
    assert!(circles_of(&list, s.color(StyleKey::Background)).is_empty());

    // The legend says what the hollow dot is.
    let (list, _) = paint(&chart.legend(true), &s, &away());
    assert!(shows(&list, CURRENT_LABEL));
}

#[test]
fn the_legend_lists_the_series_and_hovering_one_fades_the_rest() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let values = [Some(1.0), Some(2.0)];
    let mut series = [line("Input", RED, &values), line("Thinking", BLUE, &values)];
    series[1].estimated = true;
    let chart = LineChart::new(&series, &format)
        .reference(1.5)
        .reference_label("cap");
    let (list, out) = paint(&chart, &s, &away());
    assert!(shows(&list, "Input") && shows(&list, "Thinking") && shows(&list, "EST."));
    assert!(shows(&list, "cap"), "the reference has an entry");
    assert!(shows(&list, "cap 1.5"), "and a label on the plot");
    assert!(out.height > PAD + CHART_HEIGHT + X_LABELS);
    assert_eq!(
        out.height,
        chart.measure_height(WIDTH, &mut DrawList::new(), &s)
    );

    // Find "Thinking"'s entry and hover it.
    let entry = list.texts.iter().find(|t| t.content == "Thinking").unwrap();
    let (_, out) = paint(&chart, &s, &at(entry.x + 2.0, entry.y + 4.0));
    assert_eq!(out.hovered_series, Some(1));
    let (list, _) = paint(&chart.hovered_series(Some(1)), &s, &away());
    let mut faded = RED;
    faded[3] = SERIES_FADED;
    assert!(!segments_of(&list, faded).is_empty(), "Input fades");
    assert!(!segments_of(&list, BLUE).is_empty(), "Thinking doesn't");
}

#[test]
fn no_values_or_only_zeros_say_so() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let none = [None, None];
    let series = [line("Input", RED, &none)];
    let chart = LineChart::new(&series, &format).legend(false);
    let (list, out) = paint(&chart, &s, &at(200.0, 100.0));
    assert!(shows(&list, "NO DATA IN THIS RANGE"));
    assert_eq!(out.hovered, None);
    assert_eq!(
        list.segment_instances().filter(|g| g.color == RED).count(),
        0
    );

    let zeros = [Some(0.0), Some(0.0)];
    let series = [line("Input", RED, &zeros)];
    let (list, _) = paint(&LineChart::new(&series, &format), &s, &away());
    assert!(shows(&list, "ALL VALUES ARE ZERO"));
    assert!(
        !segments_of(&list, RED).is_empty(),
        "the flat line still shows"
    );

    let empty: [LineSeries<'_>; 0] = [];
    let (list, _) = paint(&LineChart::new(&empty, &format), &s, &away());
    assert!(shows(&list, "NO DATA IN THIS RANGE"));
}

#[test]
fn a_time_axis_labels_whole_local_hours() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    // 2026-10-08 00:00 UTC, every 15 minutes for a day.
    let start = 1_791_417_600_000.0;
    let x: Vec<f64> = (0..=96).map(|i| start + f64::from(i) * 900_000.0).collect();
    let values: Vec<_> = (0..=96).map(|i| Some(f64::from(i))).collect();
    let series = [line("Input", RED, &values)];
    let chart = LineChart::new(&series, &format).x(&x).time(0).legend(false);
    let (list, out) = paint(&chart, &s, &at(WIDTH - INSET, 50.0));
    assert!(shows(&list, "Oct 8"), "midnight is named by its date");
    assert!(list.texts.iter().any(|t| t.content.ends_with(":00")));
    let tip = out.tooltip.unwrap();
    assert_eq!(tip.index, 96);
    let mut over = DrawList::new();
    chart.draw_tooltip(&tip, VIEWPORT, &mut over, &s).unwrap();
    assert!(
        shows(&over, "Fri, Oct 9, 00:00"),
        "{:?}",
        over.texts.iter().map(|t| &t.content).collect::<Vec<_>>()
    );
}

#[test]
fn the_tooltip_lists_the_series_last_first_with_a_dash_for_a_gap() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let input = [Some(1.0), Some(2.0)];
    let thinking = [Some(3.0), None];
    let mut series = [
        line("Input", RED, &input),
        line("Thinking", BLUE, &thinking),
    ];
    series[0].estimated = true;
    let long = ["first", "second"];
    let chart = LineChart::new(&series, &format)
        .long_labels(&long)
        .current(1)
        .legend(false);
    let layout = layout_of(&chart, &s);
    let (_, out) = paint(&chart, &s, &at(layout.px(1.0), 50.0));
    let tip = out.tooltip.unwrap();
    assert_eq!(tip.index, 1);
    let mut list = DrawList::new();
    let rect = chart.draw_tooltip(&tip, VIEWPORT, &mut list, &s).unwrap();
    assert!(rect.width >= TIP_MIN_WIDTH);
    assert!(
        rect.right() <= tip.guide - TIP_GAP + 0.5,
        "left of the guide past the middle"
    );
    let texts: Vec<&str> = list.texts.iter().map(|t| t.content.as_str()).collect();
    assert_eq!(texts[0], "second");
    assert!(texts.contains(&"RUNNING"));
    let thinking_at = texts.iter().position(|&t| t == "Thinking").unwrap();
    let input_at = texts.iter().position(|&t| t == "Input").unwrap();
    assert!(thinking_at < input_at, "the last series first: {texts:?}");
    assert!(texts.contains(&"—") && texts.contains(&"2") && texts.contains(&" (est.)"));

    // A point out of range has no tooltip.
    let stale = LineTooltip { index: 9, ..tip };
    assert_eq!(
        chart.draw_tooltip(&stale, VIEWPORT, &mut DrawList::new(), &s),
        None
    );
}

#[test]
fn a_reference_above_the_data_raises_the_axis() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let values = [Some(10.0), Some(20.0)];
    let series = [line("Input", RED, &values)];
    let chart = LineChart::new(&series, &format).reference(55.0);
    assert_eq!(layout_of(&chart, &s).max, nice_max(55.0));
}

#[test]
fn a_chart_with_no_room_draws_without_panicking() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let values = [Some(1.0), Some(2.0), Some(3.0)];
    let series = [line("Input", RED, &values)];
    let chart = LineChart::new(&series, &format).current(2);
    for width in [0.0, 5.0, 30.0] {
        let mut list = DrawList::new();
        chart.draw(0.0, 0.0, width, &mut list, &s, &at(width * 0.5, 50.0));
    }
    let one = [Some(4.0)];
    let series = [line("Input", RED, &one)];
    let (list, out) = paint(&LineChart::new(&series, &format), &s, &at(200.0, 50.0));
    assert_eq!(out.hovered, Some(0));
    assert_eq!(
        circles_of(&list, RED)
            .iter()
            .filter(|c| c[2] == LONE_DOT)
            .count(),
        1
    );
}
