//! Scatter plot — a small calibration scatter in a sunken well (Forge's
//! `ScatterPlot`): readings as dots, those left out of a fit flagged amber,
//! and a dashed reference (the fit's median) across them.
//!
//! y runs from zero to a nice maximum a little over the highest reading or
//! the reference ([`nice_max`]); x runs over 0..1 when every x is in it,
//! else over the readings' own span (times, say). The axes are minimal: the
//! maximum top-left and zero bottom-left, on plates inside the well, and
//! optional labels under it spread edge to edge.
//!
//! Under [`ScatterPlot::min_points`] readings there is too little to read a
//! trend from: the well says so, with the count so far, over the readings
//! at half strength.
//!
//! Like the other charts, the widget only paints and reports the reading
//! nearest the pointer (within 10px); the caller feeds it back and paints
//! the tooltip last ([`ScatterPlot::draw_tooltip`]).

use crate::InputState;
use crate::color::oklch;
use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};
use crate::text::{TextAlign, TextBlock};

use super::DrawList;
use super::chart::{self, DASH, DASH_GAP, DOT, HOVER_DOT, TIP_ROW, nice_max};

/// The well's height when the caller doesn't say.
pub const SCATTER_HEIGHT: f32 = 110.0;
/// Room inside the well around the readings, and the y labels' column kept
/// clear of them on either side.
const PAD: f32 = 8.0;
const Y_LABELS: f32 = 14.0;
/// The axis reaches this far over the highest reading.
const HEADROOM: f64 = 1.08;
/// The well: its recess (`--well-inset`'s `inset 0 2px 4px`) and lit lip
/// (`0 1px 0` white at 7%).
const RECESS: ([f32; 2], f32) = ([0.0, 2.0], 4.0);
const LIP: BoxShadow = BoxShadow {
    offset: [0.0, 1.0],
    blur: 0.0,
    spread: 0.0,
    color: [1.0, 1.0, 1.0, 0.07],
    inset: false,
};
/// The plates the labels inside the well sit on: their padding, height and
/// how far in from the well's edge, and the ring round them.
const PLATE_PAD: f32 = 3.0;
const PLATE: f32 = 10.0;
const PLATE_INSET_X: f32 = 3.0;
const PLATE_INSET_Y: f32 = 4.0;
const PLATE_RADIUS: f32 = 1.0;
const PLATE_RING: [f32; 4] = [0.0, 0.0, 0.0, 0.35];
/// The rings outside a reading's dot, and outside the hovered one's.
const DOT_RING: [f32; 4] = [0.0, 0.0, 0.0, 0.75];
const HOVER_DOT_RING: [f32; 4] = [0.0, 0.0, 0.0, 0.8];
/// Readings under the minimum: half strength.
const TOO_FEW: f32 = 0.5;
/// How near the pointer must be to a reading to hover it.
const HOVER_REACH: f32 = 10.0;
/// The empty state's gap between its caption and its hint, and its sides.
const EMPTY_GAP: f32 = 4.0;
const EMPTY_PAD: f32 = 12.0;
/// The x labels: under the well, padded in from its sides.
const X_LABEL_GAP: f32 = 4.0;
const X_LABEL_ROW: f32 = 12.0;
const X_LABEL_PAD: f32 = 2.0;
/// The tooltip: above the reading, beside it, the gap between its value and
/// the flag, and the tip's line under them.
const TIP_GAP: f32 = 8.0;
const TIP_FLAG_GAP: f32 = 8.0;
const TIP_LINE_GAP: f32 = 2.0;

/// One reading of a [`ScatterPlot`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScatterPoint<'a> {
    /// Where along x: in 0..1, or (when any reading isn't) anything, e.g. a
    /// time.
    pub x: f64,
    /// Its value.
    pub y: f64,
    /// Left out of the fit: amber, drawn over the others, and said so in its
    /// tooltip.
    pub flagged: bool,
    /// A line under its value in the tooltip (when it was taken, say).
    pub tip: Option<&'a str>,
}

/// What a [`ScatterPlot`] did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScatterPlotOutput {
    /// The reading the pointer is over, if any.
    pub hovered: Option<usize>,
    /// The height it took, x labels included.
    pub height: f32,
    /// Where the hovered reading's tooltip goes, for
    /// [`ScatterPlot::draw_tooltip`]. Only while the pointer itself is over
    /// it: a reading lit from a table shows no tooltip.
    pub tooltip: Option<ScatterTooltip>,
}

/// Where a reading's tooltip goes: above it, toward the middle of the well.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScatterTooltip {
    /// The reading's index.
    pub index: usize,
    /// The reading's dot.
    pub at: (f32, f32),
    /// Whether the tooltip runs right from the dot (it is in the left half).
    pub rightward: bool,
    /// The well's top, which the tooltip's bottom stays below.
    pub top: f32,
}

/// The layout every part of the plot shares, worked out once a frame.
struct Layout {
    /// The area inside the well's border.
    inner: Rect,
    /// The y axis' maximum, and the x span.
    max: f64,
    x0: f64,
    x1: f64,
}

impl Layout {
    /// Where reading `(x, y)` is.
    fn at(&self, x: f64, y: f64) -> (f32, f32) {
        let t = if self.x1 > self.x0 {
            ((x - self.x0) / (self.x1 - self.x0)) as f32
        } else {
            0.5
        };
        let span = (self.inner.width - (PAD + Y_LABELS) * 2.0).max(0.0);
        (self.inner.x + PAD + Y_LABELS + t * span, self.y(y))
    }

    /// Where value `y` is.
    fn y(&self, y: f64) -> f32 {
        let span = self.inner.height - PAD * 2.0;
        self.inner.bottom() - PAD - (y / self.max) as f32 * span
    }
}

/// A scatter plot.
#[derive(Clone, Copy)]
pub struct ScatterPlot<'a> {
    points: &'a [ScatterPoint<'a>],
    format: &'a dyn Fn(f64) -> String,
    color: [f32; 4],
    height: f32,
    reference: Option<f64>,
    reference_label: &'a str,
    x_labels: Option<&'a [&'a str]>,
    min_points: usize,
    empty_label: &'a str,
    empty_hint: Option<&'a str>,
    flagged_label: &'a str,
    hovered: Option<usize>,
}

impl<'a> ScatterPlot<'a> {
    /// A plot of `points`, their values written by `format` on the axis and
    /// in the tooltip.
    pub fn new(points: &'a [ScatterPoint<'a>], format: &'a dyn Fn(f64) -> String) -> Self {
        Self {
            points,
            format,
            color: oklch(0.68, 0.1, 200.0, 1.0),
            height: SCATTER_HEIGHT,
            reference: None,
            reference_label: "median",
            x_labels: None,
            min_points: 3,
            empty_label: "Not enough readings yet",
            empty_hint: None,
            flagged_label: "excluded from fit",
            hovered: None,
        }
    }

    /// The readings' colour; flagged ones are amber whatever it is.
    pub fn color(mut self, color: [f32; 4]) -> Self {
        self.color = color;
        self
    }

    /// The well's height.
    pub fn height(mut self, height: f32) -> Self {
        self.height = height.max(PAD * 2.0 + 1.0);
        self
    }

    /// A dashed line across the well at `value` (the fit's median),
    /// labelled on the right.
    pub fn reference(mut self, value: f64) -> Self {
        self.reference = value.is_finite().then_some(value);
        self
    }

    /// What the reference is called on its label.
    pub fn reference_label(mut self, label: &'a str) -> Self {
        self.reference_label = label;
        self
    }

    /// Labels under the well, spread from its left edge to its right ("14 d
    /// ago", "now").
    pub fn x_labels(mut self, labels: &'a [&'a str]) -> Self {
        self.x_labels = Some(labels);
        self
    }

    /// How many readings it takes to plot them; under it the well says
    /// there are too few.
    pub fn min_points(mut self, min: usize) -> Self {
        self.min_points = min;
        self
    }

    /// What the well says with too few readings, and the line under it
    /// ("2 of 3 readings" when `None`).
    pub fn empty_labels(mut self, label: &'a str, hint: Option<&'a str>) -> Self {
        self.empty_label = label;
        self.empty_hint = hint;
        self
    }

    /// What a flagged reading's tooltip says it is.
    pub fn flagged_label(mut self, label: &'a str) -> Self {
        self.flagged_label = label;
        self
    }

    /// The reading to light: last frame's [`ScatterPlotOutput::hovered`],
    /// or the row hovered in a table of the same readings.
    pub fn hovered(mut self, hovered: Option<usize>) -> Self {
        self.hovered = hovered;
        self
    }

    /// Whether there are enough readings to plot.
    fn enough(&self) -> bool {
        self.points.len() >= self.min_points
    }

    /// The readings that can be placed: those with finite coordinates.
    fn placed(&self) -> impl Iterator<Item = (usize, &ScatterPoint<'a>)> {
        self.points
            .iter()
            .enumerate()
            .filter(|(_, p)| p.x.is_finite() && p.y.is_finite())
    }

    fn layout(&self, well: Rect, s: &StyleResolver) -> Layout {
        let border = s.scalar(StyleKey::BorderWidth);
        let highest = self
            .placed()
            .map(|(_, p)| p.y)
            .chain(self.reference)
            .fold(0.0, f64::max);
        let unit = self.placed().all(|(_, p)| (0.0..=1.0).contains(&p.x));
        let (x0, x1) = if unit {
            (0.0, 1.0)
        } else {
            self.placed()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (_, p)| {
                    (lo.min(p.x), hi.max(p.x))
                })
        };
        Layout {
            inner: well.inset(border),
            max: nice_max(highest * HEADROOM),
            x0,
            x1,
        }
    }

    /// The reading nearest the pointer, within reach.
    fn hit(&self, layout: &Layout, well: Rect, input: &InputState) -> Option<usize> {
        if input.mouse_consumed || !self.enough() || !well.contains(input.mouse_x, input.mouse_y) {
            return None;
        }
        let mut best = None;
        let mut nearest = HOVER_REACH * HOVER_REACH;
        for (i, p) in self.placed() {
            let (px, py) = layout.at(p.x, p.y);
            let d = (px - input.mouse_x).powi(2) + (py - input.mouse_y).powi(2);
            if d < nearest {
                (nearest, best) = (d, Some(i));
            }
        }
        best
    }

    /// The height it takes, x labels included.
    pub fn measure_height(&self) -> f32 {
        self.height
            + if self.x_labels.is_some() {
                X_LABEL_GAP + X_LABEL_ROW
            } else {
                0.0
            }
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
    ) -> ScatterPlotOutput {
        let well = Rect::new(x, y, width, self.height);
        let layout = self.layout(well, s);
        let hit = self.hit(&layout, well, input);
        let lit = hit.or(self.hovered).filter(|&i| i < self.points.len());
        let enough = self.enough();

        draw_well(list, s, well);
        if let (true, Some(reference)) = (enough, self.reference) {
            self.draw_reference(reference, &layout, list, s);
        }

        list.push_tint();
        if !enough {
            list.multiply_tint([1.0, 1.0, 1.0, TOO_FEW]);
        }
        // The fit's readings, then the flagged ones over them, then the lit
        // one over everything.
        for flagged in [false, true] {
            for (i, p) in self.placed() {
                if p.flagged == flagged && lit != Some(i) {
                    let fill = self.fill(p, s);
                    let center = layout.at(p.x, p.y);
                    list.circle(center, DOT, fill);
                    list.circle_outline(center, DOT + 0.5, 1.0, DOT_RING);
                }
            }
        }
        if let Some(i) = lit
            && let Some(p) = self
                .points
                .get(i)
                .filter(|p| p.x.is_finite() && p.y.is_finite())
        {
            let center = layout.at(p.x, p.y);
            list.circle(center, HOVER_DOT, self.fill(p, s));
            list.circle_outline(center, HOVER_DOT + 0.5, 1.0, HOVER_DOT_RING);
            list.circle_outline(center, HOVER_DOT + 1.5, 1.0, s.ink(Ink::Value));
        }
        list.pop_tint();

        if enough {
            self.draw_plates(&layout, list, s);
        } else {
            self.draw_empty(&layout, list, s);
        }
        if let Some(labels) = self.x_labels {
            draw_x_labels(list, s, labels, well);
        }

        let tooltip = hit.map(|index| {
            let p = &self.points[index];
            let at = layout.at(p.x, p.y);
            ScatterTooltip {
                index,
                at,
                rightward: at.0 < x + width * 0.5,
                top: y,
            }
        });
        ScatterPlotOutput {
            hovered: hit,
            height: self.measure_height(),
            tooltip,
        }
    }

    /// A reading's colour: amber when flagged.
    fn fill(&self, point: &ScatterPoint<'_>, s: &StyleResolver) -> [f32; 4] {
        if point.flagged {
            s.color(StyleKey::WarnMeta)
        } else {
            self.color
        }
    }

    /// A label in `ink` on a plate inside the well, its top at `y` and the
    /// edge `anchor` names at its x.
    fn plate(
        &self,
        list: &mut DrawList,
        s: &StyleResolver,
        text: &str,
        ink: Ink,
        y: f32,
        anchor: Anchor,
    ) {
        let caption = s.text_size(TextSize::Caption);
        let mut block = s.mono_block(text, 0.0, 0.0, TextSize::Caption, ink);
        let (w, _) = list.measure_block(&block);
        let width = w + PLATE_PAD * 2.0;
        let left = match anchor {
            Anchor::Left(x) => x,
            Anchor::Right(x) => x - width,
        };
        let rect = Rect::new(left, y, width, PLATE);
        list.chrome_rect(
            rect.inset(-1.0),
            PLATE_RADIUS + 1.0,
            1.0,
            s.color(StyleKey::WellDeep),
            PLATE_RING,
        );
        block.x = left + PLATE_PAD;
        block.y = crate::text::vcentered_line_y(y, PLATE, caption);
        list.text(block);
    }

    /// The reference's dashed line across the readings' span, under them.
    fn draw_reference(&self, value: f64, layout: &Layout, list: &mut DrawList, s: &StyleResolver) {
        let inner = layout.inner;
        let left = inner.x + PAD + Y_LABELS;
        let width = (inner.width - (PAD + Y_LABELS) * 2.0).max(0.0);
        let line_y = layout.y(value).round();
        list.dashed_hline(left, line_y, width, DASH, DASH_GAP, s.ink(Ink::Glyph));
    }

    /// The labels on their plates, over the readings: the axis' maximum
    /// top-left, zero bottom-left, and the reference's on the right of its
    /// line.
    fn draw_plates(&self, layout: &Layout, list: &mut DrawList, s: &StyleResolver) {
        let inner = layout.inner;
        let left = Anchor::Left(inner.x + PLATE_INSET_X);
        let top = inner.y + PLATE_INSET_Y;
        let bottom = inner.bottom() - PLATE_INSET_Y - PLATE;
        self.plate(list, s, &(self.format)(layout.max), Ink::Dim, top, left);
        self.plate(list, s, "0", Ink::Dim, bottom, left);
        if let Some(value) = self.reference {
            let text = format!("{} {}", self.reference_label, (self.format)(value));
            let right = Anchor::Right(inner.right() - PLATE_INSET_X);
            let line_y = layout.y(value).round();
            self.plate(list, s, &text, Ink::Glyph, line_y - PLATE * 0.5, right);
        }
    }

    /// The caption and count over too few readings, centred in the well.
    fn draw_empty(&self, layout: &Layout, list: &mut DrawList, s: &StyleResolver) {
        let inner = layout.inner;
        let caption = self.empty_label.to_uppercase();
        let hint = self.empty_hint.map_or_else(
            || format!("{} of {} readings", self.points.len(), self.min_points),
            str::to_owned,
        );
        // Each wraps to the well's width, less its sides, centred.
        let room = (inner.width - EMPTY_PAD * 2.0).max(1.0);
        let left = inner.x + EMPTY_PAD;
        let centred = |block: TextBlock| block.with_max_width(room).with_align(TextAlign::Center);
        let mut top = centred(s.mono_block(&caption, left, 0.0, TextSize::Caption, Ink::Caption));
        let mut under = centred(s.mono_block(&hint, left, 0.0, TextSize::Meta, Ink::Dim));
        let (_, top_h) = list.measure_block(&top);
        let (_, under_h) = list.measure_block(&under);
        top.y = inner.y + (inner.height - top_h - EMPTY_GAP - under_h) * 0.5;
        under.y = top.y + top_h + EMPTY_GAP;
        list.text(top);
        list.text(under);
    }

    /// Paint the tooltip `tip` placed for its reading: its value, "excluded
    /// from fit" when it is flagged, and its tip under them. `list` should be
    /// drawn above everything the tooltip may overlap, and `viewport` is what
    /// it must stay inside.
    pub fn draw_tooltip(
        &self,
        tip: &ScatterTooltip,
        viewport: Rect,
        list: &mut DrawList,
        s: &StyleResolver,
    ) -> Option<Rect> {
        let point = self.points.get(tip.index)?;
        let value = (self.format)(point.y);
        let flag = point.flagged.then(|| self.flagged_label.to_uppercase());
        let mut first = s.mono_width(list, &value, TextSize::Meta);
        if let Some(flag) = &flag {
            first += TIP_FLAG_GAP + s.mono_width(list, flag, TextSize::Caption);
        }
        let second = point.tip.map(|text| chart::row_text_width(list, s, text));
        let content = first.max(second.unwrap_or(0.0));
        let content_height = TIP_ROW + second.map_or(0.0, |_| TIP_LINE_GAP + TIP_ROW);
        let (width, height) = chart::tooltip_size(s, content, content_height, 0.0);
        let (px, py) = tip.at;
        let x = if tip.rightward {
            px - TIP_GAP
        } else {
            px + TIP_GAP - width
        };
        let bottom = (py - TIP_GAP).max(tip.top);
        let x = x.min(viewport.right() - width).max(viewport.x).round();
        let y = (bottom - height)
            .min(viewport.bottom() - height)
            .max(viewport.y)
            .round();
        let rect = Rect::new(x, y, width, height);

        chart::paint_tooltip(list, s, rect, |list, inner| {
            let meta = s.text_size(TextSize::Meta);
            let caption = s.text_size(TextSize::Caption);
            let row_y = |y: f32, size| crate::text::vcentered_line_y(y, TIP_ROW, size);
            let block = s.mono_block(
                &value,
                inner.x,
                row_y(inner.y, meta),
                TextSize::Meta,
                Ink::Max,
            );
            let (w, _) = list.measure_block(&block);
            list.text(block);
            if let Some(flag) = &flag {
                let at = inner.x + w + TIP_FLAG_GAP;
                let block = s
                    .mono_block(
                        flag,
                        at,
                        row_y(inner.y, caption),
                        TextSize::Caption,
                        Ink::Dim,
                    )
                    .with_color_f32(s.color(StyleKey::WarnMeta));
                list.text(block);
            }
            if let Some(text) = point.tip {
                let y = inner.y + TIP_ROW + TIP_LINE_GAP;
                let row = s.text_size(TextSize::Row);
                list.text(s.sans_block(text, inner.x, row_y(y, row), TextSize::Row, Ink::Second));
            }
        });
        Some(rect)
    }
}

/// Which edge of a plate sits at the x given.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Anchor {
    Left(f32),
    Right(f32),
}

/// The sunken well: `--well-deep` in a `--well-border` edge, recessed, with
/// a lit lip under it.
fn draw_well(list: &mut DrawList, s: &StyleResolver, well: Rect) {
    let radius = s.scalar(StyleKey::BorderRadius);
    let border = s.scalar(StyleKey::BorderWidth);
    list.box_shadow_outset(well, CornerRadii::uniform(radius), LIP);
    list.chrome_rect(
        well,
        radius,
        border,
        s.color(StyleKey::WellDeep),
        s.color(StyleKey::InputBorder),
    );
    let (offset, blur) = RECESS;
    list.box_shadow_inset(
        well.inset(border),
        CornerRadii::uniform((radius - border).max(0.0)),
        BoxShadow {
            offset,
            blur,
            color: s.color(StyleKey::InnerShadow),
            inset: true,
            ..BoxShadow::default()
        },
    );
}

/// `labels` under `well`, the first at its left, the last at its right and
/// the rest spread evenly between them.
fn draw_x_labels(list: &mut DrawList, s: &StyleResolver, labels: &[&str], well: Rect) {
    let caption = s.text_size(TextSize::Caption);
    let y = crate::text::vcentered_line_y(well.bottom() + X_LABEL_GAP, X_LABEL_ROW, caption);
    let mut blocks: Vec<_> = labels
        .iter()
        .map(|label| s.mono_block(*label, 0.0, y, TextSize::Caption, Ink::Dim))
        .collect();
    let widths: Vec<f32> = blocks.iter().map(|b| list.measure_block(b).0).collect();
    let room = well.width - X_LABEL_PAD * 2.0;
    let gap = if blocks.len() > 1 {
        (room - widths.iter().sum::<f32>()) / (blocks.len() - 1) as f32
    } else {
        0.0
    };
    let mut x = well.x + X_LABEL_PAD;
    for (block, w) in blocks.iter_mut().zip(widths) {
        block.x = x;
        x += w + gap;
    }
    for block in blocks {
        list.text(block);
    }
}

#[cfg(test)]
#[path = "scatter_plot_tests.rs"]
mod tests;
