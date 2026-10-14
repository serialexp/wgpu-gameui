//! Bar chart — stacked vertical bars over a few gridlines, with an x label
//! under every so many bars, a legend, and a tooltip for the bar under the
//! pointer (Forge's `BarChart`: usage per day or per rate-limit window).
//!
//! Each bar stacks one value per series, bottom up. A bar may carry a
//! reference value, drawn as a dashed line across it (an estimated cap), and
//! may be the current one, hatched and ticked "now" (a window still
//! running). A series may be estimated rather than measured, drawn as a
//! faint fill inside a full-colour outline.
//!
//! The axis runs from zero to a "nice" maximum that divides into thirds
//! ([`nice_max`]). A lone outlier — over three times the next tallest bar —
//! would flatten every other bar, so the axis is set by the next tallest
//! instead and the outlier is cut with a break and labelled with its value.
//! Segment edges snap to whole pixels, and a bar with anything in it is
//! never under 2px.
//!
//! Past a bar every 2px, neighbouring bars share a slot, which shows the
//! tallest of them (or the outlier, or the running one, or the one lit from
//! a table): what is drawn stops growing with the bar count. Bars under 4px
//! wide sit on whole pixels, drop their lit top edge, and their reference is
//! a plain tick rather than an edged dash.
//!
//! An overlay ([`BarChart::overlay`]) adds one more value a bar on its own
//! axis to the right, in the same thirds: a line with dots through the bar
//! centres, broken where a bar has no value. Its legend entry fades the
//! bars, and a series' fades it.
//!
//! Like [`Waffle`](super::Waffle), the widget only paints and reports what
//! the pointer is over: the caller feeds the hovered bar back (and may share
//! it with a table of the same rows), and paints the tooltip last, above
//! whatever the chart sits among ([`BarChart::draw_tooltip`]).

use crate::InputState;
use crate::layout::Rect;
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};

use super::DrawList;
use super::chart::{
    self, AxisSide, CHART_HEIGHT, DASH, DASH_GAP, DOT, DOT_RING, DOT_SPACING, EDGE_ROOM,
    LABEL_SPACING, LEGEND_DASH, LEGEND_GAP, LEGEND_LINE, NOW_TICK, PAD, SERIES_FADED, SWATCH_GAP,
    TIP_PAD_X, TIP_ROW, TIP_ROW_GAP, TIP_SWATCH_GAP, TIP_VALUE_GAP, TIP_ZERO, X_LABELS, nice_max,
};

/// The widest a bar gets when the caller doesn't say.
const MAX_BAR_WIDTH: f32 = 56.0;
/// More room over the plot for an outlier's value.
const OUTLIER_ROOM: f32 = 4.0;
/// The gap between bars, and the narrower one once there are many.
const BAR_GAP: f32 = 3.0;
const BAR_GAP_CROWDED: f32 = 1.0;
const CROWDED: usize = 40;
/// The shortest bar with anything in it.
const MIN_BAR: f32 = 2.0;
/// The narrowest slot: past a bar this often, neighbours share one.
const MIN_SLOT: f32 = 2.0;
/// Bars narrower than this skip their top edge and cap with a plain tick.
const NARROW: f32 = 4.0;
/// An outlier is over this many times the next tallest bar.
const OUTLIER_RATIO: f64 = 3.0;
/// With an outlier, the axis reaches this far over the next tallest.
const OUTLIER_HEADROOM: f64 = 1.15;
/// The outlier's break: a gap this far below its top, this tall.
const BREAK_AT: f32 = 8.0;
const BREAK: f32 = 3.0;
/// The other bars while one is hovered.
const DIMMED: f32 = 0.45;
/// An estimated series' fill, over the ground; its outline is full colour.
const ESTIMATED_FILL: f32 = 0.4;
/// A bar's lit top edge.
const BAR_HI: [f32; 4] = [1.0, 1.0, 1.0, 0.22];
/// The current bar's hatch.
const HATCH: [f32; 4] = [0.0, 0.0, 0.0, 0.45];
const HATCH_STEP: f32 = 4.0;
/// The reference line, at this strength, edged dark.
const REFERENCE: f32 = 0.85;
const REFERENCE_EDGE: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
/// The legend's square swatch.
const LEGEND_SWATCH: f32 = 8.0;
/// The tooltip: beside the bar, its rows of series.
const TIP_GAP: f32 = 8.0;
const TIP_MIN_WIDTH: f32 = 160.0;
const TIP_SWATCH: f32 = 7.0;
/// The reference's and the overlay's swatch in the tooltip: a short line.
const TIP_LINE: f32 = 10.0;
const TIP_RULE: [f32; 4] = [0.0, 0.0, 0.0, 0.5];
const TIP_RULE_LIT: [f32; 4] = [1.0, 1.0, 1.0, 0.06];
/// The overlay: its line's halo, its axis labels' strength, and what its
/// legend entry says.
const OVERLAY_HALO: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
const OVERLAY_LABELS: f32 = 0.85;
const RIGHT_AXIS: &str = "RIGHT AXIS";

/// One series of a [`BarChart`]: a layer of every bar, and a legend entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarSeries<'a> {
    /// Its legend entry and tooltip row.
    pub name: &'a str,
    /// Categorical, e.g. `oklch(0.68 0.1 <hue>)`; one series may be neutral
    /// ink.
    pub color: [f32; 4],
    /// Estimated rather than measured: a faint fill in a full outline, and
    /// "est." beside its name.
    pub estimated: bool,
}

/// One bar of a [`BarChart`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar<'a> {
    /// Under the bar, when there is room ("09-21").
    pub label: &'a str,
    /// In its tooltip ("Sun 21 Sep").
    pub long_label: &'a str,
    /// One value per series, bottom up; a missing one is zero.
    pub segments: &'a [f64],
    /// A dashed line across the bar at this value (an estimated cap).
    pub reference: Option<f64>,
    /// Still running: hatched, and ticked "now".
    pub current: bool,
}

impl Bar<'_> {
    fn total(&self) -> f64 {
        self.segments.iter().map(|v| v.max(0.0)).sum()
    }

    /// How tall the axis must reach for this bar: its stack or its
    /// reference, whichever is higher.
    fn peak(&self) -> f64 {
        self.total().max(self.reference.unwrap_or(0.0))
    }
}

/// A line over a [`BarChart`]'s bars, one value a bar, on its own axis to
/// the right (a rate beside the counts, say).
#[derive(Clone, Copy)]
pub struct BarOverlay<'a> {
    /// Its legend entry and tooltip row.
    pub label: &'a str,
    /// Its line, dots and axis labels.
    pub color: [f32; 4],
    /// One per bar; `None` (or a value that isn't finite) breaks the line.
    pub values: &'a [Option<f64>],
    /// The axis maximum; a nice one over the values when `None`.
    pub max: Option<f64>,
    /// How a value is written on the axis and in the tooltip.
    pub format: &'a dyn Fn(f64) -> String,
}

impl BarOverlay<'_> {
    /// The value at bar `i`, if there is one.
    fn value(&self, i: usize) -> Option<f64> {
        self.values
            .get(i)
            .copied()
            .flatten()
            .filter(|v| v.is_finite())
    }
}

/// What a [`BarChart`] did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BarChartOutput {
    /// The bar under the pointer, if any.
    pub hovered: Option<usize>,
    /// The series whose legend entry is under the pointer, if any.
    pub hovered_series: Option<usize>,
    /// Whether the overlay's legend entry is under the pointer.
    pub hovered_overlay: bool,
    /// The height it took, legend included.
    pub height: f32,
    /// Where the hovered bar's tooltip goes, for
    /// [`BarChart::draw_tooltip`]. Only while the pointer itself is over a
    /// bar: a bar lit from a table row shows no tooltip.
    pub tooltip: Option<BarTooltip>,
}

/// Where a bar's tooltip goes: beside the bar, on the side with more room.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarTooltip {
    /// The bar's index.
    pub index: usize,
    /// The bar's left edge.
    pub bar_left: f32,
    /// The bar's right edge.
    pub bar_right: f32,
    /// Whether the tooltip goes right of the bar (it is in the left half).
    pub right_of_bar: bool,
    /// The chart's top, where the tooltip's top goes.
    pub top: f32,
}

/// The layout every part of the chart shares, worked out once a frame.
struct Layout {
    /// The axis maximum.
    max: f64,
    all_zero: bool,
    /// Room over the plot, with more for an outlier's value.
    top: f32,
    /// The y labels' column, left of the plot, and the overlay's, right of
    /// it (none without one).
    gutter: f32,
    right_gutter: f32,
    /// The overlay's axis maximum and labels, bottom up.
    overlay_max: f64,
    overlay_ticks: [String; 4],
    /// Each slot's share of the plot's width, and the bar's own width.
    slot: f32,
    bar_width: f32,
    /// Bars narrower than [`NARROW`], set on whole pixels.
    snap: bool,
    /// How many bars there are, and how many slots they are drawn in.
    bars: usize,
    slots: usize,
    /// The bar cut short by the axis, if any.
    outlier: Option<usize>,
    /// The bar each slot shows, once bars share slots ([`BarChart::group`]);
    /// a slot a bar otherwise.
    shown: Option<Vec<usize>>,
    /// The y labels, bottom up.
    ticks: [String; 4],
}

impl Layout {
    /// The bar slot `k` shows.
    fn shown(&self, k: usize) -> usize {
        self.shown.as_ref().map_or(k, |shown| shown[k])
    }

    /// The bar slot `k` draws: the lit bar when it falls in that slot (a
    /// table row may light a bar its slot doesn't show), else the one the
    /// slot shows.
    fn drawn(&self, k: usize, lit: Option<usize>) -> usize {
        match lit {
            Some(lit) if self.slot_of(lit) == k => lit,
            _ => self.shown(k),
        }
    }

    /// The slot bar `i` is drawn in.
    fn slot_of(&self, i: usize) -> usize {
        if self.slots < self.bars {
            i * self.slots / self.bars
        } else {
            i
        }
    }

    /// Where slot `k`'s centre is, from the plot's left edge.
    fn center(&self, k: usize) -> f32 {
        (k as f32 + 0.5) * self.slot
    }

    /// The left edge of slot `k`'s bar, with the plot's left edge at
    /// `origin`: on a whole pixel once bars are narrow, where a fraction
    /// would smear a bar across two columns.
    fn bar_left(&self, origin: f32, k: usize) -> f32 {
        let left = origin + self.center(k) - self.bar_width * 0.5;
        if self.snap { left.round() } else { left }
    }
}

/// How a bar ranks for its slot ([`BarChart::group`]): outlier, running,
/// stack, reference, compared in that order.
type SlotRank = (bool, bool, f64, f64);

/// A stacked bar chart.
#[derive(Clone, Copy)]
pub struct BarChart<'a> {
    bars: &'a [Bar<'a>],
    series: &'a [BarSeries<'a>],
    format: &'a dyn Fn(f64) -> String,
    height: f32,
    max_bar_width: f32,
    hovered: Option<usize>,
    hovered_series: Option<usize>,
    overlay: Option<BarOverlay<'a>>,
    hovered_overlay: bool,
    legend: bool,
    reference_label: &'a str,
    empty_label: &'a str,
    zero_label: &'a str,
}

impl<'a> BarChart<'a> {
    /// A chart of `bars` stacking `series`, its values written by `format`
    /// (`1.2M`) on the axis and in the tooltip.
    pub fn new(
        bars: &'a [Bar<'a>],
        series: &'a [BarSeries<'a>],
        format: &'a dyn Fn(f64) -> String,
    ) -> Self {
        Self {
            bars,
            series,
            format,
            height: CHART_HEIGHT,
            max_bar_width: MAX_BAR_WIDTH,
            hovered: None,
            hovered_series: None,
            overlay: None,
            hovered_overlay: false,
            legend: true,
            reference_label: "cap (estimated)",
            empty_label: "No usage in this range",
            zero_label: "No usage in any bucket",
        }
    }

    /// The plot's height.
    pub fn height(mut self, height: f32) -> Self {
        self.height = height.max(1.0);
        self
    }

    /// The widest a bar gets.
    pub fn max_bar_width(mut self, width: f32) -> Self {
        self.max_bar_width = width.max(1.0);
        self
    }

    /// The bar to light: last frame's [`BarChartOutput::hovered`], or the
    /// row hovered in a table of the same bars.
    pub fn hovered(mut self, hovered: Option<usize>) -> Self {
        self.hovered = hovered;
        self
    }

    /// The series to light: last frame's
    /// [`BarChartOutput::hovered_series`].
    pub fn hovered_series(mut self, hovered: Option<usize>) -> Self {
        self.hovered_series = hovered;
        self
    }

    /// A line over the bars on its own axis to the right.
    pub fn overlay(mut self, overlay: BarOverlay<'a>) -> Self {
        self.overlay = Some(overlay);
        self
    }

    /// Whether to light the overlay: last frame's
    /// [`BarChartOutput::hovered_overlay`].
    pub fn hovered_overlay(mut self, hovered: bool) -> Self {
        self.hovered_overlay = hovered;
        self
    }

    /// Whether to draw the legend under the plot.
    pub fn legend(mut self, legend: bool) -> Self {
        self.legend = legend;
        self
    }

    /// What the reference line is called in the legend and tooltip.
    pub fn reference_label(mut self, label: &'a str) -> Self {
        self.reference_label = label;
        self
    }

    /// What the plot says with no bars, and with bars that are all zero.
    pub fn empty_labels(mut self, empty: &'a str, zero: &'a str) -> Self {
        self.empty_label = empty;
        self.zero_label = zero;
        self
    }

    /// The overlay, unless there are no bars for it to run over.
    fn shown_overlay(&self) -> Option<&BarOverlay<'a>> {
        self.overlay.as_ref().filter(|_| !self.bars.is_empty())
    }

    fn layout(&self, width: f32, list: &mut DrawList, s: &StyleResolver) -> Layout {
        // The two tallest bars, for the outlier: no sort of every bar.
        let (mut first, mut second, mut tallest) = (0.0_f64, 0.0_f64, 0);
        for (i, bar) in self.bars.iter().enumerate() {
            let peak = bar.peak();
            if peak > first {
                second = first;
                first = peak;
                tallest = i;
            } else if peak > second {
                second = peak;
            }
        }
        let n = self.bars.len();
        let outlier = n >= 4 && second > 0.0 && first > OUTLIER_RATIO * second;
        let all_zero = n > 0 && first <= 0.0;
        let max = nice_max(if outlier {
            second * OUTLIER_HEADROOM
        } else {
            first
        });
        let ticks = chart::axis_ticks(max, all_zero, self.format);
        let gutter = if n == 0 {
            0.0
        } else {
            chart::gutter(&ticks, list, s)
        };
        let (overlay_max, overlay_ticks, right_gutter) = match self.shown_overlay() {
            Some(overlay) => {
                let max = overlay.max.filter(|max| *max > 0.0).unwrap_or_else(|| {
                    let most = (0..n).filter_map(|i| overlay.value(i)).fold(0.0, f64::max);
                    nice_max(most)
                });
                let ticks = chart::axis_ticks(max, false, overlay.format);
                let gutter = chart::gutter(&ticks, list, s);
                (max, ticks, gutter)
            }
            None => (1.0, Default::default(), 0.0),
        };
        let plot = (width - gutter - right_gutter).max(0.0);
        let slots = n.min(((plot / MIN_SLOT).floor() as usize).max(1));
        let slot = if n == 0 { 0.0 } else { plot / slots as f32 };
        let gap = if slots > CROWDED {
            BAR_GAP_CROWDED
        } else {
            BAR_GAP
        };
        let bar_width = (slot - gap).min(self.max_bar_width).max(1.0);
        // Snapped bars are floored, so a whole-pixel gap is left between them.
        let snap = bar_width < NARROW;
        Layout {
            max,
            all_zero,
            top: PAD + if outlier { OUTLIER_ROOM } else { 0.0 },
            gutter,
            right_gutter,
            overlay_max,
            overlay_ticks,
            slot,
            bar_width: if snap { bar_width.floor() } else { bar_width },
            snap,
            bars: n,
            slots,
            outlier: outlier.then_some(tallest),
            shown: None,
            ticks,
        }
    }

    /// The bar each slot shows, the bars split evenly between the slots in
    /// order, or `None` with a slot a bar. A slot shows its outlier (the axis
    /// keeps room for its value), else its running bar, else its tallest
    /// stack, a higher reference breaking a tie; the first of equals.
    fn group(&self, layout: &Layout) -> Option<Vec<usize>> {
        let (n, slots) = (layout.bars, layout.slots);
        if slots >= n {
            return None;
        }
        let rank = |i: usize, bar: &Bar<'_>| -> SlotRank {
            (
                layout.outlier == Some(i),
                bar.current,
                bar.total(),
                bar.reference.unwrap_or(0.0),
            )
        };
        let mut best: Vec<Option<(usize, SlotRank)>> = vec![None; slots];
        for (i, bar) in self.bars.iter().enumerate() {
            let held = &mut best[i * slots / n];
            let candidate = rank(i, bar);
            let replace = held.is_none_or(|(_, kept)| {
                (candidate.0, candidate.1)
                    .cmp(&(kept.0, kept.1))
                    .then(candidate.2.total_cmp(&kept.2))
                    .then(candidate.3.total_cmp(&kept.3))
                    .is_gt()
            });
            if replace {
                *held = Some((i, candidate));
            }
        }
        Some(
            best.into_iter()
                .map(|held| held.map_or(0, |(i, _)| i))
                .collect(),
        )
    }

    /// The bar under `(mx, my)`, with the chart's plot area at `area`.
    fn hit(&self, area: Rect, layout: &Layout, input: &InputState) -> Option<usize> {
        if input.mouse_consumed || layout.slot <= 0.0 {
            return None;
        }
        let plot = Rect::new(
            area.x + layout.gutter,
            area.y,
            area.width - layout.gutter - layout.right_gutter,
            area.height,
        );
        if !plot.contains(input.mouse_x, input.mouse_y) {
            return None;
        }
        let k = ((input.mouse_x - plot.x) / layout.slot) as usize;
        (k < layout.slots).then(|| layout.shown(k))
    }

    /// The height it takes `width` wide, legend included.
    pub fn measure_height(&self, width: f32, list: &mut DrawList, s: &StyleResolver) -> f32 {
        let layout = self.layout(width, list, s);
        let mut height = layout.top + self.height + X_LABELS;
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
    ) -> BarChartOutput {
        let mut layout = self.layout(width, list, s);
        layout.shown = self.group(&layout);
        let area = Rect::new(x, y, width, layout.top + self.height + X_LABELS);
        let hit = self.hit(area, &layout, input);
        let lit = hit.or(self.hovered).filter(|&i| i < self.bars.len());
        let lit_slot = lit.map(|i| layout.slot_of(i));
        let base = area.bottom() - X_LABELS;
        let n = self.bars.len();

        self.draw_grid(area, base, &layout, list, s);
        if n == 0 || layout.all_zero {
            let caption = if n == 0 {
                self.empty_label
            } else {
                self.zero_label
            };
            let plot = Rect::new(
                x + layout.gutter,
                base - self.height,
                width - layout.gutter,
                self.height,
            );
            chart::draw_empty_caption(list, s, plot, caption);
        }

        let scale = self.height as f64 / layout.max;
        for k in 0..layout.slots {
            let bar = &self.bars[layout.drawn(k, lit)];
            let left = layout.bar_left(x + layout.gutter, k);
            let dim = lit_slot.is_some_and(|lit| lit != k);
            list.push_tint();
            if dim {
                list.multiply_tint([1.0, 1.0, 1.0, DIMMED]);
            }
            self.draw_bar(bar, left, base, scale, &layout, list, s);
            list.pop_tint();
        }
        if let Some(overlay) = self.shown_overlay() {
            self.draw_overlay(overlay, x + layout.gutter, base, &layout, lit, list, s);
        }
        self.draw_x_labels(area, base, &layout, lit, list, s);

        let (mut hovered_series, mut hovered_overlay) = (None, false);
        let mut height = area.height;
        if self.legend && !self.series.is_empty() {
            let legend_y = area.bottom() + LEGEND_GAP;
            let rows = self.legend_entries(width, &layout, list, s, |list, entry, r| {
                let r = Rect::new(x + r.x, legend_y + r.y, r.width, r.height);
                if !input.mouse_consumed && r.contains(input.mouse_x, input.mouse_y) {
                    match entry {
                        LegendEntry::Series(index) => hovered_series = Some(index),
                        LegendEntry::Overlay => hovered_overlay = true,
                        LegendEntry::Reference | LegendEntry::Current => {}
                    }
                }
                self.draw_legend_entry(entry, r, list, s);
            });
            height += LEGEND_GAP + chart::legend_height(rows);
        }

        let tooltip = hit.map(|index| {
            let bar_left = layout.bar_left(x + layout.gutter, layout.slot_of(index));
            let bar_right = bar_left + layout.bar_width;
            BarTooltip {
                index,
                bar_left,
                bar_right,
                right_of_bar: (bar_left + bar_right) * 0.5 < x + width * 0.5,
                top: y,
            }
        });
        BarChartOutput {
            hovered: hit,
            hovered_series,
            hovered_overlay,
            height,
            tooltip,
        }
    }

    fn draw_grid(
        &self,
        area: Rect,
        base: f32,
        layout: &Layout,
        list: &mut DrawList,
        s: &StyleResolver,
    ) {
        let left = area.x + layout.gutter;
        let right = area.right() - layout.right_gutter;
        let plot = Rect::new(left, base - self.height, right - left, self.height);
        chart::draw_gridlines(list, plot);
        if !self.bars.is_empty() {
            chart::draw_axis_labels(list, s, plot, AxisSide::Left, &layout.ticks, None);
        }
        if let Some(overlay) = self.shown_overlay() {
            let mut color = overlay.color;
            color[3] *= OVERLAY_LABELS;
            let ticks = &layout.overlay_ticks;
            chart::draw_axis_labels(list, s, plot, AxisSide::Right, ticks, Some(color));
        }
    }

    /// The overlay's line through the bar centres from `origin` (the plot's
    /// left edge), broken where a bar has no value; a dot on each point
    /// while the slots are wide enough, and the lit bar's ringed. Faded
    /// while a series is lit.
    #[allow(clippy::too_many_arguments)]
    fn draw_overlay(
        &self,
        overlay: &BarOverlay<'_>,
        origin: f32,
        base: f32,
        layout: &Layout,
        lit: Option<usize>,
        list: &mut DrawList,
        s: &StyleResolver,
    ) {
        let points: Vec<Option<[f32; 2]>> = (0..layout.slots)
            .map(|k| {
                let value = overlay.value(layout.drawn(k, lit))?;
                let t = (value / layout.overlay_max).clamp(0.0, 1.0) as f32;
                Some([origin + layout.center(k), base - t * self.height])
            })
            .collect();
        list.push_tint();
        if self.hovered_series.is_some() {
            list.multiply_tint([1.0, 1.0, 1.0, SERIES_FADED]);
        }
        let color = overlay.color;
        for run in points.split(Option::is_none) {
            if run.len() > 1 {
                let run: Vec<[f32; 2]> = run.iter().flatten().copied().collect();
                chart::stroke_with_halo(list, &run, OVERLAY_HALO, color, None);
            }
        }
        let lit_slot = lit.map(|i| layout.slot_of(i));
        for (k, point) in points.iter().enumerate() {
            let Some([px, py]) = *point else { continue };
            if lit_slot == Some(k) {
                chart::hovered_dot(list, s, (px, py), color);
            } else if layout.slot >= DOT_SPACING {
                chart::dot(list, (px, py), DOT, color, DOT_RING);
            }
        }
        list.pop_tint();
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_bar(
        &self,
        bar: &Bar<'_>,
        left: f32,
        base: f32,
        scale: f64,
        layout: &Layout,
        list: &mut DrawList,
        s: &StyleResolver,
    ) {
        let width = layout.bar_width;
        let total = bar.total();
        let plot = self.height;
        let clipped = total * scale > f64::from(plot) + 0.5;
        let fit = if clipped {
            f64::from(plot) / total
        } else {
            scale
        };
        let top_px = (total * fit).round() as f32;
        if total > 0.0 {
            let height = top_px.max(MIN_BAR);
            let rect = Rect::new(left, base - height, width, height);
            if clipped {
                // The break: the bar in two parts with a gap near its top.
                let above = Rect::new(left, rect.y, width, BREAK_AT);
                let below = Rect::new(
                    left,
                    rect.y + BREAK_AT + BREAK,
                    width,
                    height - BREAK_AT - BREAK,
                );
                for part in [above, below] {
                    list.push_clip(part);
                    self.paint_stack(bar, rect, fit, top_px < MIN_BAR, list);
                    list.pop_clip();
                }
                let label = format!("▲ {}", (self.format)(total));
                let mut block = s.mono_block(&label, 0.0, 0.0, TextSize::Caption, Ink::Value);
                let (w, _) = list.measure_block(&block);
                block.x = left + width * 0.5 - w * 0.5;
                block.y = base - plot - layout.top;
                list.text(block);
            } else {
                self.paint_stack(bar, rect, fit, top_px < MIN_BAR, list);
            }
        } else {
            list.quad(left, base - 1.0, width, 1.0, s.ink(Ink::Dim));
        }
        if let Some(reference) = bar.reference {
            let at = (reference * scale).round().min(f64::from(plot)) as f32;
            let line_y = base - at;
            let mut ink = s.ink(Ink::Value);
            ink[3] *= REFERENCE;
            if width < NARROW {
                list.quad(left, line_y, width, 1.0, ink);
            } else {
                list.quad(left - 2.0, line_y - 1.0, width + 4.0, 1.0, REFERENCE_EDGE);
                list.quad(left - 2.0, line_y + 1.0, width + 4.0, 1.0, REFERENCE_EDGE);
                list.dashed_hline(left - 2.0, line_y, width + 4.0, DASH, DASH_GAP, ink);
            }
        }
        if bar.current {
            chart::draw_now_tick(list, s, Rect::new(left, base + 1.0, width, NOW_TICK));
        }
    }

    /// Paint `bar`'s segments into `rect` (its whole height), `fit` pixels a
    /// unit; a bar too short to show (`floor`) is its largest series alone.
    fn paint_stack(&self, bar: &Bar<'_>, rect: Rect, fit: f64, floor: bool, list: &mut DrawList) {
        let base = rect.bottom();
        if floor {
            let largest = bar
                .segments
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map_or(0, |(index, _)| index);
            let floor = Rect::new(rect.x, base - MIN_BAR, rect.width, MIN_BAR);
            self.paint_segment(largest, floor, list);
        } else {
            let (mut sum, mut below) = (0.0, 0.0_f32);
            for (index, value) in bar.segments.iter().enumerate() {
                sum += value.max(0.0);
                let top = (sum * fit).round() as f32;
                if top > below {
                    let segment = Rect::new(rect.x, base - top, rect.width, top - below);
                    self.paint_segment(index, segment, list);
                }
                below = top.max(below);
            }
        }
        if bar.current {
            list.hatch(rect, HATCH_STEP, HATCH);
        }
        if rect.width >= NARROW {
            list.quad(rect.x, rect.y, rect.width, 1.0, BAR_HI);
        }
    }

    fn paint_segment(&self, index: usize, rect: Rect, list: &mut DrawList) {
        let Some(series) = self.series.get(index) else {
            return;
        };
        let faded = self.hovered_overlay || self.hovered_series.is_some_and(|lit| lit != index);
        list.push_tint();
        if faded {
            list.multiply_tint([1.0, 1.0, 1.0, SERIES_FADED]);
        }
        paint_fill(list, rect, series);
        list.pop_tint();
    }

    fn draw_x_labels(
        &self,
        area: Rect,
        base: f32,
        layout: &Layout,
        lit: Option<usize>,
        list: &mut DrawList,
        s: &StyleResolver,
    ) {
        let n = layout.slots;
        if n == 0 {
            return;
        }
        let plot = area.width - layout.gutter - layout.right_gutter;
        let fits = ((plot / LABEL_SPACING).floor() as usize).max(2);
        let step = n.div_ceil(fits).max(1);
        let size = s.text_size(TextSize::Caption);
        let y = crate::text::vcentered_line_y(base + 4.0, 12.0, size);
        for k in 0..n {
            if !(n - 1 - k).is_multiple_of(step) {
                continue;
            }
            let index = layout.drawn(k, lit);
            let bar = &self.bars[index];
            let center = area.x + layout.gutter + layout.center(k);
            let (text, ink) = if bar.current {
                ("now", None)
            } else if lit == Some(index) {
                (bar.label, Some(Ink::Value))
            } else {
                (bar.label, Some(Ink::Dim))
            };
            let mut block = s.mono_block(text, 0.0, y, TextSize::Caption, ink.unwrap_or(Ink::Dim));
            if ink.is_none() {
                block = block.with_color_f32(s.color(StyleKey::Accent));
            }
            let (w, _) = list.measure_block(&block);
            let half = layout.bar_width * 0.5;
            block.x = if area.right() - layout.right_gutter - center < EDGE_ROOM {
                center + half - w
            } else if center - (area.x + layout.gutter) < EDGE_ROOM {
                (center - half).max(area.x + layout.gutter)
            } else {
                center - w * 0.5
            };
            list.text(block);
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
        let has_reference = self.bars.iter().any(|bar| bar.reference.is_some());
        let has_current = self.bars.iter().any(|bar| bar.current);
        let has_overlay = self.shown_overlay().is_some();
        let entries: Vec<(LegendEntry, f32)> = (0..self.series.len())
            .map(LegendEntry::Series)
            .chain(has_overlay.then_some(LegendEntry::Overlay))
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
                let mut w = LEGEND_SWATCH + SWATCH_GAP + text(list, series.name);
                if series.estimated {
                    w += SWATCH_GAP + s.mono_width(list, "EST.", TextSize::Caption);
                }
                w
            }
            LegendEntry::Overlay => {
                let label = self.overlay.as_ref().map_or("", |overlay| overlay.label);
                LEGEND_LINE
                    + SWATCH_GAP
                    + text(list, label)
                    + SWATCH_GAP
                    + s.mono_width(list, RIGHT_AXIS, TextSize::Caption)
            }
            LegendEntry::Reference => LEGEND_DASH + SWATCH_GAP + text(list, self.reference_label),
            LegendEntry::Current => LEGEND_SWATCH + SWATCH_GAP + text(list, "current window"),
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
        let swatch = Rect::new(
            r.x,
            (r.y + (r.height - LEGEND_SWATCH) * 0.5).round(),
            LEGEND_SWATCH,
            LEGEND_SWATCH,
        );
        let label_x = r.x + LEGEND_SWATCH + SWATCH_GAP;
        match entry {
            LegendEntry::Series(index) => {
                let series = &self.series[index];
                paint_fill(list, swatch, series);
                let ink = if self.hovered_series == Some(index) {
                    Ink::Value
                } else {
                    Ink::Second
                };
                let block = s.sans_block(series.name, label_x, text_y, TextSize::Row, ink);
                let (w, _) = list.measure_block(&block);
                list.text(block);
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
            LegendEntry::Overlay => {
                let Some(overlay) = &self.overlay else { return };
                let mid = (r.y + r.height * 0.5).round();
                chart::legend_line(list, r.x, mid, overlay.color, false, true);
                let ink = if self.hovered_overlay {
                    Ink::Value
                } else {
                    Ink::Second
                };
                let label_x = r.x + LEGEND_LINE + SWATCH_GAP;
                let block = s.sans_block(overlay.label, label_x, text_y, TextSize::Row, ink);
                let (w, _) = list.measure_block(&block);
                list.text(block);
                let caption = s.text_size(TextSize::Caption);
                list.text(s.mono_block(
                    RIGHT_AXIS,
                    label_x + w + SWATCH_GAP,
                    crate::text::vcentered_line_y(r.y, r.height, caption),
                    TextSize::Caption,
                    Ink::Dim,
                ));
            }
            LegendEntry::Reference => {
                let mid = (r.y + r.height * 0.5).round();
                list.dashed_hline(r.x, mid, LEGEND_DASH, DASH, DASH_GAP, s.ink(Ink::Value));
                list.text(s.sans_block(
                    self.reference_label,
                    r.x + LEGEND_DASH + SWATCH_GAP,
                    text_y,
                    TextSize::Row,
                    Ink::Second,
                ));
            }
            LegendEntry::Current => {
                let (x, y) = (swatch.x, swatch.y);
                list.quad(x, y, swatch.width, swatch.height, s.ink(Ink::Glyph));
                list.hatch(swatch, HATCH_STEP, HATCH);
                let label = "current window";
                list.text(s.sans_block(label, label_x, text_y, TextSize::Row, Ink::Second));
            }
        }
    }

    /// Paint the tooltip `output` placed for its bar: the bar's long label,
    /// each series' value top down, the total, and the reference with the
    /// share of it the total is. `list` should be drawn above everything the
    /// tooltip may overlap, and `viewport` is what it must stay inside.
    pub fn draw_tooltip(
        &self,
        tip: &BarTooltip,
        viewport: Rect,
        list: &mut DrawList,
        s: &StyleResolver,
    ) -> Option<Rect> {
        let bar = self.bars.get(tip.index)?;
        let total = bar.total();
        let mono = |list: &mut DrawList, text: &str| s.mono_width(list, text, TextSize::Meta);
        let sans = |list: &mut DrawList, text: &str| chart::row_text_width(list, s, text);

        // Rows: the title, each series, a rule, the total, the reference.
        let values: Vec<String> = (0..self.series.len())
            .map(|index| (self.format)(bar.segments.get(index).copied().unwrap_or(0.0)))
            .collect();
        let total_text = (self.format)(total);
        let reference_text = bar.reference.map(|reference| {
            let share = if reference > 0.0 {
                (total / reference * 100.0).round()
            } else {
                0.0
            };
            format!("{} · {share}%", (self.format)(reference))
        });
        let overlay = self.overlay.as_ref().map(|overlay| {
            let value = overlay.value(tip.index);
            (
                overlay,
                value.map_or_else(|| "—".to_owned(), overlay.format),
            )
        });
        // Labels follow their swatch: a square for a series, a short line for
        // the reference and the overlay.
        let label_x = TIP_SWATCH + TIP_SWATCH_GAP;
        let line_label_x = TIP_LINE + TIP_SWATCH_GAP;
        let mut content = sans(list, bar.long_label)
            + if bar.current {
                TIP_VALUE_GAP + chart::running_width(list, s)
            } else {
                0.0
            };
        for (series, value) in self.series.iter().zip(&values) {
            let name = sans(list, series.name)
                + if series.estimated {
                    sans(list, " (est.)")
                } else {
                    0.0
                };
            content = content.max(label_x + name + TIP_VALUE_GAP + mono(list, value));
        }
        content = content.max(sans(list, "Total") + TIP_VALUE_GAP + mono(list, &total_text));
        let mut line_row = |list: &mut DrawList, label: &str, value: &str| {
            content =
                content.max(line_label_x + sans(list, label) + TIP_VALUE_GAP + mono(list, value));
        };
        if let Some(reference) = &reference_text {
            line_row(list, self.reference_label, reference);
        }
        if let Some((overlay, value)) = &overlay {
            line_row(list, overlay.label, value);
        }
        let rows = 2
            + self.series.len()
            + usize::from(reference_text.is_some())
            + usize::from(overlay.is_some());
        let rule = TIP_ROW_GAP * 2.0 + 1.0;
        let content_height = rows as f32 * TIP_ROW + (rows - 1) as f32 * TIP_ROW_GAP + rule;
        let (width, height) = chart::tooltip_size(s, content, content_height, TIP_MIN_WIDTH);
        let x = if tip.right_of_bar {
            tip.bar_right + TIP_GAP
        } else {
            tip.bar_left - TIP_GAP - width
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
            let mut y = inner.y;
            let text_y = |y: f32| crate::text::vcentered_line_y(y, TIP_ROW, row_size);
            let value = |list: &mut DrawList, text: &str, y: f32, ink: Ink| {
                chart::draw_tip_value(list, s, text, right, y, ink, None);
            };

            list.text(s.sans_block(bar.long_label, left, text_y(y), TextSize::Row, Ink::Max));
            if bar.current {
                chart::draw_running(list, s, right, y);
            }
            y += TIP_ROW + TIP_ROW_GAP;
            for (index, series) in self.series.iter().enumerate().rev() {
                let zero = bar.segments.get(index).copied().unwrap_or(0.0) == 0.0;
                list.push_tint();
                if zero {
                    list.multiply_tint([1.0, 1.0, 1.0, TIP_ZERO]);
                }
                let swatch = Rect::new(
                    left,
                    (y + (TIP_ROW - TIP_SWATCH) * 0.5).round(),
                    TIP_SWATCH,
                    TIP_SWATCH,
                );
                paint_fill(list, swatch, series);
                let label = |text, x| s.sans_block(text, x, text_y(y), TextSize::Row, Ink::Second);
                let block = label(series.name, left + label_x);
                let (w, _) = list.measure_block(&block);
                list.text(block);
                if series.estimated {
                    list.text(label(" (est.)", left + label_x + w));
                }
                value(list, &values[index], y, Ink::Value);
                list.pop_tint();
                y += TIP_ROW + TIP_ROW_GAP;
            }
            // The rule sits halfway through the space `rule` adds, across
            // the padding too.
            let rule_y = (y + TIP_ROW_GAP * 0.5).round();
            let (x, w) = (inner.x - TIP_PAD_X, inner.width + TIP_PAD_X * 2.0);
            list.quad(x, rule_y, w, 1.0, TIP_RULE);
            list.quad(x, rule_y + 1.0, w, 1.0, TIP_RULE_LIT);
            y += rule;
            list.text(s.sans_block("Total", left, text_y(y), TextSize::Row, Ink::Emph));
            value(list, &total_text, y, Ink::Max);
            if let (Some(reference), Some(text)) = (bar.reference, &reference_text) {
                y += TIP_ROW + TIP_ROW_GAP;
                let mid = (y + TIP_ROW * 0.5).round();
                list.dashed_hline(left, mid, TIP_LINE, DASH, DASH_GAP, s.ink(Ink::Value));
                list.text(s.sans_block(
                    self.reference_label,
                    left + line_label_x,
                    text_y(y),
                    TextSize::Row,
                    Ink::Second,
                ));
                let warn = (total > reference).then(|| s.color(StyleKey::WarnMeta));
                chart::draw_tip_value(list, s, text, right, y, Ink::Value, warn);
            }
            if let Some((overlay, text)) = &overlay {
                y += TIP_ROW + TIP_ROW_GAP;
                list.push_tint();
                if overlay.value(tip.index).is_none() {
                    list.multiply_tint([1.0, 1.0, 1.0, TIP_ZERO]);
                }
                let mid = (y + TIP_ROW * 0.5).round();
                chart::line_swatch(list, left, mid, TIP_LINE, overlay.color, false);
                let label_x = left + line_label_x;
                let ink = Ink::Second;
                list.text(s.sans_block(overlay.label, label_x, text_y(y), TextSize::Row, ink));
                let color = Some(overlay.color);
                chart::draw_tip_value(list, s, text, right, y, Ink::Value, color);
                list.pop_tint();
            }
        });
        Some(rect)
    }
}

/// An entry of the legend: a series, the overlay, the reference line, the
/// current bar.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LegendEntry {
    Series(usize),
    Overlay,
    Reference,
    Current,
}

/// Fill `rect` the way `series` is painted: solid, or (estimated) faint
/// inside a full-colour outline.
fn paint_fill(list: &mut DrawList, rect: Rect, series: &BarSeries<'_>) {
    if series.estimated {
        let mut faint = series.color;
        faint[3] *= ESTIMATED_FILL;
        list.quad(rect.x, rect.y, rect.width, rect.height, faint);
        list.rect_outline(rect, 1.0, series.color);
    } else {
        list.quad(rect.x, rect.y, rect.width, rect.height, series.color);
    }
}

#[cfg(test)]
#[path = "bar_chart_tests.rs"]
mod tests;
