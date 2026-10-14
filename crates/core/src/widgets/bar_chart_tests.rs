//! The bar chart: a nice axis in thirds, segments stacked bottom up and
//! snapped, the 2px floor, the outlier break, hover that dims the other
//! bars and places a tooltip, the legend, the empty captions, estimated
//! series, and the reference line.
use super::*;
use crate::Theme;
use crate::widgets::chart::{HALO_WIDTH, HOVER_DOT, LINE_WIDTH};

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const VIEWPORT: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 800.0,
    height: 600.0,
};

fn series() -> [BarSeries<'static>; 2] {
    [
        BarSeries {
            name: "Input",
            color: RED,
            estimated: false,
        },
        BarSeries {
            name: "Thinking",
            color: BLUE,
            estimated: false,
        },
    ]
}

fn bar(segments: &[f64]) -> Bar<'_> {
    Bar {
        label: "09-21",
        long_label: "Sun 21 Sep",
        segments,
        reference: None,
        current: false,
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

/// Draw `chart` at the origin, `width` wide.
fn paint(
    chart: &BarChart<'_>,
    width: f32,
    s: &StyleResolver,
    input: &InputState,
) -> (DrawList, BarChartOutput) {
    let mut list = DrawList::new();
    let out = chart.draw(0.0, 0.0, width, &mut list, s, input);
    (list, out)
}

/// The rects of the quads painted in `color`, `[x, y, w, h]`.
fn rects_of(list: &DrawList, color: [f32; 4]) -> Vec<[f32; 4]> {
    list.chrome_instances()
        .filter(|c| c.bg == color)
        .map(|c| c.rect)
        .collect()
}

fn shows(list: &DrawList, text: &str) -> bool {
    list.texts.iter().any(|t| t.content == text)
}

#[test]
fn segments_stack_bottom_up_on_whole_pixels() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let segments = [10.0, 20.0];
    let bars = [bar(&segments)];
    let chart = BarChart::new(&bars, &series, &format).legend(false);
    let (list, out) = paint(&chart, 300.0, &s, &away());
    // The axis is 30 (nice already): 10 of it is 63px of 190, 20 the rest.
    let base = PAD + CHART_HEIGHT;
    let red = rects_of(&list, RED);
    let blue = rects_of(&list, BLUE);
    assert_eq!(red.len(), 1);
    assert_eq!(blue.len(), 1);
    assert_eq!(red[0][1] + red[0][3], base, "it sits on the baseline");
    assert_eq!(red[0][3], 63.0);
    assert_eq!(blue[0][1] + blue[0][3], red[0][1], "the next on top of it");
    assert_eq!(blue[0][3], 127.0);
    assert_eq!(red[0][2], MAX_BAR_WIDTH, "a lone bar is capped in width");
    assert_eq!(out.height, PAD + CHART_HEIGHT + X_LABELS);
    for tick in ["0", "10", "20", "30"] {
        assert!(shows(&list, tick), "{tick}");
    }
}

#[test]
fn a_bar_too_short_to_show_is_two_pixels_of_its_largest_series() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let (tall, tiny) = ([1_000.0, 0.0], [0.5, 1.0]);
    let bars = [bar(&tall), bar(&tiny)];
    let chart = BarChart::new(&bars, &series, &format).legend(false);
    let (list, _) = paint(&chart, 300.0, &s, &away());
    let blue = rects_of(&list, BLUE);
    assert_eq!(blue.len(), 1);
    assert_eq!(blue[0][3], MIN_BAR);
}

#[test]
fn a_lone_outlier_is_broken_and_labelled_rather_than_flattening_the_rest() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let (usual, huge) = ([10.0, 0.0], [100.0, 0.0]);
    let bars = [bar(&usual), bar(&usual), bar(&usual), bar(&huge)];
    let chart = BarChart::new(&bars, &series, &format).legend(false);
    let (list, _) = paint(&chart, 400.0, &s, &away());
    // The axis is set by the usual bars: 10 × 1.15 → 12.
    assert!(shows(&list, "12"));
    assert!(shows(&list, "▲ 100"));
    let usual_height = (10.0 * CHART_HEIGHT as f64 / 12.0).round() as f32;
    let red = rects_of(&list, RED);
    assert_eq!(
        red.iter().filter(|r| r[3] == usual_height).count(),
        3,
        "the usual bars keep their height: {red:?}"
    );
}

#[test]
fn hovering_a_bar_dims_the_others_and_places_its_tooltip() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let segments = [10.0, 20.0];
    let bars = [bar(&segments), bar(&segments), bar(&segments)];
    let chart = BarChart::new(&bars, &series, &format).legend(false);
    // Find the bars first, away from them.
    let (list, _) = paint(&chart, 300.0, &s, &away());
    let red = rects_of(&list, RED);
    assert_eq!(red.len(), 3);
    let first = red[0];
    let over = InputState {
        mouse_x: first[0] + first[2] * 0.5,
        mouse_y: first[1],
        ..Default::default()
    };
    let (list, out) = paint(&chart, 300.0, &s, &over);
    assert_eq!(out.hovered, Some(0));
    let dimmed = [RED[0], RED[1], RED[2], DIMMED];
    assert_eq!(rects_of(&list, RED).len(), 1);
    assert_eq!(rects_of(&list, dimmed).len(), 2);
    let tip = out.tooltip.expect("a tooltip over a bar");
    assert!(tip.right_of_bar, "the first bar's tooltip goes right of it");

    let mut tips = DrawList::new();
    let rect = chart.draw_tooltip(&tip, VIEWPORT, &mut tips, &s).unwrap();
    assert!(rect.x >= first[0] + first[2]);
    for text in ["Sun 21 Sep", "Input", "Thinking", "Total", "30"] {
        assert!(shows(&tips, text), "{text}");
    }

    // Lit from a table row instead: dimmed the same, but no tooltip.
    let (list, out) = paint(&chart.hovered(Some(1)), 300.0, &s, &away());
    assert_eq!(out.hovered, None);
    assert_eq!(out.tooltip, None);
    assert_eq!(rects_of(&list, dimmed).len(), 2);
}

#[test]
fn the_tooltip_gives_the_reference_and_the_share_of_it_spent() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let segments = [30.0, 20.0];
    let bars = [Bar {
        reference: Some(200.0),
        current: true,
        ..bar(&segments)
    }];
    let chart = BarChart::new(&bars, &series, &format);
    let tip = BarTooltip {
        index: 0,
        bar_left: 500.0,
        bar_right: 520.0,
        right_of_bar: false,
        top: 0.0,
    };
    let mut list = DrawList::new();
    let rect = chart.draw_tooltip(&tip, VIEWPORT, &mut list, &s).unwrap();
    let left_of_bar = 500.0 - TIP_GAP + 0.5;
    assert!(rect.right() <= left_of_bar, "left of the bar: {rect:?}");
    for text in ["200 · 25%", "cap (estimated)", "RUNNING", "50"] {
        assert!(shows(&list, text), "{text}");
    }
    // Kept inside the viewport where the bar's side has too little room.
    let cramped = BarTooltip {
        bar_left: 20.0,
        bar_right: 40.0,
        ..tip
    };
    let rect = chart.draw_tooltip(&cramped, VIEWPORT, &mut list, &s);
    assert_eq!(rect.unwrap().x, 0.0);
}

#[test]
fn the_legend_lists_the_series_the_cap_and_the_current_bar_and_reports_hover() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let mut series = series();
    series[1].estimated = true;
    let segments = [10.0, 20.0];
    let bars = [Bar {
        reference: Some(50.0),
        current: true,
        ..bar(&segments)
    }];
    let chart = BarChart::new(&bars, &series, &format);
    let (mut list, out) = paint(&chart, 600.0, &s, &away());
    let entries = ["Input", "Thinking", "EST.", "cap (estimated)"];
    for text in entries.into_iter().chain(["current window"]) {
        assert!(shows(&list, text), "{text}");
    }
    assert_eq!(out.height, chart.measure_height(600.0, &mut list, &s));
    // An estimated series is faint inside its outline.
    let faint = [BLUE[0], BLUE[1], BLUE[2], ESTIMATED_FILL];
    assert!(!rects_of(&list, faint).is_empty());

    let input = list.texts.iter().find(|t| t.content == "Input").unwrap();
    let over = InputState {
        mouse_x: input.x + 2.0,
        mouse_y: input.y + 4.0,
        ..Default::default()
    };
    let (_, out) = paint(&chart, 600.0, &s, &over);
    assert_eq!(out.hovered_series, Some(0));
    let (list, _) = paint(&chart.hovered_series(Some(0)), 600.0, &s, &away());
    let faded = [BLUE[0], BLUE[1], BLUE[2], ESTIMATED_FILL * SERIES_FADED];
    assert!(!rects_of(&list, faded).is_empty(), "the other series fades");
}

#[test]
fn no_bars_or_only_zeros_say_so() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let (list, _) = paint(&BarChart::new(&[], &series, &format), 300.0, &s, &away());
    assert!(shows(&list, "NO USAGE IN THIS RANGE"));

    let zeros = [0.0, 0.0];
    let bars = [bar(&zeros), bar(&zeros)];
    let chart = BarChart::new(&bars, &series, &format);
    let (list, _) = paint(&chart, 300.0, &s, &away());
    assert!(shows(&list, "NO USAGE IN ANY BUCKET"));
    // A zero bar is a 1px tick on the baseline, in dim ink.
    let ticks = rects_of(&list, s.ink(Ink::Dim));
    assert_eq!(ticks.iter().filter(|r| r[3] == 1.0).count(), 2);
}

#[test]
fn many_bars_label_only_every_so_many_counting_from_the_newest() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let segments = [1.0, 1.0];
    let labels: Vec<String> = (0..30).map(|i| format!("d{i}")).collect();
    let bars: Vec<Bar<'_>> = labels
        .iter()
        .map(|label| Bar {
            label,
            ..bar(&segments)
        })
        .collect();
    let chart = BarChart::new(&bars, &series, &format).legend(false);
    let (list, _) = paint(&chart, 300.0, &s, &away());
    let shown: Vec<&str> = list
        .texts
        .iter()
        .filter(|t| t.content.starts_with('d'))
        .map(|t| t.content.as_str())
        .collect();
    assert!(shown.contains(&"d29"), "the newest is always labelled");
    assert!(shown.len() < 10, "not every bar: {shown:?}");
}

/// The shapes of `bars` plain bars of two series, 900px wide, no legend.
fn shapes_of(bars: usize) -> crate::PrimCounts {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let segments = [1.0, 2.0];
    let bars = vec![bar(&segments); bars];
    let chart = BarChart::new(&bars, &series, &format).legend(false);
    paint(&chart, 900.0, &s, &away()).0.prim_counts()
}

#[test]
fn a_bar_costs_a_shape_a_series_and_its_top_while_labels_stay_as_many_as_fit() {
    let (few, more) = (shapes_of(30), shapes_of(60));
    // Each segment, and the lit top edge: no text, no soup.
    assert_eq!(more.chrome_instances - few.chrome_instances, 30 * 3);
    assert_eq!((few.indices, more.indices), (0, 0));
    // The x labels thin out to what fits the width, whatever the count.
    let most = shapes_of(10_000);
    assert!(
        most.texts <= 4 + (900.0 / LABEL_SPACING) as usize,
        "{most:?}"
    );
}

#[test]
fn past_a_bar_every_two_pixels_the_cost_stops_growing() {
    let (many, most) = (shapes_of(2_000), shapes_of(10_000));
    assert_eq!(many, most, "both are grouped to the plot's width");
    // Narrow bars: a shape a series a slot, no top edge.
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let segments = [1.0, 2.0];
    let bars = vec![bar(&segments); 10_000];
    let chart = BarChart::new(&bars, &series, &format).legend(false);
    let (list, _) = paint(&chart, 900.0, &s, &away());
    let slots = chart.layout(900.0, &mut DrawList::new(), &s).slots;
    assert!((300..450).contains(&slots), "{slots}");
    assert_eq!(rects_of(&list, RED).len(), slots);
    assert_eq!(rects_of(&list, BLUE).len(), slots);
    assert!(rects_of(&list, BAR_HI).is_empty());
}

#[test]
fn grouped_bars_show_each_groups_tallest_and_hover_reports_it() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let (usual, tall) = ([1.0], [2.0]);
    let mut bars = vec![bar(&usual); 1_000];
    bars[500] = bar(&tall);
    let chart = BarChart::new(&bars, &series, &format).legend(false);
    let (list, _) = paint(&chart, 300.0, &s, &away());
    let drawn = rects_of(&list, RED);
    assert!(
        (60..=150).contains(&drawn.len()),
        "a bar every two pixels or so of the plot, not 1,000: {}",
        drawn.len()
    );
    let tallest = drawn
        .iter()
        .max_by(|a, b| a[3].total_cmp(&b[3]))
        .copied()
        .unwrap();
    assert!(
        drawn.iter().filter(|r| r[3] == tallest[3]).count() == 1,
        "only the tall bar's group shows it"
    );
    let over = InputState {
        mouse_x: tallest[0] + tallest[2] * 0.5,
        mouse_y: tallest[1] + 1.0,
        ..Default::default()
    };
    let (_, out) = paint(&chart, 300.0, &s, &over);
    assert_eq!(out.hovered, Some(500));
    let tip = out.tooltip.expect("a tooltip for it");
    assert!(tip.bar_left <= tallest[0] && tallest[0] < tip.bar_right);
}

#[test]
fn a_running_bar_stays_shown_in_its_group() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let (usual, small) = ([2.0], [1.0]);
    let mut bars = vec![bar(&usual); 1_000];
    bars[999] = Bar {
        current: true,
        ..bar(&small)
    };
    let chart = BarChart::new(&bars, &series, &format).legend(false);
    let (list, _) = paint(&chart, 300.0, &s, &away());
    assert!(shows(&list, "now"), "the running bar's label");
    let drawn = rects_of(&list, RED);
    let last = drawn.iter().max_by(|a, b| a[0].total_cmp(&b[0])).unwrap();
    let first = drawn.iter().min_by(|a, b| a[0].total_cmp(&b[0])).unwrap();
    assert!(last[3] < first[3], "the newest group shows the running bar");
}

#[test]
fn narrow_bars_skip_their_top_edge_and_cap_with_a_plain_tick() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let segments = [1.0];
    let capped = Bar {
        reference: Some(2.0),
        ..bar(&segments)
    };
    // 200 bars in 600px: under 3px each.
    let bars = vec![capped; 200];
    let chart = BarChart::new(&bars, &series, &format).legend(false);
    let (list, _) = paint(&chart, 600.0, &s, &away());
    assert!(rects_of(&list, BAR_HI).is_empty(), "no lit top edges");
    assert!(
        rects_of(&list, REFERENCE_EDGE).is_empty(),
        "no dark cap edges"
    );
    assert_eq!(list.stripe_instance_count(), 0, "no dashes");

    // Wide bars keep both, the cap one dashed record a bar.
    let bars = vec![capped; 3];
    let chart = BarChart::new(&bars, &series, &format).legend(false);
    let (list, _) = paint(&chart, 600.0, &s, &away());
    assert_eq!(rects_of(&list, BAR_HI).len(), 3);
    assert_eq!(rects_of(&list, REFERENCE_EDGE).len(), 6);
    assert_eq!(list.stripe_instance_count(), 3);
}

#[test]
fn a_cap_over_every_bar_does_not_hide_the_tallest_in_its_group() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let (usual, tall) = ([1.0], [2.0]);
    let capped = |segments| Bar {
        reference: Some(4.0),
        ..bar(segments)
    };
    // Every peak is the cap; the stacks still differ.
    let mut bars = vec![capped(&usual); 1_000];
    bars[501] = capped(&tall);
    let chart = BarChart::new(&bars, &series, &format).legend(false);
    let (list, _) = paint(&chart, 300.0, &s, &away());
    let drawn = rects_of(&list, RED);
    let tallest = drawn.iter().map(|r| r[3]).fold(0.0, f32::max);
    assert_eq!(drawn.iter().filter(|r| r[3] == tallest).count(), 1);
    assert!(drawn.iter().any(|r| r[3] < tallest), "the rest are usual");
}

#[test]
fn a_bar_lit_from_a_table_is_drawn_in_its_slot() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let (usual, short) = ([2.0], [1.0]);
    let mut bars = vec![bar(&usual); 1_000];
    let lit = Bar {
        label: "lit",
        ..bar(&short)
    };
    bars[500] = lit;
    // Its slot shows a taller neighbour until the table lights it.
    let chart = BarChart::new(&bars, &series, &format).legend(false);
    let (list, _) = paint(&chart, 300.0, &s, &away());
    let lowest = |list: &DrawList| {
        rects_of(list, RED)
            .iter()
            .map(|r| r[3])
            .fold(f32::MAX, f32::min)
    };
    let usual_height = lowest(&list);
    let chart = chart.hovered(Some(500));
    let (list, out) = paint(&chart, 300.0, &s, &away());
    assert!(
        lowest(&list) < usual_height,
        "the lit bar, not its neighbour"
    );
    assert_eq!(out.hovered, None, "lit, not under the pointer");
    let layout = chart.layout(300.0, &mut DrawList::new(), &s);
    let step = layout
        .slots
        .div_ceil(((300.0 - layout.gutter) / LABEL_SPACING) as usize);
    if (layout.slots - 1 - layout.slot_of(500)).is_multiple_of(step) {
        assert!(shows(&list, "lit"), "its label, where its slot has one");
    }
}

#[test]
fn an_outlier_keeps_its_slot_over_the_running_bar() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let (usual, huge) = ([10.0], [100.0]);
    let mut bars = vec![bar(&usual); 1_000];
    bars[998] = bar(&huge);
    bars[999] = Bar {
        current: true,
        ..bar(&usual)
    };
    let chart = BarChart::new(&bars, &series, &format).legend(false);
    let (list, _) = paint(&chart, 300.0, &s, &away());
    // The axis kept room for its value, so the bar and its label show.
    assert!(shows(&list, "▲ 100"));
}

#[test]
fn every_slot_shows_one_of_its_own_bars() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let segments = [1.0];
    for n in [1, 2, 99, 100, 101, 333, 1_000, 4_097] {
        let bars = vec![bar(&segments); n];
        let chart = BarChart::new(&bars, &series, &format).legend(false);
        for width in [0.0, 40.0, 233.0, 300.0, 900.0] {
            let mut layout = chart.layout(width, &mut DrawList::new(), &s);
            layout.shown = chart.group(&layout);
            assert!(layout.slots >= 1 && layout.slots <= n);
            let mut last = None;
            for k in 0..layout.slots {
                let i = layout.shown(k);
                assert_eq!(layout.slot_of(i), k, "n {n}, width {width}");
                assert!(last.is_none_or(|last| last < i), "in order");
                last = Some(i);
            }
            // Every bar falls in a slot, and the slots run in order.
            assert_eq!(layout.slot_of(0), 0);
            assert_eq!(layout.slot_of(n - 1), layout.slots - 1);
        }
    }
}

#[test]
fn narrow_bars_sit_on_whole_pixels() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let segments = [1.0];
    let bars = vec![bar(&segments); 1_000];
    let chart = BarChart::new(&bars, &series, &format).legend(false);
    let mut list = DrawList::new();
    chart.draw(0.3, 0.0, 333.0, &mut list, &s, &away());
    let drawn = rects_of(&list, RED);
    assert!(!drawn.is_empty());
    for r in &drawn {
        assert_eq!((r[0].fract(), r[2].fract()), (0.0, 0.0), "{r:?}");
    }
    let mut lefts: Vec<f32> = drawn.iter().map(|r| r[0]).collect();
    lefts.sort_by(f32::total_cmp);
    assert!(
        lefts.windows(2).all(|w| w[1] - w[0] > drawn[0][2]),
        "a gap between every two"
    );
}

#[test]
fn a_grouped_chart_with_no_room_draws_without_panicking() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let segments = [1.0];
    let bars = vec![bar(&segments); 1_000];
    let chart = BarChart::new(&bars, &series, &format);
    let inside = InputState {
        mouse_x: 1.0,
        mouse_y: 50.0,
        ..Default::default()
    };
    for width in [0.0, 1.0, 10.0] {
        let (_, out) = paint(&chart, width, &s, &inside);
        assert!(out.height > 0.0);
    }
    let none: [Bar<'_>; 0] = [];
    let chart = BarChart::new(&none, &series, &format).legend(false);
    let (list, out) = paint(&chart, 300.0, &s, &inside);
    assert_eq!(out.hovered, None);
    assert!(rects_of(&list, RED).is_empty());
}

const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

fn percent(value: f64) -> String {
    format!("{value}%")
}

fn overlay<'a>(values: &'a [Option<f64>], format: &'a dyn Fn(f64) -> String) -> BarOverlay<'a> {
    BarOverlay {
        label: "Cache hits",
        color: GREEN,
        values,
        max: None,
        format,
    }
}

/// The segments stroked in `color`, `[ax, ay, bx, by]`, of `half` width.
fn strokes_of(list: &DrawList, color: [f32; 4], half: f32) -> Vec<[f32; 4]> {
    list.segment_instances()
        .filter(|g| g.color == color && (g.translation[3] - half).abs() < 0.01)
        .map(|g| g.ends)
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

#[test]
fn an_overlay_runs_through_the_bar_centres_on_its_own_right_axis() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let segments = [10.0, 20.0];
    let bars = [bar(&segments); 4];
    let values = [Some(10.0), Some(20.0), None, Some(30.0)];
    let chart = BarChart::new(&bars, &series, &format)
        .overlay(overlay(&values, &percent))
        .legend(false);
    let layout = chart.layout(400.0, &mut DrawList::new(), &s);
    assert!(layout.right_gutter > 0.0);
    let (list, _) = paint(&chart, 400.0, &s, &away());
    for text in ["0%", "10%", "20%", "30%"] {
        assert!(shows(&list, text), "{text} on the right axis");
    }
    let right = list.texts.iter().find(|t| t.content == "30%").unwrap();
    assert!(right.x >= 400.0 - layout.right_gutter, "right of the plot");
    // The bars keep out of the right gutter.
    let red = rects_of(&list, RED);
    assert!(
        red.iter()
            .all(|r| r[0] + r[2] <= 400.0 - layout.right_gutter)
    );

    // The gap breaks it: a line from the first bar to the second, the last
    // alone. A dot on each.
    let lines = strokes_of(&list, GREEN, LINE_WIDTH * 0.5);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(strokes_of(&list, OVERLAY_HALO, HALO_WIDTH * 0.5).len(), 1);
    let (base, height) = (PAD + CHART_HEIGHT, CHART_HEIGHT);
    let center = |k: usize| layout.gutter + layout.center(k);
    let [ax, ay, bx, by] = lines[0];
    assert!((ax - center(0)).abs() < 0.01 && (bx - center(1)).abs() < 0.01);
    assert!((ay - (base - height / 3.0)).abs() < 0.01, "10 of 30");
    assert!((by - (base - height * 2.0 / 3.0)).abs() < 0.01, "20 of 30");
    let dots: Vec<_> = circles_of(&list, GREEN)
        .into_iter()
        .filter(|c| c[2] == DOT && c[3] <= 0.0)
        .collect();
    assert_eq!(dots.len(), 3);
    assert!((dots[2][1] - PAD).abs() < 0.01, "30 at the top");
}

#[test]
fn hovering_a_bar_lights_its_overlay_point_and_tells_its_value() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let segments = [10.0, 20.0];
    let bars = [bar(&segments); 3];
    let values = [Some(10.0), None, Some(300.0)];
    let chart = BarChart::new(&bars, &series, &format)
        .overlay(BarOverlay {
            max: Some(60.0),
            ..overlay(&values, &percent)
        })
        .legend(false);
    let layout = chart.layout(400.0, &mut DrawList::new(), &s);
    let over = |k: usize| InputState {
        mouse_x: layout.gutter + layout.center(k),
        mouse_y: PAD + 5.0,
        ..Default::default()
    };
    let (list, out) = paint(&chart, 400.0, &s, &over(2));
    assert_eq!(out.hovered, Some(2));
    // Past the axis' maximum it stays at the top.
    let lit: Vec<_> = circles_of(&list, GREEN)
        .into_iter()
        .filter(|c| c[2] == HOVER_DOT)
        .collect();
    assert_eq!(lit.len(), 1);
    assert!((lit[0][1] - PAD).abs() < 0.01);
    assert!(shows(&list, "60%"), "the maximum the caller gave");

    let mut tips = DrawList::new();
    chart.draw_tooltip(&out.tooltip.unwrap(), VIEWPORT, &mut tips, &s);
    assert!(shows(&tips, "Cache hits") && shows(&tips, "300%"));
    let (_, out) = paint(&chart, 400.0, &s, &over(1));
    let mut tips = DrawList::new();
    chart.draw_tooltip(&out.tooltip.unwrap(), VIEWPORT, &mut tips, &s);
    assert!(shows(&tips, "—"), "no value");
}

#[test]
fn the_overlay_has_a_legend_entry_and_fades_against_the_bars() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let segments = [10.0, 20.0];
    let bars = [bar(&segments); 3];
    let values = [Some(10.0), Some(20.0), Some(30.0)];
    let chart = BarChart::new(&bars, &series, &format).overlay(overlay(&values, &percent));
    let (list, out) = paint(&chart, 400.0, &s, &away());
    assert!(shows(&list, "Cache hits") && shows(&list, "RIGHT AXIS"));
    assert_eq!(
        out.height,
        chart.measure_height(400.0, &mut DrawList::new(), &s)
    );

    let entry = list
        .texts
        .iter()
        .find(|t| t.content == "Cache hits")
        .unwrap();
    let over = InputState {
        mouse_x: entry.x + 2.0,
        mouse_y: entry.y + 4.0,
        ..Default::default()
    };
    let (_, out) = paint(&chart, 400.0, &s, &over);
    assert!(out.hovered_overlay);
    assert_eq!(out.hovered_series, None);

    // The overlay lit: every bar fades (the legend's swatches don't).
    let chart = chart.legend(false);
    let (list, _) = paint(&chart.hovered_overlay(true), 400.0, &s, &away());
    let faded = |c: [f32; 4]| [c[0], c[1], c[2], SERIES_FADED];
    assert!(rects_of(&list, RED).is_empty() && rects_of(&list, BLUE).is_empty());
    assert_eq!(rects_of(&list, faded(RED)).len(), 3);
    assert!(!strokes_of(&list, GREEN, LINE_WIDTH * 0.5).is_empty());
    // A series lit: the overlay fades.
    let (list, _) = paint(&chart.hovered_series(Some(0)), 400.0, &s, &away());
    assert!(strokes_of(&list, GREEN, LINE_WIDTH * 0.5).is_empty());
    assert!(!strokes_of(&list, faded(GREEN), LINE_WIDTH * 0.5).is_empty());
}

#[test]
fn a_dense_overlay_drops_its_dots_and_stops_growing() {
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let series = series();
    let segments = [1.0, 2.0];
    let cost = |n: usize| {
        let bars = vec![bar(&segments); n];
        let values: Vec<_> = (0..n).map(|i| Some((i % 17) as f64)).collect();
        let chart = BarChart::new(&bars, &series, &format)
            .overlay(overlay(&values, &percent))
            .legend(false);
        let (list, _) = paint(&chart, 900.0, &s, &away());
        (list.prim_counts(), circles_of(&list, GREEN).len())
    };
    let ((many, many_dots), (most, most_dots)) = (cost(2_000), cost(10_000));
    assert_eq!(many, most);
    assert_eq!(
        (many_dots, most_dots),
        (0, 0),
        "slots under 4px have no dots"
    );
}
