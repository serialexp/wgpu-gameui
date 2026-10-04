//! The bar chart: a nice axis in thirds, segments stacked bottom up and
//! snapped, the 2px floor, the outlier break, hover that dims the other
//! bars and places a tooltip, the legend, the empty captions, estimated
//! series, and the reference line.
use super::*;
use crate::Theme;

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
fn the_axis_tops_out_at_a_nice_value_in_thirds() {
    assert_eq!(nice_max(0.0), 1.0);
    assert_eq!(nice_max(f64::NAN), 1.0);
    assert_eq!(nice_max(1.0), 1.2);
    assert_eq!(nice_max(100.0), 120.0);
    assert_eq!(nice_max(130.0), 150.0);
    assert_eq!(nice_max(250.0), 300.0);
    assert_eq!(nice_max(9.5), 12.0);
    assert_eq!(nice_max(1_200.0), 1_200.0);
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
    let base = PAD + BAR_CHART_HEIGHT;
    let red = rects_of(&list, RED);
    let blue = rects_of(&list, BLUE);
    assert_eq!(red.len(), 1);
    assert_eq!(blue.len(), 1);
    assert_eq!(red[0][1] + red[0][3], base, "it sits on the baseline");
    assert_eq!(red[0][3], 63.0);
    assert_eq!(blue[0][1] + blue[0][3], red[0][1], "the next on top of it");
    assert_eq!(blue[0][3], 127.0);
    assert_eq!(red[0][2], MAX_BAR_WIDTH, "a lone bar is capped in width");
    assert_eq!(out.height, PAD + BAR_CHART_HEIGHT + X_LABELS);
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
    let usual_height = (10.0 * BAR_CHART_HEIGHT as f64 / 12.0).round() as f32;
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
