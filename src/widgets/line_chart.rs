//! Line chart — lines over a shared x axis in [`BarChart`](super::BarChart)'s
//! frame (Forge's `LineChart`): the same thirds gridlines on a nice maximum,
//! gutter, legend and tooltip, so the two sit side by side.
//!
//! Each series is a 1.5px line over a 3.5px dark halo, mitred at its turns
//! (a bevel past a miter four half-widths long), its runs broken where a
//! value is missing. One series may fill the area under it, fading from
//! 24% at the plot's top to nothing at the baseline; an estimated series is
//! dashed and its fill half as strong. A series may mark its points with
//! dots while there are fewer than one every 4px.
//!
//! The x values may be indexes or times (ms since the epoch,
//! [`LineChart::time`]): times are labelled on whole local hours, days,
//! Mondays or months, the finest that fits. A running point
//! ([`LineChart::current`]) gets a hollow dot and the accent "now".
//!
//! Past two points a pixel, each column of the plot keeps only its lowest
//! and highest value, so what is drawn stops growing with the data.
//!
//! Like [`BarChart`](super::BarChart), the widget only paints and reports
//! what the pointer is over: the caller feeds the hovered point back (and
//! may share it with a table of the same rows), and paints the tooltip last
//! ([`LineChart::draw_tooltip`]).

use crate::InputState;
use crate::layout::Rect;
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};

use super::DrawList;
use super::chart::{
    self, AxisSide, CHART_HEIGHT, DASH, DASH_GAP, DOT, DOT_RING, DOT_SPACING, EDGE_ROOM,
    LABEL_SPACING, LEGEND_DASH, LEGEND_GAP, LEGEND_LINE, NOW_TICK, PAD, SERIES_FADED, SWATCH_GAP,
    TIP_ROW, TIP_ROW_GAP, TIP_SWATCH_GAP, TIP_VALUE_GAP, TIP_ZERO, X_LABELS, nice_max,
};
use super::chart_time::{time_long, time_ticks};

/// The plot's points stay this far inside its left and right edges, so the
/// end dots aren't cut.
const INSET: f32 = 4.0;
/// The dark halo under a line.
const HALO: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
/// An estimated series' dashes.
const ESTIMATED_DASH: (f32, f32) = (4.0, 3.0);
/// The area's fade at the plot's top, and an estimated series'.
const AREA_TOP: f32 = 0.24;
const AREA_TOP_ESTIMATED: f32 = 0.12;
/// A lone point's dot (a run of one).
const LONE_DOT: f32 = 2.0;
/// The running point: a hollow dot over a dark backing.
const CURRENT_DOT: f32 = 3.0;
const CURRENT_BACKING: f32 = 4.5;
const CURRENT_BACKING_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
const CURRENT_STROKE: f32 = 1.5;
/// Its "now" tick under the baseline, and how far other x labels keep from
/// its label.
const NOW_TICK_WIDTH: f32 = 12.0;
const NOW_ROOM: f32 = 44.0;
/// The hover guide.
const GUIDE: [f32; 4] = [1.0, 1.0, 1.0, 0.18];
/// The reference line's strength, and the room left of its label.
const REFERENCE: f32 = 0.7;
const REFERENCE_LABEL_PAD: f32 = 4.0;
/// The legend's ring for the running point.
const LEGEND_CURRENT: f32 = 6.0;
const CURRENT_LABEL: &str = "current window · provisional";
/// The tooltip: beside the guide, its rows of series.
const TIP_GAP: f32 = 10.0;
const TIP_MIN_WIDTH: f32 = 150.0;
const TIP_SWATCH: f32 = 10.0;
const TIP_HEADER_GAP: f32 = 2.0;

/// One series of a [`LineChart`]: a line, and a legend entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineSeries<'a> {
    /// Its legend entry and tooltip row.
    pub name: &'a str,
    /// Categorical, e.g. `oklch(0.68 0.1 <hue>)`.
    pub color: [f32; 4],
    /// One per x; `None` (or a value that isn't finite) breaks the line.
    pub values: &'a [Option<f64>],
    /// Mark each point, while there are fewer than one every 4px.
    pub dots: bool,
    /// Fill the area under the line, fading to nothing at the baseline. One
    /// series a chart: the primary one.
    pub area: bool,
    /// Estimated rather than measured: dashed, its fill half as strong.
    pub estimated: bool,
}

impl LineSeries<'_> {
    /// The value at `i`, if there is one.
    fn value(&self, i: usize) -> Option<f64> {
        self.values
            .get(i)
            .copied()
            .flatten()
            .filter(|v| v.is_finite())
    }
}

/// What a [`LineChart`] did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LineChartOutput {
    /// The point the pointer is nearest along x, if it is over the plot.
    pub hovered: Option<usize>,
    /// The legend entry under the pointer, if any.
    pub hovered_series: Option<usize>,
    /// The height it took, legend included.
    pub height: f32,
    /// Where the hovered point's tooltip goes, for
    /// [`LineChart::draw_tooltip`]. Only while the pointer itself is over
    /// the plot: a point lit from a table row shows no tooltip.
    pub tooltip: Option<LineTooltip>,
}

/// Where a point's tooltip goes: beside the guide, on the side with more
/// room.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineTooltip {
    /// The point's index.
    pub index: usize,
    /// The guide's x.
    pub guide: f32,
    /// Whether the tooltip goes right of the guide (it is in the left half).
    pub right_of_guide: bool,
    /// The chart's top, where the tooltip's top goes.
    pub top: f32,
}

/// The layout every part of the chart shares, worked out once a frame.
struct Layout {
    /// How many points there are.
    n: usize,
    /// The axis maximum.
    max: f64,
    /// Whether any series has a value, and whether every value is zero.
    any: bool,
    all_zero: bool,
    /// The y labels, bottom up, and their column left of the plot.
    ticks: [String; 4],
    gutter: f32,
    /// The plot's left edge, the span the points are spread over, and the
    /// first and last x.
    left: f32,
    span: f32,
    x0: f64,
    x1: f64,
    /// Where the plot's baseline is, and how tall it is.
    base: f32,
    height: f32,
}

impl Layout {
    /// Where x value `v` is.
    fn px(&self, v: f64) -> f32 {
        let t = if self.x1 > self.x0 {
            ((v - self.x0) / (self.x1 - self.x0)) as f32
        } else {
            0.5
        };
        self.left + INSET + t * self.span
    }

    /// Where value `v` is, cut off at the plot's top.
    fn py(&self, v: f64) -> f32 {
        self.base - (v / self.max).min(1.0) as f32 * self.height
    }
}

/// A line chart.
#[derive(Clone, Copy)]
pub struct LineChart<'a> {
    series: &'a [LineSeries<'a>],
    format: &'a dyn Fn(f64) -> String,
    x: Option<&'a [f64]>,
    labels: Option<&'a [&'a str]>,
    long_labels: Option<&'a [&'a str]>,
    x_format: Option<&'a dyn Fn(f64) -> String>,
    time: Option<i32>,
    current: Option<usize>,
    height: f32,
    hovered: Option<usize>,
    hovered_series: Option<usize>,
    legend: bool,
    reference: Option<f64>,
    reference_label: &'a str,
    empty_label: &'a str,
    zero_label: &'a str,
}

impl<'a> LineChart<'a> {
    /// A chart of `series`, its values written by `format` (`42%`) on the
    /// axis and in the tooltip. Its x is each point's index until
    /// [`x`](Self::x) says otherwise.
    pub fn new(series: &'a [LineSeries<'a>], format: &'a dyn Fn(f64) -> String) -> Self {
        Self {
            series,
            format,
            x: None,
            labels: None,
            long_labels: None,
            x_format: None,
            time: None,
            current: None,
            height: CHART_HEIGHT,
            hovered: None,
            hovered_series: None,
            legend: true,
            reference: None,
            reference_label: "limit",
            empty_label: "No data in this range",
            zero_label: "All values are zero",
        }
    }

    /// The x of each point, sorted ascending: indexes, or times in ms since
    /// the epoch (see [`time`](Self::time)). The series' values line up with
    /// it.
    pub fn x(mut self, x: &'a [f64]) -> Self {
        self.x = Some(x);
        self
    }

    /// An x label per point, thinned to fit and counted from the newest.
    pub fn labels(mut self, labels: &'a [&'a str]) -> Self {
        self.labels = Some(labels);
        self
    }

    /// A tooltip title per point.
    pub fn long_labels(mut self, labels: &'a [&'a str]) -> Self {
        self.long_labels = Some(labels);
        self
    }

    /// How an x value is written without [`labels`](Self::labels) or a time
    /// axis.
    pub fn x_format(mut self, format: &'a dyn Fn(f64) -> String) -> Self {
        self.x_format = Some(format);
        self
    }

    /// The x values are times (ms since the epoch), labelled on whole local
    /// hours, days, Mondays or months, local time being `offset` seconds east
    /// of UTC. Ignored with [`labels`](Self::labels).
    pub fn time(mut self, offset: i32) -> Self {
        self.time = Some(offset);
        self
    }

    /// The running point: a hollow dot on every series, the accent "now"
    /// under it and "running" in its tooltip.
    pub fn current(mut self, index: usize) -> Self {
        self.current = Some(index);
        self
    }

    /// The plot's height.
    pub fn height(mut self, height: f32) -> Self {
        self.height = height.max(1.0);
        self
    }

    /// The point to light: last frame's [`LineChartOutput::hovered`], or the
    /// row hovered in a table of the same points.
    pub fn hovered(mut self, hovered: Option<usize>) -> Self {
        self.hovered = hovered;
        self
    }

    /// The series to light: last frame's
    /// [`LineChartOutput::hovered_series`].
    pub fn hovered_series(mut self, hovered: Option<usize>) -> Self {
        self.hovered_series = hovered;
        self
    }

    /// Whether to draw the legend under the plot.
    pub fn legend(mut self, legend: bool) -> Self {
        self.legend = legend;
        self
    }

    /// A dashed line across the plot at `value`, labelled on the right.
    pub fn reference(mut self, value: f64) -> Self {
        self.reference = value.is_finite().then_some(value);
        self
    }

    /// What the reference line is called on the plot and in the legend.
    pub fn reference_label(mut self, label: &'a str) -> Self {
        self.reference_label = label;
        self
    }

    /// What the plot says with no values, and with values that are all zero.
    pub fn empty_labels(mut self, empty: &'a str, zero: &'a str) -> Self {
        self.empty_label = empty;
        self.zero_label = zero;
        self
    }

    /// How many points there are: as many as x values, or the longest
    /// series.
    fn points(&self) -> usize {
        self.x.map_or_else(
            || {
                self.series
                    .iter()
                    .map(|s| s.values.len())
                    .max()
                    .unwrap_or(0)
            },
            <[f64]>::len,
        )
    }

    /// Point `i`'s x.
    fn xv(&self, i: usize) -> f64 {
        self.x.map_or(i as f64, |x| x[i])
    }

    /// The running point, if it is one of the points.
    fn current_point(&self, n: usize) -> Option<usize> {
        self.current.filter(|&i| i < n)
    }

    fn layout(&self, x: f32, y: f32, width: f32, list: &mut DrawList, s: &StyleResolver) -> Layout {
        let n = self.points();
        let (mut any, mut most) = (false, 0.0_f64);
        for series in self.series {
            for i in 0..n {
                if let Some(v) = series.value(i) {
                    any = true;
                    most = most.max(v);
                }
            }
        }
        let all_zero = any && most <= 0.0;
        let max = nice_max(most.max(self.reference.unwrap_or(0.0)));
        let ticks = chart::axis_ticks(max, all_zero, self.format);
        let gutter = if any {
            chart::gutter(&ticks, list, s)
        } else {
            0.0
        };
        let (x0, x1) = if n > 0 {
            (self.xv(0), self.xv(n - 1))
        } else {
            (0.0, 1.0)
        };
        Layout {
            n,
            max,
            any,
            all_zero,
            ticks,
            gutter,
            left: x + gutter,
            span: (width - gutter - INSET * 2.0).max(0.0),
            x0,
            x1,
            base: y + PAD + self.height,
            height: self.height,
        }
    }

    /// The point nearest x position `mx`.
    fn nearest(&self, layout: &Layout, mx: f32) -> usize {
        let n = layout.n;
        let t = layout.x0
            + f64::from((mx - layout.left - INSET) / layout.span.max(1.0))
                * (layout.x1 - layout.x0);
        let (mut lo, mut hi) = (0, n - 1);
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if self.xv(mid) < t {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        if (self.xv(lo) - t).abs() <= (self.xv(hi) - t).abs() {
            lo
        } else {
            hi
        }
    }

    /// The point the pointer is nearest, with the chart's plot area (labels
    /// included) at `area`.
    fn hit(&self, area: Rect, layout: &Layout, input: &InputState) -> Option<usize> {
        if input.mouse_consumed || layout.n == 0 || !layout.any {
            return None;
        }
        let plot = Rect::new(layout.left, area.y, area.right() - layout.left, area.height);
        plot.contains(input.mouse_x, input.mouse_y)
            .then(|| self.nearest(layout, input.mouse_x))
    }

    /// The height it takes `width` wide, legend included.
    pub fn measure_height(&self, width: f32, list: &mut DrawList, s: &StyleResolver) -> f32 {
        let layout = self.layout(0.0, 0.0, width, list, s);
        let mut height = PAD + self.height + X_LABELS;
        if self.legend && !self.series.is_empty() {
            let rows = self.legend_entries(width, &layout, list, s, |_, _, _| {});
            height += LEGEND_GAP + chart::legend_height(rows);
        }
        height
    }

    /// Draw it with its top-left corner at `(x, y)`, `width` wide.
    pub fn draw(
        &self,
        x: f32,
        y: f32,
        width: f32,
        list: &mut DrawList,
        s: &StyleResolver,
        input: &InputState,
    ) -> LineChartOutput {
        let layout = self.layout(x, y, width, list, s);
        let area = Rect::new(x, y, width, PAD + self.height + X_LABELS);
        let (base, top) = (layout.base, layout.base - self.height);
        let n = layout.n;
        let hit = self.hit(area, &layout, input);
        let lit = hit.or(self.hovered).filter(|&i| i < n);
        let current = self.current_point(n);

        let plot = Rect::new(layout.left, top, area.right() - layout.left, self.height);
        chart::draw_gridlines(list, plot);
        if layout.any {
            chart::draw_axis_labels(list, s, plot, AxisSide::Left, &layout.ticks, None);
        }
        if !layout.any || layout.all_zero {
            let caption = if layout.any {
                self.zero_label
            } else {
                self.empty_label
            };
            chart::draw_empty_caption(list, s, plot, caption);
        }
        if let (Some(reference), true) = (self.reference, layout.any) {
            self.draw_reference(reference, area, &layout, list, s);
        }
        if let Some(i) = hit {
            let guide = layout.px(self.xv(i)).round();
            list.quad(guide, top, 1.0, self.height, GUIDE);
        }

        if layout.any && layout.span > 0.0 {
            let mut points = Vec::new();
            let mut run = Vec::new();
            let dots = n as f32 <= layout.span / DOT_SPACING;
            for (index, series) in self.series.iter().enumerate() {
                self.with_fade(index, list, |list| {
                    self.reduce(series, &layout, &mut points);
                    self.draw_series(series, &points, &mut run, &layout, list);
                    if series.dots && dots {
                        for i in 0..n {
                            if let Some(v) = series.value(i) {
                                let center = (layout.px(self.xv(i)), layout.py(v));
                                chart::dot(list, center, DOT, series.color, DOT_RING);
                            }
                        }
                    }
                });
            }
            self.draw_marked(lit, current, &layout, list, s);
        }
        if let (Some(i), true) = (current, layout.any) {
            let center = layout.px(self.xv(i)).round();
            let tick = Rect::new(
                center - NOW_TICK_WIDTH * 0.5,
                base + 1.0,
                NOW_TICK_WIDTH,
                NOW_TICK,
            );
            chart::draw_now_tick(list, s, tick);
        }
        if layout.any {
            self.draw_x_labels(area, &layout, lit, current, list, s);
        }

        let mut hovered_series = None;
        let mut height = area.height;
        if self.legend && !self.series.is_empty() {
            let legend_y = area.bottom() + LEGEND_GAP;
            let rows = self.legend_entries(width, &layout, list, s, |list, entry, r| {
                let r = Rect::new(x + r.x, legend_y + r.y, r.width, r.height);
                if let LegendEntry::Series(index) = entry
                    && !input.mouse_consumed
                    && r.contains(input.mouse_x, input.mouse_y)
                {
                    hovered_series = Some(index);
                }
                self.draw_legend_entry(entry, r, list, s);
            });
            height += LEGEND_GAP + chart::legend_height(rows);
        }

        let tooltip = hit.map(|index| {
            let guide = layout.px(self.xv(index));
            LineTooltip {
                index,
                guide,
                right_of_guide: guide < x + width * 0.5,
                top: y,
            }
        });
        LineChartOutput {
            hovered: hit,
            hovered_series,
            height,
            tooltip,
        }
    }

    /// Run `paint` faded when another series' legend entry is hovered.
    fn with_fade(&self, index: usize, list: &mut DrawList, paint: impl FnOnce(&mut DrawList)) {
        let faded = self.hovered_series.is_some_and(|lit| lit != index);
        list.push_tint();
        if faded {
            list.multiply_tint([1.0, 1.0, 1.0, SERIES_FADED]);
        }
        paint(list);
        list.pop_tint();
    }

    /// `series`' points as `(x, value)`, `None` breaking the line: every
    /// point, or past two a pixel column, each column's lowest and highest
    /// in the order they come, a column with a gap ending its run after it.
    fn reduce(&self, series: &LineSeries<'_>, layout: &Layout, out: &mut Vec<(f64, Option<f64>)>) {
        out.clear();
        let n = layout.n;
        if n <= layout.span.ceil() as usize * 2 {
            out.extend((0..n).map(|i| (self.xv(i), series.value(i))));
            return;
        }
        let scale = if layout.x1 > layout.x0 {
            f64::from(layout.span) / (layout.x1 - layout.x0)
        } else {
            0.0
        };
        // Columns of the drawn pixels, as `Layout::px` places them.
        let origin = f64::from(layout.left + INSET);
        let column_of = |x: f64| (origin + (x - layout.x0) * scale).floor() as i64;
        let mut column = Column::new(column_of(self.xv(0)));
        for i in 0..n {
            let x = self.xv(i);
            let at = column_of(x);
            if at != column.at {
                column.flush(out);
                column = Column::new(at);
            }
            column.add(i, x, series.value(i));
        }
        column.flush(out);
    }

    /// Paint `series` through `points`: under each run its area, then its
    /// halo and line, and a lone point as a small dot.
    fn draw_series(
        &self,
        series: &LineSeries<'_>,
        points: &[(f64, Option<f64>)],
        run: &mut Vec<[f32; 2]>,
        layout: &Layout,
        list: &mut DrawList,
    ) {
        let paint = |run: &[[f32; 2]], list: &mut DrawList| match run {
            [] => {}
            [only] => chart::dot(list, (only[0], only[1]), LONE_DOT, series.color, DOT_RING),
            _ => {
                if series.area {
                    self.paint_area(series, run, layout, list);
                }
                let dash = series.estimated.then_some(ESTIMATED_DASH);
                chart::stroke_with_halo(list, run, HALO, series.color, dash);
            }
        };
        run.clear();
        for &(x, value) in points {
            match value {
                Some(v) => run.push([layout.px(x), layout.py(v)]),
                None => {
                    paint(run, list);
                    run.clear();
                }
            }
        }
        paint(run, list);
    }

    /// The area under `run`, down to the baseline, fading from the series'
    /// colour at the plot's top to nothing at the baseline: one triangle
    /// strip through each point and the baseline under it, coloured at its
    /// corners, which reproduces the fade exactly since it is linear in y.
    fn paint_area(
        &self,
        series: &LineSeries<'_>,
        run: &[[f32; 2]],
        layout: &Layout,
        list: &mut DrawList,
    ) {
        let top = if series.estimated {
            AREA_TOP_ESTIMATED
        } else {
            AREA_TOP
        };
        let base = layout.base;
        let at = |y: f32| {
            let mut color = series.color;
            color[3] *= (top * (base - y) / layout.height).clamp(0.0, 1.0);
            color
        };
        let clear = at(base);
        list.triangle_strip(
            run.iter()
                .flat_map(|&[x, y]| [((x, y), at(y)), ((x, base), clear)]),
        );
    }

    /// The hovered point's dots, ringed, and the running point's hollow
    /// ones (unless it is the hovered one).
    fn draw_marked(
        &self,
        lit: Option<usize>,
        current: Option<usize>,
        layout: &Layout,
        list: &mut DrawList,
        s: &StyleResolver,
    ) {
        for (index, series) in self.series.iter().enumerate() {
            self.with_fade(index, list, |list| {
                if let Some(i) = lit
                    && let Some(v) = series.value(i)
                {
                    let center = (layout.px(self.xv(i)), layout.py(v));
                    chart::hovered_dot(list, s, center, series.color);
                }
                if let Some(i) = current
                    && lit != Some(i)
                    && let Some(v) = series.value(i)
                {
                    let center = (layout.px(self.xv(i)), layout.py(v));
                    list.circle(center, CURRENT_BACKING, CURRENT_BACKING_COLOR);
                    list.circle(center, CURRENT_DOT, s.color(StyleKey::Background));
                    list.circle_outline(center, CURRENT_DOT, CURRENT_STROKE, series.color);
                }
            });
        }
    }

    /// The reference line across the plot, and its label right-aligned over
    /// it on a backing of the app's surface.
    fn draw_reference(
        &self,
        value: f64,
        area: Rect,
        layout: &Layout,
        list: &mut DrawList,
        s: &StyleResolver,
    ) {
        let line_y = layout.py(value).round();
        let mut ink = s.ink(Ink::Value);
        ink[3] *= REFERENCE;
        let width = area.right() - layout.left;
        list.dashed_hline(layout.left, line_y, width, DASH, DASH_GAP, ink);
        let text = format!("{} {}", self.reference_label, (self.format)(value));
        let caption = s.text_size(TextSize::Caption);
        let mut block = s.mono_block(&text, 0.0, 0.0, TextSize::Caption, Ink::Value);
        let (w, _) = list.measure_block(&block);
        block.x = area.right() - w;
        block.y = crate::text::vcentered_line_y(line_y - 12.0, 10.0, caption);
        let backing = Rect::new(
            block.x - REFERENCE_LABEL_PAD,
            line_y - 12.0,
            w + REFERENCE_LABEL_PAD,
            10.0,
        );
        list.quad(
            backing.x,
            backing.y,
            backing.width,
            backing.height,
            s.color(StyleKey::Background),
        );
        list.text(block);
    }

    /// The x labels: every so many points counted from the newest (or the
    /// time axis' ticks), the running point's accent "now" clearing the
    /// labels near it, and labels near an edge aligned to it.
    fn draw_x_labels(
        &self,
        area: Rect,
        layout: &Layout,
        lit: Option<usize>,
        current: Option<usize>,
        list: &mut DrawList,
        s: &StyleResolver,
    ) {
        let n = layout.n;
        let most = ((layout.span / LABEL_SPACING).floor() as usize).max(2);
        // Each label: where, what, and which point it names (if one).
        let mut labels: Vec<(f32, String, Option<usize>)> = Vec::new();
        match (self.time, self.labels) {
            (Some(offset), None) => {
                for tick in time_ticks(layout.x0, layout.x1, most, offset) {
                    labels.push((layout.px(tick.at), tick.label, None));
                }
            }
            _ => {
                let step = n.div_ceil(most).max(1);
                for i in (0..n).rev().step_by(step) {
                    labels.push((layout.px(self.xv(i)), self.x_label(i), Some(i)));
                }
                labels.reverse();
            }
        }
        if let Some(c) = current {
            let at = layout.px(self.xv(c));
            labels.retain(|(p, _, i)| *i != Some(c) && (p - at).abs() >= NOW_ROOM);
        }
        let size = s.text_size(TextSize::Caption);
        let y = crate::text::vcentered_line_y(layout.base + 4.0, 12.0, size);
        let now = current.map(|c| (layout.px(self.xv(c)), "now".to_owned(), Some(c)));
        for (center, text, index) in labels.into_iter().chain(now) {
            let is_now = index.is_some() && index == current;
            let ink = if index.is_some() && index == lit {
                Ink::Value
            } else {
                Ink::Dim
            };
            let mut block = s.mono_block(&text, 0.0, y, TextSize::Caption, ink);
            if is_now {
                block = block.with_color_f32(s.color(StyleKey::Accent));
            }
            let (w, _) = list.measure_block(&block);
            block.x = if area.right() - center < EDGE_ROOM {
                area.right() - w
            } else if center - area.x - layout.gutter < EDGE_ROOM {
                layout.left
            } else {
                center - w * 0.5
            };
            list.text(block);
        }
    }

    /// Point `i`'s x label.
    fn x_label(&self, i: usize) -> String {
        if let Some(label) = self.labels.and_then(|labels| labels.get(i)) {
            return (*label).to_owned();
        }
        let x = self.xv(i);
        match self.x_format {
            Some(format) => format(x),
            None => plain_number(x),
        }
    }

    /// Point `i`'s tooltip title.
    fn long_label(&self, i: usize, layout: &Layout) -> String {
        if let Some(label) = self.long_labels.and_then(|labels| labels.get(i)) {
            return (*label).to_owned();
        }
        match (self.labels, self.time) {
            (Some(_), _) | (None, None) => self.x_label(i),
            (None, Some(offset)) => time_long(self.xv(i), layout.x1 - layout.x0, offset),
        }
    }

    /// Lay the legend's entries out in rows `width` wide, handing each to
    /// `place` with its rect relative to the legend's top-left. Returns the
    /// number of rows.
    fn legend_entries(
        &self,
        width: f32,
        layout: &Layout,
        list: &mut DrawList,
        s: &StyleResolver,
        place: impl FnMut(&mut DrawList, LegendEntry, Rect),
    ) -> usize {
        let has_reference = self.reference.is_some() && layout.any;
        let has_current = self.current_point(layout.n).is_some() && layout.any;
        let entries: Vec<(LegendEntry, f32)> = (0..self.series.len())
            .map(LegendEntry::Series)
            .chain(has_reference.then_some(LegendEntry::Reference))
            .chain(has_current.then_some(LegendEntry::Current))
            .map(|entry| (entry, self.legend_entry_width(entry, list, s)))
            .collect();
        chart::legend_rows(list, width, layout.gutter, entries, place)
    }

    fn legend_entry_width(
        &self,
        entry: LegendEntry,
        list: &mut DrawList,
        s: &StyleResolver,
    ) -> f32 {
        let text = |list: &mut DrawList, text: &str| chart::row_text_width(list, s, text);
        match entry {
            LegendEntry::Series(index) => {
                let series = &self.series[index];
                let mut w = LEGEND_LINE + SWATCH_GAP + text(list, series.name);
                if series.estimated {
                    w += SWATCH_GAP + s.mono_width(list, "EST.", TextSize::Caption);
                }
                w
            }
            LegendEntry::Reference => LEGEND_DASH + SWATCH_GAP + text(list, self.reference_label),
            LegendEntry::Current => LEGEND_CURRENT + SWATCH_GAP + text(list, CURRENT_LABEL),
        }
    }

    fn draw_legend_entry(
        &self,
        entry: LegendEntry,
        r: Rect,
        list: &mut DrawList,
        s: &StyleResolver,
    ) {
        let row = s.text_size(TextSize::Row);
        let text_y = crate::text::vcentered_line_y(r.y, r.height, row);
        let mid = (r.y + r.height * 0.5).round();
        let label = |list: &mut DrawList, text: &str, x: f32, ink: Ink| {
            let block = s.sans_block(text, x, text_y, TextSize::Row, ink);
            let (w, _) = list.measure_block(&block);
            list.text(block);
            w
        };
        match entry {
            LegendEntry::Series(index) => {
                let series = &self.series[index];
                let (color, dashed) = (series.color, series.estimated);
                chart::legend_line(list, r.x, mid, color, dashed, series.dots);
                let ink = if self.hovered_series == Some(index) {
                    Ink::Value
                } else {
                    Ink::Second
                };
                let label_x = r.x + LEGEND_LINE + SWATCH_GAP;
                let w = label(list, series.name, label_x, ink);
                if series.estimated {
                    let caption = s.text_size(TextSize::Caption);
                    list.text(s.mono_block(
                        "EST.",
                        label_x + w + SWATCH_GAP,
                        crate::text::vcentered_line_y(r.y, r.height, caption),
                        TextSize::Caption,
                        Ink::Dim,
                    ));
                }
            }
            LegendEntry::Reference => {
                list.dashed_hline(r.x, mid, LEGEND_DASH, DASH, DASH_GAP, s.ink(Ink::Value));
                label(
                    list,
                    self.reference_label,
                    r.x + LEGEND_DASH + SWATCH_GAP,
                    Ink::Second,
                );
            }
            LegendEntry::Current => {
                let center = (r.x + LEGEND_CURRENT * 0.5, mid);
                let radius = (LEGEND_CURRENT - CURRENT_STROKE) * 0.5;
                list.circle_outline(center, radius, CURRENT_STROKE, s.ink(Ink::Glyph));
                label(
                    list,
                    CURRENT_LABEL,
                    r.x + LEGEND_CURRENT + SWATCH_GAP,
                    Ink::Second,
                );
            }
        }
    }

    /// Paint the tooltip `tip` placed for its point: its title, and each
    /// series' value there, the last series first ("—" where it has none).
    /// `list` should be drawn above everything the tooltip may overlap, and
    /// `viewport` is what it must stay inside.
    pub fn draw_tooltip(
        &self,
        tip: &LineTooltip,
        viewport: Rect,
        list: &mut DrawList,
        s: &StyleResolver,
    ) -> Option<Rect> {
        let n = self.points();
        if tip.index >= n {
            return None;
        }
        let i = tip.index;
        let running = self.current_point(n) == Some(i);
        let (x0, x1) = (self.xv(0), self.xv(n - 1));
        let span_layout = Layout {
            n,
            max: 1.0,
            any: true,
            all_zero: false,
            ticks: Default::default(),
            gutter: 0.0,
            left: 0.0,
            span: 0.0,
            x0,
            x1,
            base: 0.0,
            height: 1.0,
        };
        let title = self.long_label(i, &span_layout);
        let values: Vec<Option<String>> = self
            .series
            .iter()
            .map(|series| series.value(i).map(|v| (self.format)(v)))
            .collect();
        let mono = |list: &mut DrawList, text: &str| s.mono_width(list, text, TextSize::Meta);
        let sans = |list: &mut DrawList, text: &str| chart::row_text_width(list, s, text);
        let label_x = TIP_SWATCH + TIP_SWATCH_GAP;
        let mut content = sans(list, &title)
            + if running {
                TIP_VALUE_GAP + chart::running_width(list, s)
            } else {
                0.0
            };
        for (series, value) in self.series.iter().zip(&values) {
            let mut name = sans(list, series.name);
            if series.estimated {
                name += sans(list, " (est.)");
            }
            let value = value.as_deref().unwrap_or("—");
            content = content.max(label_x + name + TIP_VALUE_GAP + mono(list, value));
        }
        let rows = 1 + self.series.len();
        let content_height =
            rows as f32 * TIP_ROW + (rows - 1) as f32 * TIP_ROW_GAP + TIP_HEADER_GAP;
        let (width, height) = chart::tooltip_size(s, content, content_height, TIP_MIN_WIDTH);
        let x = if tip.right_of_guide {
            tip.guide + TIP_GAP
        } else {
            tip.guide - TIP_GAP - width
        };
        let x = x.min(viewport.right() - width).max(viewport.x).round();
        let y = tip
            .top
            .min(viewport.bottom() - height)
            .max(viewport.y)
            .round();
        let rect = Rect::new(x, y, width, height);

        chart::paint_tooltip(list, s, rect, |list, inner| {
            let (left, right) = (inner.x, inner.right());
            let row_size = s.text_size(TextSize::Row);
            let text_y = |y: f32| crate::text::vcentered_line_y(y, TIP_ROW, row_size);
            let mut y = inner.y;
            list.text(s.sans_block(&title, left, text_y(y), TextSize::Row, Ink::Max));
            if running {
                chart::draw_running(list, s, right, y);
            }
            y += TIP_ROW + TIP_ROW_GAP + TIP_HEADER_GAP;
            for (series, value) in self.series.iter().zip(&values).rev() {
                list.push_tint();
                if value.is_none() {
                    list.multiply_tint([1.0, 1.0, 1.0, TIP_ZERO]);
                }
                let mid = (y + TIP_ROW * 0.5).round();
                chart::line_swatch(list, left, mid, TIP_SWATCH, series.color, series.estimated);
                let label = |text, x| s.sans_block(text, x, text_y(y), TextSize::Row, Ink::Second);
                let block = label(series.name, left + label_x);
                let (w, _) = list.measure_block(&block);
                list.text(block);
                if series.estimated {
                    list.text(label(" (est.)", left + label_x + w));
                }
                let text = value.as_deref().unwrap_or("—");
                chart::draw_tip_value(list, s, text, right, y, Ink::Value, None);
                list.pop_tint();
                y += TIP_ROW + TIP_ROW_GAP;
            }
        });
        Some(rect)
    }
}

/// A pixel column of points being gathered by [`LineChart::reduce`]: its
/// lowest and highest point (index, x and value), whether it has a gap, and
/// the x it reaches.
struct Column {
    at: i64,
    lo: Option<(usize, f64, f64)>,
    hi: Option<(usize, f64, f64)>,
    gap: bool,
    last: f64,
}

impl Column {
    fn new(at: i64) -> Self {
        Self {
            at,
            lo: None,
            hi: None,
            gap: false,
            last: 0.0,
        }
    }

    fn add(&mut self, i: usize, x: f64, value: Option<f64>) {
        self.last = x;
        let Some(v) = value else {
            self.gap = true;
            return;
        };
        if self.lo.is_none_or(|(_, _, lo)| v < lo) {
            self.lo = Some((i, x, v));
        }
        if self.hi.is_none_or(|(_, _, hi)| v > hi) {
            self.hi = Some((i, x, v));
        }
    }

    /// Its lowest and highest point in the order they come, then a break
    /// after it if it had a gap.
    fn flush(&self, out: &mut Vec<(f64, Option<f64>)>) {
        if let (Some(lo), Some(hi)) = (self.lo, self.hi) {
            let (a, b) = if lo.0 <= hi.0 { (lo, hi) } else { (hi, lo) };
            out.push((a.1, Some(a.2)));
            if b.0 != a.0 {
                out.push((b.1, Some(b.2)));
            }
        }
        if self.gap {
            out.push((self.last, None));
        }
    }
}

/// An entry of the legend: a series, the reference line, the running point.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LegendEntry {
    Series(usize),
    Reference,
    Current,
}

/// `x` written plainly: whole numbers without a fraction.
fn plain_number(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

#[cfg(test)]
#[path = "line_chart_tests.rs"]
mod tests;
