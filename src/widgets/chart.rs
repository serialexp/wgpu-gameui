//! The frame Forge's charts share ([`BarChart`](super::BarChart),
//! [`LineChart`](super::LineChart)): an axis from zero to a "nice" maximum in
//! thirds, its gridlines and gutter of labels, the empty captions, the
//! accent "now" tick, the legend's rows, and the tooltip's surface.

use crate::SurfacePainter;
use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};

use super::{DrawList, Stroke};

/// The plot's height when the caller doesn't say.
pub const CHART_HEIGHT: f32 = 190.0;
/// The row of x labels under the plot.
pub(crate) const X_LABELS: f32 = 16.0;
/// Room over the plot for the tallest value.
pub(crate) const PAD: f32 = 10.0;
/// The space an x label wants, and how near an edge one is pinned to it.
pub(crate) const LABEL_SPACING: f32 = 64.0;
pub(crate) const EDGE_ROOM: f32 = 28.0;
/// The narrowest an axis' gutter gets, and the room between its labels and
/// the plot.
const MIN_GUTTER: f32 = 24.0;
pub(crate) const GUTTER_GAP: f32 = 6.0;
/// The other series while a legend entry is hovered.
pub(crate) const SERIES_FADED: f32 = 0.25;
const GRID: [f32; 4] = [1.0, 1.0, 1.0, 0.05];
const BASELINE: [f32; 4] = [1.0, 1.0, 1.0, 0.14];
const BASELINE_SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.5];
/// Reference lines: 3px dashes 2px apart.
pub(crate) const DASH: f32 = 3.0;
pub(crate) const DASH_GAP: f32 = 2.0;
/// The running point's "now" tick under the baseline.
pub(crate) const NOW_TICK: f32 = 2.0;
/// The legend: under the plot, entries wrapped in rows.
pub(crate) const LEGEND_GAP: f32 = 8.0;
pub(crate) const LEGEND_ROW: f32 = 16.0;
const LEGEND_ROW_GAP: f32 = 4.0;
const LEGEND_COL_GAP: f32 = 14.0;
pub(crate) const LEGEND_DASH: f32 = 12.0;
pub(crate) const SWATCH_GAP: f32 = 6.0;
/// The tooltip's padding, rows and the gap before a value.
pub(crate) const TIP_PAD_X: f32 = 8.0;
pub(crate) const TIP_PAD_Y: f32 = 6.0;
pub(crate) const TIP_ROW: f32 = 14.0;
pub(crate) const TIP_ROW_GAP: f32 = 3.0;
pub(crate) const TIP_VALUE_GAP: f32 = 14.0;
/// A row with nothing to show in the tooltip.
pub(crate) const TIP_ZERO: f32 = 0.5;
/// The gap after a tooltip row's swatch.
pub(crate) const TIP_SWATCH_GAP: f32 = 8.0;
/// Lines: a 1.5px stroke over a 3.5px dark halo.
pub(crate) const LINE_WIDTH: f32 = 1.5;
pub(crate) const HALO_WIDTH: f32 = 3.5;
/// Dots: a point's, the hovered point's and the ring round it, and the dark
/// rings round each.
pub(crate) const DOT: f32 = 2.5;
pub(crate) const HOVER_DOT: f32 = 3.5;
pub(crate) const HOVER_RING: f32 = 4.5;
pub(crate) const DOT_RING: [f32; 4] = [0.0, 0.0, 0.0, 0.75];
pub(crate) const HOVER_DOT_RING: [f32; 4] = [0.0, 0.0, 0.0, 0.8];
/// Dots are hidden past one every this many px.
pub(crate) const DOT_SPACING: f32 = 4.0;
/// The legend's line swatch, and the dot on it.
pub(crate) const LEGEND_LINE: f32 = 14.0;
const LEGEND_DOT: f32 = 3.0;
const LEGEND_DOT_X: f32 = 7.0;
/// A swatch line's thickness.
const SWATCH_LINE: f32 = 2.0;

/// The axis maxima: each divides cleanly into thirds.
const NICE: [f64; 9] = [1.2, 1.5, 2.4, 3.0, 4.5, 6.0, 7.5, 9.0, 12.0];

/// The smallest "nice" value at or over `value`: 1.2, 1.5, 2.4, 3, 4.5, 6,
/// 7.5, 9 or 12 times a power of ten, so the axis divides into thirds on
/// round numbers. 1 for nothing to show.
pub fn nice_max(value: f64) -> f64 {
    if value.is_nan() || value <= 0.0 {
        return 1.0;
    }
    let power = 10f64.powf(value.log10().floor());
    let fraction = value / power;
    let nice = NICE
        .iter()
        .copied()
        .find(|&nice| nice >= fraction - 1e-9)
        .unwrap_or(12.0);
    nice * power
}

/// The axis' labels at 0, ⅓, ⅔ and `max`, bottom up; with every value zero,
/// only the zero is labelled.
pub(crate) fn axis_ticks(max: f64, all_zero: bool, format: &dyn Fn(f64) -> String) -> [String; 4] {
    std::array::from_fn(|k| {
        if all_zero && k > 0 {
            String::new()
        } else if all_zero {
            format(0.0)
        } else {
            format(max * k as f64 / 3.0)
        }
    })
}

/// The gutter `ticks` need beside the plot: their widest, the gap, and a
/// pixel either side, never under [`MIN_GUTTER`].
pub(crate) fn gutter(ticks: &[String; 4], list: &mut DrawList, s: &StyleResolver) -> f32 {
    let widest = ticks
        .iter()
        .map(|tick| s.mono_width(list, tick, TextSize::Meta))
        .fold(0.0, f32::max);
    (widest + GUTTER_GAP + 2.0).ceil().max(MIN_GUTTER)
}

/// Where gridline `k` (0 the baseline, 3 the top) of `plot` runs.
pub(crate) fn gridline_y(plot: Rect, k: usize) -> f32 {
    plot.bottom() - (plot.height * k as f32 / 3.0).round()
}

/// The four gridlines across `plot`: the lit baseline at its bottom with its
/// shadow under it, and three faint lines over it.
pub(crate) fn draw_gridlines(list: &mut DrawList, plot: Rect) {
    for k in 0..4 {
        let line_y = gridline_y(plot, k);
        if k == 0 {
            list.quad(plot.x, line_y, plot.width, 1.0, BASELINE);
            list.quad(plot.x, line_y + 1.0, plot.width, 1.0, BASELINE_SHADOW);
        } else {
            list.quad(plot.x, line_y, plot.width, 1.0, GRID);
        }
    }
}

/// Which side of the plot an axis' labels go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AxisSide {
    /// Right-aligned [`GUTTER_GAP`] left of the plot.
    Left,
    /// Left-aligned [`GUTTER_GAP`] right of the plot.
    Right,
}

/// Write `ticks` beside `plot`'s gridlines on `side`, in `color` (the dim
/// ink when `None`).
pub(crate) fn draw_axis_labels(
    list: &mut DrawList,
    s: &StyleResolver,
    plot: Rect,
    side: AxisSide,
    ticks: &[String; 4],
    color: Option<[f32; 4]>,
) {
    let meta = s.text_size(TextSize::Meta);
    for (k, tick) in ticks.iter().enumerate() {
        if tick.is_empty() {
            continue;
        }
        let line_y = gridline_y(plot, k);
        let mut block = s.mono_block(tick, 0.0, 0.0, TextSize::Meta, Ink::Dim);
        if let Some(color) = color {
            block = block.with_color_f32(color);
        }
        let (w, _) = list.measure_block(&block);
        block.x = match side {
            AxisSide::Left => plot.x - GUTTER_GAP - w,
            AxisSide::Right => plot.right() + GUTTER_GAP,
        };
        block.y = crate::text::vcentered_line_y(line_y - 6.0, 12.0, meta);
        list.text(block);
    }
}

/// The caption across an empty `plot`, in capitals.
pub(crate) fn draw_empty_caption(
    list: &mut DrawList,
    s: &StyleResolver,
    plot: Rect,
    caption: &str,
) {
    let caption = caption.to_uppercase();
    let size = s.text_size(TextSize::Caption);
    let mut block = s.mono_block(
        &caption,
        0.0,
        crate::text::vcentered_line_y(plot.y, plot.height, size),
        TextSize::Caption,
        Ink::Caption,
    );
    let (w, _) = list.measure_block(&block);
    block.x = plot.x + ((plot.width - w) * 0.5).max(0.0);
    list.text(block);
}

/// The accent "now" tick under the baseline, glowing.
pub(crate) fn draw_now_tick(list: &mut DrawList, s: &StyleResolver, tick: Rect) {
    let accent = s.color(StyleKey::Accent);
    list.box_shadow_outset(
        tick,
        CornerRadii::uniform(0.0),
        BoxShadow {
            blur: 4.0,
            color: [accent[0], accent[1], accent[2], accent[3] * 0.5],
            ..BoxShadow::default()
        },
    );
    list.quad(tick.x, tick.y, tick.width, tick.height, accent);
}

/// A line through `points` in `color` over its dark `halo`, both dashed
/// `(on, off)` if `dash` says so.
pub(crate) fn stroke_with_halo(
    list: &mut DrawList,
    points: &[[f32; 2]],
    halo: [f32; 4],
    color: [f32; 4],
    dash: Option<(f32, f32)>,
) {
    let (mut under, mut line) = (Stroke::new(HALO_WIDTH), Stroke::new(LINE_WIDTH));
    if let Some((on, off)) = dash {
        under = under.dashed(on, off);
        line = line.dashed(on, off);
    }
    list.stroke_polyline(points, &under, halo);
    list.stroke_polyline(points, &line, color);
}

/// A dot of `radius` in `fill`, with a 1px `ring` centred on its edge.
pub(crate) fn dot(
    list: &mut DrawList,
    center: (f32, f32),
    radius: f32,
    fill: [f32; 4],
    ring: [f32; 4],
) {
    list.circle(center, radius, fill);
    list.circle_outline(center, radius, 1.0, ring);
}

/// The hovered point's dot: larger, ringed dark, and ringed again in the
/// value ink.
pub(crate) fn hovered_dot(
    list: &mut DrawList,
    s: &StyleResolver,
    center: (f32, f32),
    color: [f32; 4],
) {
    list.circle_outline(center, HOVER_RING, 1.0, s.ink(Ink::Value));
    dot(list, center, HOVER_DOT, color, HOVER_DOT_RING);
}

/// A 2px line `width` long from `x` along the row `mid`, dashed if `dashed`:
/// a line's swatch in a legend or tooltip.
pub(crate) fn line_swatch(
    list: &mut DrawList,
    x: f32,
    mid: f32,
    width: f32,
    color: [f32; 4],
    dashed: bool,
) {
    let mut stroke = Stroke::new(SWATCH_LINE);
    if dashed {
        stroke = stroke.dashed(DASH, DASH_GAP);
    }
    list.stroke_line([x, mid], [x + width, mid], &stroke, color);
}

/// A line's legend swatch, [`LEGEND_LINE`] long, with a dot on it if the
/// line has dots.
pub(crate) fn legend_line(
    list: &mut DrawList,
    x: f32,
    mid: f32,
    color: [f32; 4],
    dashed: bool,
    dots: bool,
) {
    line_swatch(list, x, mid, LEGEND_LINE, color, dashed);
    if dots {
        dot(list, (x + LEGEND_DOT_X, mid), LEGEND_DOT, color, DOT_RING);
    }
}

/// Lay legend `entries` (each with its width) out in rows `width` wide from
/// `start`, handing each to `place` with its rect relative to the legend's
/// top-left. Returns the number of rows.
pub(crate) fn legend_rows<E>(
    list: &mut DrawList,
    width: f32,
    start: f32,
    entries: impl IntoIterator<Item = (E, f32)>,
    mut place: impl FnMut(&mut DrawList, E, Rect),
) -> usize {
    let (mut x, mut row) = (start, 0);
    for (entry, w) in entries {
        if x > start && x + w > width {
            x = start;
            row += 1;
        }
        let y = row as f32 * (LEGEND_ROW + LEGEND_ROW_GAP);
        place(list, entry, Rect::new(x, y, w, LEGEND_ROW));
        x += w + LEGEND_COL_GAP;
    }
    row + 1
}

/// The height of `rows` rows of legend.
pub(crate) fn legend_height(rows: usize) -> f32 {
    rows as f32 * LEGEND_ROW + rows.saturating_sub(1) as f32 * LEGEND_ROW_GAP
}

/// The width a sans label takes in a legend or tooltip row.
pub(crate) fn row_text_width(list: &mut DrawList, s: &StyleResolver, text: &str) -> f32 {
    list.measure_block(&s.sans_block(text, 0.0, 0.0, TextSize::Row, Ink::Second))
        .0
}

/// The tooltip's surface around content `content_width` wide and
/// `content_height` tall, with its padding and border, `min_width` at least.
pub(crate) fn tooltip_size(
    s: &StyleResolver,
    content_width: f32,
    content_height: f32,
    min_width: f32,
) -> (f32, f32) {
    let border = s.tooltip().surface.border_widths;
    let width = (content_width + TIP_PAD_X * 2.0 + border.left + border.right).max(min_width);
    let height = content_height + TIP_PAD_Y * 2.0 + border.top + border.bottom;
    (width, height)
}

/// Paint the tooltip's surface at `rect` and its content through `content`,
/// which gets the area inside the padding.
pub(crate) fn paint_tooltip(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    content: impl FnOnce(&mut DrawList, Rect),
) {
    let chrome = s.tooltip();
    let border = chrome.surface.border_widths;
    let padding_box = rect.inset(border.left);
    let mut surface = SurfacePainter::new(
        list,
        rect,
        padding_box,
        chrome.surface.corner_radii,
        chrome.surface,
        std::slice::from_ref(&chrome.shadow),
        &chrome.lines,
    );
    surface.paint_pre_content();
    let inner = Rect::new(
        padding_box.x + TIP_PAD_X,
        padding_box.y + TIP_PAD_Y,
        padding_box.width - TIP_PAD_X * 2.0,
        padding_box.height - TIP_PAD_Y * 2.0,
    );
    content(surface.draw_list(), inner);
    surface.paint_post_content();
}

/// The accent "RUNNING" caption, right-aligned at `right` in the tooltip row
/// at `y`; returns nothing, measured by [`running_width`].
pub(crate) fn draw_running(list: &mut DrawList, s: &StyleResolver, right: f32, y: f32) {
    let caption = s.text_size(TextSize::Caption);
    let mut block = s
        .mono_block("RUNNING", 0.0, 0.0, TextSize::Caption, Ink::Dim)
        .with_color_f32(s.color(StyleKey::Accent));
    let (w, _) = list.measure_block(&block);
    block.x = right - w;
    block.y = crate::text::vcentered_line_y(y, TIP_ROW, caption);
    list.text(block);
}

/// The width [`draw_running`]'s caption takes.
pub(crate) fn running_width(list: &mut DrawList, s: &StyleResolver) -> f32 {
    s.mono_width(list, "RUNNING", TextSize::Meta)
}

/// A value in a tooltip row at `y`, right-aligned at `right`, in mono.
pub(crate) fn draw_tip_value(
    list: &mut DrawList,
    s: &StyleResolver,
    text: &str,
    right: f32,
    y: f32,
    ink: Ink,
    color: Option<[f32; 4]>,
) {
    let meta = s.text_size(TextSize::Meta);
    let mut block = s.mono_block(text, 0.0, 0.0, TextSize::Meta, ink);
    if let Some(color) = color {
        block = block.with_color_f32(color);
    }
    let (w, _) = list.measure_block(&block);
    block.x = right - w;
    block.y = crate::text::vcentered_line_y(y, TIP_ROW, meta);
    list.text(block);
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn all_zero_labels_only_the_baseline() {
        let format = |v: f64| format!("{v}");
        assert_eq!(axis_ticks(3.0, false, &format), ["0", "1", "2", "3"]);
        assert_eq!(axis_ticks(1.0, true, &format), ["0", "", "", ""]);
    }

    #[test]
    fn legend_entries_wrap_onto_a_new_row_at_the_width() {
        let mut list = DrawList::new();
        let mut placed = Vec::new();
        let rows = legend_rows(
            &mut list,
            110.0,
            10.0,
            [("a", 40.0), ("b", 40.0), ("c", 40.0)],
            |_, entry, r| placed.push((entry, r.x, r.y)),
        );
        assert_eq!(rows, 2);
        assert_eq!(
            placed,
            [
                ("a", 10.0, 0.0),
                ("b", 64.0, 0.0),
                ("c", 10.0, LEGEND_ROW + LEGEND_ROW_GAP)
            ]
        );
        assert_eq!(legend_height(2), LEGEND_ROW * 2.0 + LEGEND_ROW_GAP);
    }
}
