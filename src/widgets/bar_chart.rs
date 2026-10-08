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
//! Like [`Waffle`](super::Waffle), the widget only paints and reports what
//! the pointer is over: the caller feeds the hovered bar back (and may share
//! it with a table of the same rows), and paints the tooltip last, above
//! whatever the chart sits among ([`BarChart::draw_tooltip`]).

use crate::InputState;
use crate::SurfacePainter;
use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};

use super::DrawList;

/// The plot's height when the caller doesn't say.
pub const BAR_CHART_HEIGHT: f32 = 190.0;
/// The widest a bar gets when the caller doesn't say.
const MAX_BAR_WIDTH: f32 = 56.0;
/// The row of x labels under the plot.
const X_LABELS: f32 = 16.0;
/// Room over the plot for the tallest bar's top.
const PAD: f32 = 10.0;
/// More room over it for an outlier's value.
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
/// The space an x label wants, and how near an edge one is pinned to it.
const LABEL_SPACING: f32 = 64.0;
const EDGE_ROOM: f32 = 28.0;
/// The narrowest the y axis' gutter gets, and the room right of its labels.
const MIN_GUTTER: f32 = 24.0;
const GUTTER_GAP: f32 = 6.0;
/// The other bars while one is hovered; the other series while a legend
/// entry is.
const DIMMED: f32 = 0.45;
const SERIES_FADED: f32 = 0.25;
/// An estimated series' fill, over the ground; its outline is full colour.
const ESTIMATED_FILL: f32 = 0.4;
const GRID: [f32; 4] = [1.0, 1.0, 1.0, 0.05];
const BASELINE: [f32; 4] = [1.0, 1.0, 1.0, 0.14];
const BASELINE_SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.5];
/// A bar's lit top edge.
const BAR_HI: [f32; 4] = [1.0, 1.0, 1.0, 0.22];
/// The current bar's hatch.
const HATCH: [f32; 4] = [0.0, 0.0, 0.0, 0.45];
const HATCH_STEP: f32 = 4.0;
/// The reference line: 3px dashes 2px apart, at this strength, edged dark.
const DASH: f32 = 3.0;
const DASH_GAP: f32 = 2.0;
const REFERENCE: f32 = 0.85;
const REFERENCE_EDGE: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
/// The current bar's "now" tick under the baseline.
const NOW_TICK: f32 = 2.0;
/// The legend: under the plot, entries wrapped in rows.
const LEGEND_GAP: f32 = 8.0;
const LEGEND_ROW: f32 = 16.0;
const LEGEND_ROW_GAP: f32 = 4.0;
const LEGEND_COL_GAP: f32 = 14.0;
const LEGEND_SWATCH: f32 = 8.0;
const LEGEND_DASH: f32 = 12.0;
const SWATCH_GAP: f32 = 6.0;
/// The tooltip: beside the bar, its rows of series.
const TIP_GAP: f32 = 8.0;
const TIP_MIN_WIDTH: f32 = 160.0;
const TIP_PAD_X: f32 = 8.0;
const TIP_PAD_Y: f32 = 6.0;
const TIP_ROW: f32 = 14.0;
const TIP_ROW_GAP: f32 = 3.0;
const TIP_SWATCH: f32 = 7.0;
const TIP_VALUE_GAP: f32 = 14.0;
const TIP_RULE: [f32; 4] = [0.0, 0.0, 0.0, 0.5];
const TIP_RULE_LIT: [f32; 4] = [1.0, 1.0, 1.0, 0.06];
/// A series that is zero in the hovered bar.
const TIP_ZERO: f32 = 0.5;

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

/// What a [`BarChart`] did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BarChartOutput {
    /// The bar under the pointer, if any.
    pub hovered: Option<usize>,
    /// The legend entry under the pointer, if any.
    pub hovered_series: Option<usize>,
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
    /// The y labels' column, left of the plot.
    gutter: f32,
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
            height: BAR_CHART_HEIGHT,
            max_bar_width: MAX_BAR_WIDTH,
            hovered: None,
            hovered_series: None,
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
        let ticks = std::array::from_fn(|k| {
            if all_zero && k > 0 {
                String::new()
            } else if all_zero {
                (self.format)(0.0)
            } else {
                (self.format)(max * k as f64 / 3.0)
            }
        });
        let gutter = if n == 0 {
            0.0
        } else {
            let widest = ticks
                .iter()
                .map(|tick: &String| s.mono_width(list, tick, TextSize::Meta))
                .fold(0.0, f32::max);
            (widest + GUTTER_GAP + 2.0).ceil().max(MIN_GUTTER)
        };
        let plot = (width - gutter).max(0.0);
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
            area.width - layout.gutter,
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
            height += LEGEND_GAP + legend_height(rows);
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
            let caption = caption.to_uppercase();
            let size = s.text_size(TextSize::Caption);
            let mut block = s.mono_block(
                &caption,
                0.0,
                crate::text::vcentered_line_y(base - self.height, self.height, size),
                TextSize::Caption,
                Ink::Caption,
            );
            let (w, _) = list.measure_block(&block);
            block.x = x + layout.gutter + ((width - layout.gutter - w) * 0.5).max(0.0);
            list.text(block);
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
        self.draw_x_labels(area, base, &layout, lit, list, s);

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
            height += LEGEND_GAP + legend_height(rows);
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
        let width = area.right() - left;
        let meta = s.text_size(TextSize::Meta);
        for (k, tick) in layout.ticks.iter().enumerate() {
            let line_y = base - (self.height * k as f32 / 3.0).round();
            if k == 0 {
                list.quad(left, line_y, width, 1.0, BASELINE);
                list.quad(left, line_y + 1.0, width, 1.0, BASELINE_SHADOW);
            } else {
                list.quad(left, line_y, width, 1.0, GRID);
            }
            if self.bars.is_empty() || tick.is_empty() {
                continue;
            }
            let mut block = s.mono_block(tick, 0.0, 0.0, TextSize::Meta, Ink::Dim);
            let (w, _) = list.measure_block(&block);
            block.x = left - GUTTER_GAP - w;
            block.y = crate::text::vcentered_line_y(line_y - 6.0, 12.0, meta);
            list.text(block);
        }
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
            let accent = s.color(StyleKey::Accent);
            let tick = Rect::new(left, base + 1.0, width, NOW_TICK);
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
        let faded = self.hovered_series.is_some_and(|lit| lit != index);
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
        let plot = area.width - layout.gutter;
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
            block.x = if area.right() - center < EDGE_ROOM {
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
        mut place: impl FnMut(&mut DrawList, LegendEntry, Rect),
    ) -> usize {
        let has_reference = self.bars.iter().any(|bar| bar.reference.is_some());
        let has_current = self.bars.iter().any(|bar| bar.current);
        let entries = (0..self.series.len())
            .map(LegendEntry::Series)
            .chain(has_reference.then_some(LegendEntry::Reference))
            .chain(has_current.then_some(LegendEntry::Current));
        let (mut x, mut row) = (layout.gutter, 0);
        for entry in entries {
            let w = self.legend_entry_width(entry, list, s);
            if x > layout.gutter && x + w > width {
                x = layout.gutter;
                row += 1;
            }
            let y = row as f32 * (LEGEND_ROW + LEGEND_ROW_GAP);
            place(list, entry, Rect::new(x, y, w, LEGEND_ROW));
            x += w + LEGEND_COL_GAP;
        }
        row + 1
    }

    fn legend_entry_width(
        &self,
        entry: LegendEntry,
        list: &mut DrawList,
        s: &StyleResolver,
    ) -> f32 {
        let text = |list: &mut DrawList, text: &str| {
            list.measure_block(&s.sans_block(text, 0.0, 0.0, TextSize::Row, Ink::Second))
                .0
        };
        match entry {
            LegendEntry::Series(index) => {
                let series = &self.series[index];
                let mut w = LEGEND_SWATCH + SWATCH_GAP + text(list, series.name);
                if series.estimated {
                    w += SWATCH_GAP + s.mono_width(list, "EST.", TextSize::Caption);
                }
                w
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
        let chrome = s.tooltip();
        let border = chrome.surface.border_widths;
        let mono = |list: &mut DrawList, text: &str| s.mono_width(list, text, TextSize::Meta);
        let sans = |list: &mut DrawList, text: &str| {
            list.measure_block(&s.sans_block(text, 0.0, 0.0, TextSize::Row, Ink::Second))
                .0
        };

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
        let label_x = TIP_SWATCH + SWATCH_GAP;
        let mut content = sans(list, bar.long_label)
            + if bar.current {
                TIP_VALUE_GAP + mono(list, "RUNNING")
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
        if let Some(reference) = &reference_text {
            content = content.max(
                label_x + sans(list, self.reference_label) + TIP_VALUE_GAP + mono(list, reference),
            );
        }
        let rows = 2 + self.series.len() + usize::from(reference_text.is_some());
        let rule = TIP_ROW_GAP * 2.0 + 1.0;
        let width = (content + TIP_PAD_X * 2.0 + border.left + border.right).max(TIP_MIN_WIDTH);
        let height = rows as f32 * TIP_ROW
            + (rows - 1) as f32 * TIP_ROW_GAP
            + rule
            + TIP_PAD_Y * 2.0
            + border.top
            + border.bottom;
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
        {
            let list = surface.draw_list();
            let left = padding_box.x + TIP_PAD_X;
            let right = padding_box.right() - TIP_PAD_X;
            let row_size = s.text_size(TextSize::Row);
            let mut y = padding_box.y + TIP_PAD_Y;
            let text_y = |y: f32| crate::text::vcentered_line_y(y, TIP_ROW, row_size);
            let value = |list: &mut DrawList, text: &str, y: f32, ink: Ink| {
                let meta = s.text_size(TextSize::Meta);
                let mut block = s.mono_block(text, 0.0, 0.0, TextSize::Meta, ink);
                let (w, _) = list.measure_block(&block);
                block.x = right - w;
                block.y = crate::text::vcentered_line_y(y, TIP_ROW, meta);
                list.text(block);
            };

            list.text(s.sans_block(bar.long_label, left, text_y(y), TextSize::Row, Ink::Max));
            if bar.current {
                let caption = s.text_size(TextSize::Caption);
                let mut block = s
                    .mono_block("RUNNING", 0.0, 0.0, TextSize::Caption, Ink::Dim)
                    .with_color_f32(s.color(StyleKey::Accent));
                let (w, _) = list.measure_block(&block);
                block.x = right - w;
                block.y = crate::text::vcentered_line_y(y, TIP_ROW, caption);
                list.text(block);
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
            // The rule sits halfway through the space `rule` adds.
            let rule_y = (y + TIP_ROW_GAP * 0.5).round();
            let (x, w) = (padding_box.x, padding_box.width);
            list.quad(x, rule_y, w, 1.0, TIP_RULE);
            list.quad(x, rule_y + 1.0, w, 1.0, TIP_RULE_LIT);
            y += rule;
            list.text(s.sans_block("Total", left, text_y(y), TextSize::Row, Ink::Emph));
            value(list, &total_text, y, Ink::Max);
            if let (Some(reference), Some(text)) = (bar.reference, &reference_text) {
                y += TIP_ROW + TIP_ROW_GAP;
                let mid = (y + TIP_ROW * 0.5).round();
                list.dashed_hline(left, mid, 10.0, DASH, DASH_GAP, s.ink(Ink::Value));
                list.text(s.sans_block(
                    self.reference_label,
                    left + label_x,
                    text_y(y),
                    TextSize::Row,
                    Ink::Second,
                ));
                if total > reference {
                    let meta = s.text_size(TextSize::Meta);
                    let mut block = s
                        .mono_block(text, 0.0, 0.0, TextSize::Meta, Ink::Value)
                        .with_color_f32(s.color(StyleKey::WarnMeta));
                    let (w, _) = list.measure_block(&block);
                    block.x = right - w;
                    block.y = crate::text::vcentered_line_y(y, TIP_ROW, meta);
                    list.text(block);
                } else {
                    value(list, text, y, Ink::Value);
                }
            }
        }
        surface.paint_post_content();
        Some(rect)
    }
}

/// An entry of the legend: a series, the reference line, the current bar.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LegendEntry {
    Series(usize),
    Reference,
    Current,
}

fn legend_height(rows: usize) -> f32 {
    rows as f32 * LEGEND_ROW + rows.saturating_sub(1) as f32 * LEGEND_ROW_GAP
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
