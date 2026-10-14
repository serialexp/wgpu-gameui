//! Passive marks over a 3D viewport (Forge `TransformReadout`,
//! `SelectionMarquee`, `MeasureLine` and `ViewportStats`): they read state
//! the host already has and take no input, so they paint straight into a
//! [`DrawList`].

use crate::layout::Rect;
use crate::text::{FontHandle, TextBlock};
use crate::{Ink, StyleKey, StyleResolver, Tracking};

use super::hud::hud_glass;
use super::transform_gizmo::{GizmoHandle, GizmoMode, GizmoValue, TransformEvent};
use super::{DrawList, Stroke};

/// Ink on an axis-tinted chip (`#0a0d0f`).
const CHIP_INK: [f32; 4] = [0.039, 0.051, 0.059, 1.0];
/// The overlays' white (`#f1f5f9`).
const WHITE: [f32; 4] = [0.945, 0.961, 0.976, 1.0];
/// The text-shadow `--carve` (`0 -1px 0 rgba(0,0,0,.5)`).
const CARVE_ALPHA: u8 = 128;

/// The `y` to give a one-line block of `size` in `font` so its baseline
/// sits at `baseline`: mixed sizes in a row share one baseline.
fn top_for_baseline(
    list: &mut DrawList,
    baseline: f32,
    size: f32,
    font: Option<&FontHandle>,
) -> f32 {
    baseline - size * list.font_vmetrics(font).baseline_ratio
}

/// The baseline that optically centres capitals of `size` in `font` in a
/// row from `top`, `height` tall.
fn row_baseline(
    list: &mut DrawList,
    top: f32,
    height: f32,
    size: f32,
    font: Option<&FontHandle>,
) -> f32 {
    top + height * 0.5 + size * list.font_vmetrics(font).cap_ratio * 0.5
}

fn mono(s: &StyleResolver) -> Option<FontHandle> {
    s.theme().mono_font.clone()
}

// ---------------------------------------------------------------------------
// TransformReadout

/// Readout padding: top, sides, bottom (`4px 7px 5px`), and the gap between
/// its rows.
const READOUT_PAD: [f32; 3] = [4.0, 7.0, 5.0];
const READOUT_ROW_GAP: f32 = 3.0;
/// The readout sits this far right of and below the pointer.
const READOUT_OFFSET: [f32; 2] = [16.0, 14.0];
/// Row heights: the title row, the values row, the hint row.
const READOUT_ROWS: [f32; 3] = [14.0, 14.0, 11.0];
/// Sizes: the title, the small mono (chips, snap, hints, axis ids), values.
const READOUT_TITLE: f32 = 10.5;
const READOUT_SMALL: f32 = 8.5;
const READOUT_VALUE: f32 = 11.0;
/// Gaps: between the title row's parts, between chips, between values,
/// and inside a value between id, number and unit.
const READOUT_TITLE_GAP: f32 = 7.0;
const READOUT_CHIP_GAP: f32 = 2.0;
const READOUT_VALUE_GAP: f32 = 9.0;
const READOUT_PART_GAP: f32 = 3.0;
/// An axis chip's side padding and corner radius.
const READOUT_CHIP_PAD: f32 = 3.0;
const READOUT_CHIP_RADIUS: f32 = 2.0;
/// The mode glyph's size and gap to the label.
#[cfg(feature = "phosphor-icons")]
const READOUT_GLYPH: f32 = 11.0;
#[cfg(feature = "phosphor-icons")]
const READOUT_GLYPH_GAP: f32 = 4.0;

/// The live readout beside the pointer while a transform drag is in flight
/// (Forge `TransformReadout`): the mode and the axes it is constrained to,
/// the amounts so far, and the keys that change it. Passive.
#[derive(Clone, Copy, Debug)]
pub struct TransformReadout<'a> {
    handle: GizmoHandle,
    value: GizmoValue,
    unit: Option<&'a str>,
    precision: usize,
    snap: Option<&'a str>,
    typed: Option<&'a str>,
    hints: bool,
}

/// One number of the values row: the axis it is on (if named) and the
/// amount.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shown {
    axis: Option<usize>,
    value: f32,
}

impl<'a> TransformReadout<'a> {
    /// A readout of `value` dragged on `handle`.
    pub const fn new(handle: GizmoHandle, value: GizmoValue) -> Self {
        Self {
            handle,
            value,
            unit: None,
            precision: 2,
            snap: None,
            typed: None,
            hints: true,
        }
    }

    /// A readout of a gizmo event.
    pub const fn from_event(event: &TransformEvent) -> Self {
        Self::new(event.handle, event.value)
    }

    /// The unit after each value; `m`, `°` or `×` by mode by default.
    pub const fn unit(mut self, unit: &'a str) -> Self {
        self.unit = Some(unit);
        self
    }

    /// Decimal places (2 by default).
    pub const fn precision(mut self, precision: usize) -> Self {
        self.precision = precision;
        self
    }

    /// Show the snap increment in effect ("0.5 m").
    pub const fn snap(mut self, snap: Option<&'a str>) -> Self {
        self.snap = snap;
        self
    }

    /// A number being typed in place of the dragged amount.
    pub const fn typed(mut self, typed: Option<&'a str>) -> Self {
        self.typed = typed;
        self
    }

    /// Show or hide the hint row.
    pub const fn hints(mut self, hints: bool) -> Self {
        self.hints = hints;
        self
    }

    fn mode(&self) -> GizmoMode {
        match self.value {
            GizmoValue::Translate(_) => GizmoMode::Move,
            GizmoValue::Rotate(_) => GizmoMode::Rotate,
            GizmoValue::Scale(_) => GizmoMode::Scale,
        }
    }

    fn mode_label(&self) -> &'static str {
        match self.mode() {
            GizmoMode::Move => "Move",
            GizmoMode::Rotate => "Rotate",
            GizmoMode::Scale => "Scale",
        }
    }

    fn unit_text(&self) -> &'a str {
        self.unit.unwrap_or(match self.mode() {
            GizmoMode::Move => "m",
            GizmoMode::Rotate => "°",
            GizmoMode::Scale => "×",
        })
    }

    fn hint_text(&self) -> &'static str {
        if self.typed.is_some() {
            "Enter apply · Esc cancel"
        } else {
            "X Y Z axis · ⇧ plane · type a value · Esc cancel"
        }
    }

    /// The constrained axes, or none for a free handle.
    fn axes(&self) -> Option<[bool; 3]> {
        let axes = self.handle.axes();
        axes.contains(&true).then_some(axes)
    }

    /// The numbers the values row shows, at most three.
    fn shown(&self) -> ([Shown; 3], usize) {
        let mut out = [Shown {
            axis: None,
            value: 0.0,
        }; 3];
        let first_axis = self.axes().and_then(|a| a.iter().position(|&on| on));
        let mut n = 0;
        let mut push_masked = |v: [f32; 3], out: &mut [Shown; 3]| {
            for (i, value) in v.into_iter().enumerate() {
                if self.axes().is_none_or(|a| a[i]) {
                    out[n] = Shown {
                        axis: Some(i),
                        value,
                    };
                    n += 1;
                }
            }
        };
        match self.value {
            GizmoValue::Translate(v) => push_masked(v, &mut out),
            GizmoValue::Scale(v) if self.handle == GizmoHandle::Uniform => {
                out[0] = Shown {
                    axis: None,
                    value: v[0],
                };
                n = 1;
            }
            GizmoValue::Scale(v) => push_masked(v, &mut out),
            GizmoValue::Rotate(a) => {
                out[0] = Shown {
                    axis: first_axis,
                    value: a,
                };
                n = 1;
            }
        }
        (out, n)
    }

    /// A number as the readout prints it: signed (`+1.25`, `−0.50`) except
    /// for a scale factor.
    fn format(&self, value: f32) -> String {
        let p = self.precision;
        if self.mode() == GizmoMode::Scale {
            format!("{value:.p$}")
        } else {
            let sign = if value >= 0.0 { '+' } else { '−' };
            format!("{sign}{:.p$}", value.abs())
        }
    }

    /// Lay out (and with `paint`, draw) the readout's content from its
    /// top-left `origin`; returns its size.
    fn layout(
        &self,
        list: &mut DrawList,
        s: &StyleResolver,
        origin: [f32; 2],
        paint: bool,
    ) -> [f32; 2] {
        let mono = mono(s);
        let sans = s.theme().font.clone();
        let x0 = origin[0] + READOUT_PAD[1];
        let mut y = origin[1] + READOUT_PAD[0];
        let mut width: f32 = 0.0;

        // Row 1: glyph + mode, axis chips or "free", snap.
        {
            let h = READOUT_ROWS[0];
            let base = row_baseline(list, y, h, READOUT_TITLE, sans.as_ref());
            let mut x = x0;
            #[cfg(feature = "phosphor-icons")]
            {
                use crate::PhosphorIcon;
                let icon = match self.mode() {
                    GizmoMode::Move => PhosphorIcon::ArrowsOutCardinal,
                    GizmoMode::Rotate => PhosphorIcon::ArrowClockwise,
                    GizmoMode::Scale => PhosphorIcon::ArrowsOut,
                };
                if paint {
                    let r = Rect::new(
                        x,
                        y + (h - READOUT_GLYPH) * 0.5,
                        READOUT_GLYPH,
                        READOUT_GLYPH,
                    );
                    list.phosphor_icon(r, icon, s.color(StyleKey::AccentTick));
                }
                x += READOUT_GLYPH + READOUT_GLYPH_GAP;
            }
            let label = self.mode_label();
            let lw = list
                .measure_text_with_font(label, READOUT_TITLE, None, sans.as_ref())
                .0;
            if paint {
                let ty = top_for_baseline(list, base, READOUT_TITLE, sans.as_ref());
                list.text(
                    TextBlock::new(label, x, ty)
                        .with_size(READOUT_TITLE)
                        .with_color_f32(s.ink(Ink::Title))
                        .with_shadow(0, 0, 0, CARVE_ALPHA, 0.0, -1.0, 0.0)
                        .with_font_opt(sans.clone()),
                );
            }
            x += lw + READOUT_TITLE_GAP;

            let small_top = top_for_baseline(list, base, READOUT_SMALL, mono.as_ref());
            match self.axes() {
                Some(axes) => {
                    for (i, _) in axes.iter().enumerate().filter(|(_, on)| **on) {
                        let letter = ["X", "Y", "Z"][i];
                        let w = list
                            .measure_text_with_font(letter, READOUT_SMALL, None, mono.as_ref())
                            .0;
                        let cw = w + READOUT_CHIP_PAD * 2.0;
                        if paint {
                            let chip = Rect::new(x, small_top - 1.0, cw, READOUT_SMALL * 1.3 + 1.0);
                            let tint = s.color(StyleKey::AXES[i]);
                            list.chrome_rect(chip, READOUT_CHIP_RADIUS, 0.0, tint, tint);
                            list.text(
                                TextBlock::new(letter, x + READOUT_CHIP_PAD, small_top)
                                    .with_size(READOUT_SMALL)
                                    .with_weight(crate::Weight::BOLD)
                                    .with_color_f32(CHIP_INK)
                                    .with_font_opt(mono.clone()),
                            );
                        }
                        x += cw + READOUT_CHIP_GAP;
                    }
                    x -= READOUT_CHIP_GAP;
                }
                None => {
                    let w = list
                        .measure_text_with_font("free", READOUT_SMALL, None, mono.as_ref())
                        .0;
                    if paint {
                        list.text(
                            s.mono_block(
                                "free",
                                x,
                                small_top,
                                crate::TextSize::Dense,
                                Ink::Caption,
                            )
                            .with_size(READOUT_SMALL),
                        );
                    }
                    x += w;
                }
            }
            if let Some(snap) = self.snap {
                x += READOUT_TITLE_GAP;
                let text = format!("snap {snap}");
                let w = list
                    .measure_text_with_font(&text, READOUT_SMALL, None, mono.as_ref())
                    .0;
                if paint {
                    list.text(
                        TextBlock::new(text, x, small_top)
                            .with_size(READOUT_SMALL)
                            .with_color_f32(s.color(StyleKey::AccentTick))
                            .with_font_opt(mono.clone()),
                    );
                }
                x += w;
            }
            width = width.max(x - x0);
            y += h + READOUT_ROW_GAP;
        }

        // Row 2: the values, or the number being typed.
        {
            let h = READOUT_ROWS[1];
            let base = row_baseline(list, y, h, READOUT_VALUE, mono.as_ref());
            let value_top = top_for_baseline(list, base, READOUT_VALUE, mono.as_ref());
            let small_top = top_for_baseline(list, base, READOUT_SMALL, mono.as_ref());
            let unit = self.unit_text();
            let mut x = x0;
            let part = |list: &mut DrawList,
                        x: &mut f32,
                        text: &str,
                        size: f32,
                        top: f32,
                        color: [f32; 4]| {
                let w = list
                    .measure_text_with_font(text, size, None, mono.as_ref())
                    .0;
                if paint {
                    list.text(
                        TextBlock::new(text, *x, top)
                            .with_size(size)
                            .with_color_f32(color)
                            .with_font_opt(mono.clone()),
                    );
                }
                *x += w;
            };
            if let Some(typed) = self.typed {
                part(
                    list,
                    &mut x,
                    typed,
                    READOUT_VALUE,
                    value_top,
                    s.ink(Ink::Max),
                );
                x += 1.0;
                if paint {
                    list.quad(
                        x,
                        base - READOUT_VALUE + 2.0,
                        1.0,
                        READOUT_VALUE,
                        s.color(StyleKey::Accent),
                    );
                }
                x += 1.0 + READOUT_PART_GAP;
                part(
                    list,
                    &mut x,
                    unit,
                    READOUT_VALUE,
                    value_top,
                    s.ink(Ink::Dim),
                );
            } else {
                let (shown, n) = self.shown();
                for (k, e) in shown[..n].iter().enumerate() {
                    if k > 0 {
                        x += READOUT_VALUE_GAP;
                    }
                    if let Some(i) = e.axis {
                        let tint = s.color(StyleKey::AXES[i]);
                        part(
                            list,
                            &mut x,
                            ["X", "Y", "Z"][i],
                            READOUT_SMALL,
                            small_top,
                            tint,
                        );
                        x += READOUT_PART_GAP;
                    }
                    let number = self.format(e.value);
                    part(
                        list,
                        &mut x,
                        &number,
                        READOUT_VALUE,
                        value_top,
                        s.ink(Ink::Value),
                    );
                    part(
                        list,
                        &mut x,
                        unit,
                        READOUT_VALUE,
                        value_top,
                        s.ink(Ink::Dim),
                    );
                }
            }
            width = width.max(x - x0);
            y += h;
        }

        // Row 3: the hints.
        if self.hints {
            y += READOUT_ROW_GAP;
            let h = READOUT_ROWS[2];
            let text = self.hint_text();
            let w = list
                .measure_text_with_font(text, READOUT_SMALL, None, mono.as_ref())
                .0;
            if paint {
                let base = row_baseline(list, y, h, READOUT_SMALL, mono.as_ref());
                let top = top_for_baseline(list, base, READOUT_SMALL, mono.as_ref());
                list.text(
                    TextBlock::new(text, x0, top)
                        .with_size(READOUT_SMALL)
                        .with_color_f32(s.ink(Ink::Caption))
                        .with_font_opt(mono.clone()),
                );
            }
            width = width.max(w);
            y += h;
        }

        [
            width + READOUT_PAD[1] * 2.0 + 2.0,
            y + READOUT_PAD[2] + 1.0 - origin[1],
        ]
    }

    /// The readout's size, in px.
    pub fn size(&self, list: &mut DrawList, s: &StyleResolver) -> [f32; 2] {
        self.layout(list, s, [0.0, 0.0], false)
    }

    /// Where the readout goes for a pointer at `pointer`: below and to the
    /// right, flipped to the other side of the pointer where it would leave
    /// `viewport`.
    pub fn rect(
        &self,
        list: &mut DrawList,
        s: &StyleResolver,
        pointer: [f32; 2],
        viewport: Rect,
    ) -> Rect {
        let [w, h] = self.size(list, s);
        let mut x = pointer[0] + READOUT_OFFSET[0];
        let mut y = pointer[1] + READOUT_OFFSET[1];
        if x + w > viewport.right() {
            x = pointer[0] - READOUT_OFFSET[0] - w;
        }
        if y + h > viewport.bottom() {
            y = pointer[1] - READOUT_OFFSET[1] - h;
        }
        Rect::new(x.max(viewport.x), y.max(viewport.y), w, h)
    }

    /// Draw the readout beside a pointer at `pointer`, inside `viewport`.
    /// Returns where it went.
    pub fn draw(
        &self,
        list: &mut DrawList,
        s: &StyleResolver,
        pointer: [f32; 2],
        viewport: Rect,
    ) -> Rect {
        let rect = self.rect(list, s, pointer, viewport);
        hud_glass(list, s, rect);
        self.layout(list, s, [rect.x + 1.0, rect.y + 1.0], true);
        rect
    }
}

// ---------------------------------------------------------------------------
// SelectionMarquee

/// How a marquee drag combines with the selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MarqueeMode {
    /// Replace the selection.
    #[default]
    Replace,
    /// Add to it (Shift).
    Add,
    /// Take away from it (Ctrl).
    Subtract,
}

/// The badge: 14 px tall, padded `0 5px`, 3 px off the corner, mono 9.
const BADGE_HEIGHT: f32 = 14.0;
const BADGE_PAD: f32 = 5.0;
const BADGE_MARGIN: f32 = 3.0;
const BADGE_GAP: f32 = 4.0;
const BADGE_SIZE: f32 = 9.0;
/// Dashes of a subtracting marquee's border.
const MARQUEE_DASH: [f32; 2] = [4.0, 3.0];

/// A rubber-band selection box (Forge `SelectionMarquee`), from where the
/// drag began to the pointer. Passive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionMarquee {
    from: [f32; 2],
    to: [f32; 2],
    mode: MarqueeMode,
    count: Option<usize>,
}

impl SelectionMarquee {
    /// A replacing marquee from `from` (where the drag began) to `to`.
    pub const fn new(from: [f32; 2], to: [f32; 2]) -> Self {
        Self {
            from,
            to,
            mode: MarqueeMode::Replace,
            count: None,
        }
    }

    /// How the drag combines with the selection.
    pub const fn mode(mut self, mode: MarqueeMode) -> Self {
        self.mode = mode;
        self
    }

    /// Show how many objects are inside so far.
    pub const fn count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }

    /// The box, normalised.
    pub fn rect(&self) -> Rect {
        let x = self.from[0].min(self.to[0]);
        let y = self.from[1].min(self.to[1]);
        Rect::new(
            x,
            y,
            (self.to[0] - self.from[0]).abs(),
            (self.to[1] - self.from[1]).abs(),
        )
    }

    fn badge_text(&self) -> Option<String> {
        self.count
            .map(|n| format!("{n} {}", if n == 1 { "object" } else { "objects" }))
    }

    fn sign(&self) -> Option<&'static str> {
        match self.mode {
            MarqueeMode::Replace => None,
            MarqueeMode::Add => Some("+"),
            MarqueeMode::Subtract => Some("−"),
        }
    }

    /// The badge's rect, outside the corner under the pointer, if there is a
    /// badge.
    pub fn badge_rect(&self, list: &mut DrawList, s: &StyleResolver) -> Option<Rect> {
        let sign = self.sign();
        let text = self.badge_text();
        if sign.is_none() && text.is_none() {
            return None;
        }
        let mono = mono(s);
        let mut w = BADGE_PAD * 2.0 + 2.0;
        if let Some(sign) = sign {
            w += list
                .measure_text_with_font(sign, BADGE_SIZE, None, mono.as_ref())
                .0;
        }
        if let Some(text) = &text {
            if sign.is_some() {
                w += BADGE_GAP;
            }
            w += list
                .measure_text_with_font(text, BADGE_SIZE, None, mono.as_ref())
                .0;
        }
        let r = self.rect();
        let x = if self.to[0] >= self.from[0] {
            r.right() + BADGE_MARGIN
        } else {
            r.x - BADGE_MARGIN - w
        };
        let y = if self.to[1] >= self.from[1] {
            r.bottom() + BADGE_MARGIN
        } else {
            r.y - BADGE_MARGIN - BADGE_HEIGHT
        };
        Some(Rect::new(x, y, w, BADGE_HEIGHT))
    }

    /// Draw the marquee and its badge.
    pub fn draw(&self, list: &mut DrawList, s: &StyleResolver) {
        let r = self.rect();
        let accent = s.color(StyleKey::Accent);
        let subtract = self.mode == MarqueeMode::Subtract;
        // The dark ring outside, so the box reads on a bright scene.
        list.chrome_rect(r.inset(-1.0), 0.0, 1.0, [0.0; 4], [0.0, 0.0, 0.0, 0.45]);
        let fill = if subtract {
            [0.0, 0.0, 0.0, 0.12]
        } else {
            s.color(StyleKey::AccentRing)
        };
        if subtract {
            list.quad(r.x, r.y, r.width, r.height, fill);
            let e = r.inset(0.5);
            list.stroke_closed(
                &[
                    [e.x, e.y],
                    [e.right(), e.y],
                    [e.right(), e.bottom()],
                    [e.x, e.bottom()],
                ],
                &Stroke::new(1.0).dashed(MARQUEE_DASH[0], MARQUEE_DASH[1]),
                accent,
            );
        } else {
            list.chrome_rect(r, 0.0, 1.0, fill, accent);
        }

        let Some(badge) = self.badge_rect(list, s) else {
            return;
        };
        hud_glass(list, s, badge);
        let mono = mono(s);
        let tick = s.color(StyleKey::AccentTick);
        let mut x = badge.x + 1.0 + BADGE_PAD;
        let y = list.vcentered_text_y(badge.y, badge.height, BADGE_SIZE, mono.as_ref(), "0");
        if let Some(sign) = self.sign() {
            let w = list
                .measure_text_with_font(sign, BADGE_SIZE, None, mono.as_ref())
                .0;
            list.text(
                TextBlock::new(sign, x, y)
                    .with_size(BADGE_SIZE)
                    .with_weight(crate::Weight::BOLD)
                    .with_color_f32(tick)
                    .with_font_opt(mono.clone()),
            );
            x += w + BADGE_GAP;
        }
        if let Some(text) = self.badge_text() {
            list.text(
                TextBlock::new(text, x, y)
                    .with_size(BADGE_SIZE)
                    .with_color_f32(tick)
                    .with_font_opt(mono),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// MeasureLine

/// The line: 1.5 px, dashed 6 on 3 off, over a soft dark halo.
const MEASURE_WIDTH: f32 = 1.5;
const MEASURE_DASH: [f32; 2] = [6.0, 3.0];
const MEASURE_HALO: [f32; 4] = [0.0, 0.0, 0.0, 0.45];
/// The end marks: an 8 px dot, or a diamond where the end snapped.
const END_SIZE: f32 = 8.0;
const END_EDGE: f32 = 1.5;
const END_FILL: [f32; 4] = [0.039, 0.051, 0.059, 0.6];
const END_RING: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
/// The label pill: mono 10, padded `1px 6px`.
const MEASURE_LABEL: f32 = 10.0;
const MEASURE_PAD: [f32; 2] = [6.0, 1.0];

/// A point-to-point measurement in screen space (Forge `MeasureLine`): a
/// dashed line between two ends, the distance on a pill at its middle.
/// Passive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeasureLine<'a> {
    a: [f32; 2],
    b: [f32; 2],
    label: Option<&'a str>,
    snapped: [bool; 2],
}

impl<'a> MeasureLine<'a> {
    /// A measurement from `a` to `b`.
    pub const fn new(a: [f32; 2], b: [f32; 2]) -> Self {
        Self {
            a,
            b,
            label: None,
            snapped: [false; 2],
        }
    }

    /// The world distance to show ("4.25 m").
    pub const fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Mark which ends snapped to geometry.
    pub const fn snapped(mut self, a: bool, b: bool) -> Self {
        self.snapped = [a, b];
        self
    }

    /// The label pill's rect, centred on the midpoint.
    pub fn label_rect(&self, list: &mut DrawList, s: &StyleResolver) -> Option<Rect> {
        let label = self.label?;
        let mono = mono(s);
        let (w, h) = list.measure_text_with_font(label, MEASURE_LABEL, None, mono.as_ref());
        let w = w + MEASURE_PAD[0] * 2.0 + 2.0;
        let h = h.max(MEASURE_LABEL) + MEASURE_PAD[1] * 2.0 + 2.0;
        let mid = [(self.a[0] + self.b[0]) * 0.5, (self.a[1] + self.b[1]) * 0.5];
        Some(Rect::new(mid[0] - w * 0.5, mid[1] - h * 0.5, w, h))
    }

    /// Draw the line, its ends and its label.
    pub fn draw(&self, list: &mut DrawList, s: &StyleResolver) {
        let dash = Stroke::new(MEASURE_WIDTH).dashed(MEASURE_DASH[0], MEASURE_DASH[1]);
        list.stroke_line(
            self.a,
            self.b,
            &Stroke::new(MEASURE_WIDTH + 2.0).dashed(MEASURE_DASH[0], MEASURE_DASH[1]),
            MEASURE_HALO,
        );
        list.stroke_line(self.a, self.b, &dash, WHITE);
        for (p, snapped) in [(self.a, self.snapped[0]), (self.b, self.snapped[1])] {
            draw_end(list, s, p, snapped);
        }
        if let (Some(label), Some(pill)) = (self.label, self.label_rect(list, s)) {
            hud_glass(list, s, pill);
            let mono = mono(s);
            let y = list.vcentered_text_y(pill.y, pill.height, MEASURE_LABEL, mono.as_ref(), label);
            list.text(
                TextBlock::new(label, pill.x + 1.0 + MEASURE_PAD[0], y)
                    .with_size(MEASURE_LABEL)
                    .with_color_f32(s.ink(Ink::Max))
                    .with_font_opt(mono),
            );
        }
    }
}

/// One end of a measure line: a dot, or an amber diamond where it snapped.
fn draw_end(list: &mut DrawList, s: &StyleResolver, p: [f32; 2], snapped: bool) {
    let half = END_SIZE * 0.5;
    if snapped {
        // The 8 px square turned 45°: corners `half·√2` out.
        let d = half * std::f32::consts::SQRT_2;
        let diamond = |r: f32| {
            [
                [p[0], p[1] - r],
                [p[0] + r, p[1]],
                [p[0], p[1] + r],
                [p[0] - r, p[1]],
            ]
        };
        list.stroke_closed(&diamond(d + 0.5), &Stroke::new(1.0), END_RING);
        let inner = diamond(d - END_EDGE * 0.5);
        list.filled_polygon(&inner.map(|q| (q[0], q[1])), s.color(StyleKey::WarnMeta));
        list.stroke_closed(&diamond(d - END_EDGE * 0.5), &Stroke::new(END_EDGE), WHITE);
    } else {
        let r = Rect::new(p[0] - half, p[1] - half, END_SIZE, END_SIZE);
        list.chrome_rect(r.inset(-1.0), half + 1.0, 1.0, [0.0; 4], END_RING);
        list.chrome_rect(r, half, END_EDGE, END_FILL, WHITE);
    }
}

// ---------------------------------------------------------------------------
// ViewportStats

/// The stats panel: padded `4px 8px 5px`, columns 12 px apart, rows 1 px
/// apart, mono 9.5; the title mono-caps 8.5, 2 px above the rows.
const STATS_PAD: [f32; 3] = [4.0, 8.0, 5.0];
const STATS_COLUMN_GAP: f32 = 12.0;
const STATS_ROW_GAP: f32 = 1.0;
const STATS_SIZE: f32 = 9.5;
const STATS_ROW: f32 = 12.0;
const STATS_TITLE: f32 = 8.5;
const STATS_TITLE_ROW: f32 = 11.0;
const STATS_TITLE_GAP: f32 = 2.0;

/// A stat's value: a number (grouped with commas) or text ("62 fps").
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StatValue<'a> {
    /// A count or amount.
    Number(f64),
    /// Preformatted text.
    Text(&'a str),
}

/// One row of [`ViewportStats`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatRow<'a> {
    label: &'a str,
    value: StatValue<'a>,
    budget: Option<f64>,
    warn: bool,
}

impl<'a> StatRow<'a> {
    /// A row showing a number.
    pub const fn number(label: &'a str, value: f64) -> Self {
        Self {
            label,
            value: StatValue::Number(value),
            budget: None,
            warn: false,
        }
    }

    /// A row showing text.
    pub const fn text(label: &'a str, value: &'a str) -> Self {
        Self {
            label,
            value: StatValue::Text(value),
            budget: None,
            warn: false,
        }
    }

    /// The budget: shown as " / N", and a number over it turns the row
    /// amber.
    pub const fn budget(mut self, budget: f64) -> Self {
        self.budget = Some(budget);
        self
    }

    /// Turn the row amber regardless of the budget.
    pub const fn warn(mut self, warn: bool) -> Self {
        self.warn = warn;
        self
    }

    /// True when the row is over budget or warned.
    pub fn is_hot(&self) -> bool {
        self.warn || matches!((self.value, self.budget), (StatValue::Number(v), Some(b)) if v > b)
    }
}

/// A number the way `toLocaleString("en-US")` prints it: commas between
/// thousands, at most three decimals, no trailing zeros.
pub(crate) fn group_thousands(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    let negative = rounded < 0.0;
    let abs = rounded.abs();
    let whole = abs.trunc() as u64;
    let mut frac = format!("{:.3}", abs - abs.trunc());
    // "0.250" → ".25"; "0.000" → "".
    while frac.ends_with('0') {
        frac.pop();
    }
    if frac.ends_with('.') {
        frac.pop();
    }
    let frac = frac.trim_start_matches('0');
    let digits = whole.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + frac.len() + 1);
    if negative {
        out.push('-');
    }
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out.push_str(frac);
    out
}

/// Corner stats over the viewport (Forge `ViewportStats`): label and value
/// columns on HUD glass; a row over its budget turns amber. Passive.
#[derive(Clone, Copy, Debug)]
pub struct ViewportStats<'a> {
    rows: &'a [StatRow<'a>],
    title: Option<&'a str>,
}

/// One row's text, formatted once per layout.
struct RowText {
    value: String,
    budget: Option<String>,
}

impl<'a> ViewportStats<'a> {
    /// A stats panel of `rows`.
    pub const fn new(rows: &'a [StatRow<'a>]) -> Self {
        Self { rows, title: None }
    }

    /// A mono-caps title above the rows.
    pub const fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    fn row_text(row: &StatRow) -> RowText {
        RowText {
            value: match row.value {
                StatValue::Number(v) => group_thousands(v),
                StatValue::Text(t) => t.to_string(),
            },
            budget: row.budget.map(|b| format!(" / {}", group_thousands(b))),
        }
    }

    fn title_block(&self, s: &StyleResolver, title: &str, x: f32, y: f32) -> TextBlock {
        TextBlock::new(title.to_uppercase(), x, y)
            .with_size(STATS_TITLE)
            .with_letter_spacing(STATS_TITLE * s.scalar(StyleKey::Tracking(Tracking::Caption)))
            .with_color_f32(s.ink(Ink::Glyph))
            .with_font_opt(mono(s))
    }

    /// The label and value column widths.
    fn columns(&self, list: &mut DrawList, s: &StyleResolver) -> [f32; 2] {
        let mono = mono(s);
        let mut cols = [0.0f32; 2];
        for row in self.rows {
            let text = Self::row_text(row);
            let label = list
                .measure_text_with_font(row.label, STATS_SIZE, None, mono.as_ref())
                .0;
            let mut value = list
                .measure_text_with_font(&text.value, STATS_SIZE, None, mono.as_ref())
                .0;
            if let Some(budget) = &text.budget {
                value += list
                    .measure_text_with_font(budget, STATS_SIZE, None, mono.as_ref())
                    .0;
            }
            cols[0] = cols[0].max(label);
            cols[1] = cols[1].max(value);
        }
        cols
    }

    /// The panel's size, in px.
    pub fn size(&self, list: &mut DrawList, s: &StyleResolver) -> [f32; 2] {
        let [l, v] = self.columns(list, s);
        let mut w = l + STATS_COLUMN_GAP + v;
        let mut h = self.rows.len() as f32 * STATS_ROW
            + self.rows.len().saturating_sub(1) as f32 * STATS_ROW_GAP;
        if let Some(title) = self.title {
            let block = self.title_block(s, title, 0.0, 0.0);
            w = w.max(list.measure_block(&block).0);
            h += STATS_TITLE_ROW + STATS_TITLE_GAP;
        }
        [
            w + STATS_PAD[1] * 2.0 + 2.0,
            h + STATS_PAD[0] + STATS_PAD[2] + 2.0,
        ]
    }

    /// Draw the panel with its top-left corner at `origin`; returns its rect.
    pub fn draw(&self, list: &mut DrawList, s: &StyleResolver, origin: [f32; 2]) -> Rect {
        let [w, h] = self.size(list, s);
        let rect = Rect::new(origin[0], origin[1], w, h);
        hud_glass(list, s, rect);
        let mono = mono(s);
        let x0 = rect.x + 1.0 + STATS_PAD[1];
        let right = rect.right() - 1.0 - STATS_PAD[1];
        let mut y = rect.y + 1.0 + STATS_PAD[0];
        if let Some(title) = self.title {
            let base = row_baseline(list, y, STATS_TITLE_ROW, STATS_TITLE, mono.as_ref());
            let top = top_for_baseline(list, base, STATS_TITLE, mono.as_ref());
            list.text(self.title_block(s, title, x0, top));
            y += STATS_TITLE_ROW + STATS_TITLE_GAP;
        }
        let warn_meta = s.color(StyleKey::WarnMeta);
        for row in self.rows {
            let hot = row.is_hot();
            let text = Self::row_text(row);
            let base = row_baseline(list, y, STATS_ROW, STATS_SIZE, mono.as_ref());
            let top = top_for_baseline(list, base, STATS_SIZE, mono.as_ref());
            let block = |t: String, x: f32, color: [f32; 4]| {
                TextBlock::new(t, x, top)
                    .with_size(STATS_SIZE)
                    .with_color_f32(color)
                    .with_font_opt(mono.clone())
            };
            let (label_ink, value_ink, budget_ink) = if hot {
                (warn_meta, s.color(StyleKey::WarnBannerInk), warn_meta)
            } else {
                (s.ink(Ink::Caption), s.ink(Ink::Value), s.ink(Ink::Disabled))
            };
            list.text(block(row.label.to_string(), x0, label_ink));
            // The value column is right-aligned.
            let mut x = right;
            if let Some(budget) = text.budget {
                let bw = list
                    .measure_text_with_font(&budget, STATS_SIZE, None, mono.as_ref())
                    .0;
                x -= bw;
                list.text(block(budget, x, budget_ink));
            }
            let vw = list
                .measure_text_with_font(&text.value, STATS_SIZE, None, mono.as_ref())
                .0;
            x -= vw;
            list.text(block(text.value, x, value_ink));
            y += STATS_ROW + STATS_ROW_GAP;
        }
        rect
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;
    use crate::color::text_color;

    fn shown(r: &TransformReadout) -> Vec<(Option<usize>, f32)> {
        let (s, n) = r.shown();
        s[..n].iter().map(|e| (e.axis, e.value)).collect()
    }

    #[test]
    fn the_readout_shows_the_constrained_axes_only() {
        let v = GizmoValue::Translate([1.0, 2.0, 3.0]);
        assert_eq!(
            shown(&TransformReadout::new(GizmoHandle::X, v)),
            [(Some(0), 1.0)]
        );
        assert_eq!(
            shown(&TransformReadout::new(GizmoHandle::XZ, v)),
            [(Some(0), 1.0), (Some(2), 3.0)]
        );
        assert_eq!(
            shown(&TransformReadout::new(GizmoHandle::Free, v)),
            [(Some(0), 1.0), (Some(1), 2.0), (Some(2), 3.0)]
        );
        assert_eq!(
            shown(&TransformReadout::new(
                GizmoHandle::Y,
                GizmoValue::Rotate(45.0)
            )),
            [(Some(1), 45.0)]
        );
        assert_eq!(
            shown(&TransformReadout::new(
                GizmoHandle::View,
                GizmoValue::Rotate(45.0)
            )),
            [(None, 45.0)]
        );
        assert_eq!(
            shown(&TransformReadout::new(
                GizmoHandle::Uniform,
                GizmoValue::Scale([1.5; 3])
            )),
            [(None, 1.5)]
        );
        assert_eq!(
            shown(&TransformReadout::new(
                GizmoHandle::Z,
                GizmoValue::Scale([1.0, 1.0, 2.0])
            )),
            [(Some(2), 2.0)]
        );
    }

    #[test]
    fn the_readout_signs_amounts_but_not_factors() {
        let m = TransformReadout::new(GizmoHandle::X, GizmoValue::Translate([0.0; 3]));
        assert_eq!(m.format(1.254), "+1.25");
        assert_eq!(m.format(-0.5), "−0.50");
        assert_eq!(m.format(0.0), "+0.00");
        assert_eq!(m.precision(1).format(2.0), "+2.0");
        let s = TransformReadout::new(GizmoHandle::Uniform, GizmoValue::Scale([1.0; 3]));
        assert_eq!(s.format(1.5), "1.50");
        assert_eq!(m.unit_text(), "m");
        assert_eq!(s.unit_text(), "×");
        assert_eq!(
            TransformReadout::new(GizmoHandle::View, GizmoValue::Rotate(0.0)).unit_text(),
            "°"
        );
        assert_eq!(m.unit("cm").unit_text(), "cm");
    }

    #[test]
    fn the_readout_paints_mode_axis_values_and_hints() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let readout =
            TransformReadout::new(GizmoHandle::X, GizmoValue::Translate([0.92, 0.0, 0.0]))
                .snap(Some("0.5 m"));
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let rect = readout.draw(&mut list, &s, [100.0, 100.0], viewport);
        assert_eq!([rect.x, rect.y], [116.0, 114.0]);
        assert_eq!([rect.width, rect.height], readout.size(&mut list, &s));
        let texts: Vec<_> = list.texts.iter().map(|t| t.content.as_str()).collect();
        for want in ["Move", "X", "+0.92", "m", "snap 0.5 m", readout.hint_text()] {
            assert!(texts.contains(&want), "{want:?} in {texts:?}");
        }
        assert!(!texts.contains(&"Y"), "only the constrained axis");
        for t in &list.texts {
            assert!(
                t.x >= rect.x && t.x <= rect.right(),
                "{:?} at {} outside {rect:?}",
                t.content,
                t.x
            );
        }
    }

    #[test]
    fn typing_replaces_the_values_and_the_hints() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let readout = TransformReadout::new(GizmoHandle::X, GizmoValue::Translate([0.5, 0.0, 0.0]))
            .typed(Some("2.5"));
        readout.draw(
            &mut list,
            &s,
            [10.0, 10.0],
            Rect::new(0.0, 0.0, 800.0, 600.0),
        );
        let texts: Vec<_> = list.texts.iter().map(|t| t.content.as_str()).collect();
        assert!(
            texts.contains(&"2.5") && !texts.contains(&"+0.50"),
            "{texts:?}"
        );
        assert!(texts.contains(&"Enter apply · Esc cancel"));
        let typed = list.texts.iter().find(|t| t.content == "2.5").unwrap();
        assert_eq!(typed.color, text_color(s.ink(Ink::Max)));
    }

    #[test]
    fn the_readout_flips_away_from_the_viewport_edges() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let readout = TransformReadout::new(GizmoHandle::X, GizmoValue::Translate([0.0; 3]));
        let viewport = Rect::new(0.0, 0.0, 400.0, 300.0);
        let r = readout.rect(&mut list, &s, [390.0, 290.0], viewport);
        assert_eq!(r.right(), 390.0 - 16.0);
        assert_eq!(r.bottom(), 290.0 - 14.0);
    }

    #[test]
    fn a_marquee_normalises_and_badges_outside_the_drag_corner() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let down_right = SelectionMarquee::new([10.0, 20.0], [110.0, 80.0]).count(3);
        assert_eq!(down_right.rect(), Rect::new(10.0, 20.0, 100.0, 60.0));
        let badge = down_right.badge_rect(&mut list, &s).unwrap();
        assert_eq!([badge.x, badge.y], [113.0, 83.0]);

        let up_left = SelectionMarquee::new([110.0, 80.0], [10.0, 20.0])
            .mode(MarqueeMode::Add)
            .count(1);
        assert_eq!(up_left.rect(), down_right.rect());
        let badge = up_left.badge_rect(&mut list, &s).unwrap();
        assert_eq!(badge.right(), 7.0);
        assert_eq!(badge.bottom(), 17.0);
        assert_eq!(up_left.badge_text().as_deref(), Some("1 object"));

        assert_eq!(
            SelectionMarquee::new([0.0, 0.0], [5.0, 5.0]).badge_rect(&mut list, &s),
            None,
            "a plain replace has nothing to say"
        );
        assert!(
            SelectionMarquee::new([0.0, 0.0], [5.0, 5.0])
                .mode(MarqueeMode::Subtract)
                .badge_rect(&mut list, &s)
                .is_some()
        );
    }

    #[test]
    fn a_marquee_paints_its_sign_and_count() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        SelectionMarquee::new([10.0, 20.0], [110.0, 80.0])
            .mode(MarqueeMode::Subtract)
            .count(12)
            .draw(&mut list, &s);
        let texts: Vec<_> = list.texts.iter().map(|t| t.content.as_str()).collect();
        assert_eq!(texts, ["−", "12 objects"]);
    }

    #[test]
    fn a_measure_label_centres_on_the_midpoint() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let line = MeasureLine::new([10.0, 10.0], [110.0, 60.0]).label("4.25 m");
        let r = line.label_rect(&mut list, &s).unwrap();
        assert!((r.x + r.width * 0.5 - 60.0).abs() < 1e-4);
        assert!((r.y + r.height * 0.5 - 35.0).abs() < 1e-4);
        line.snapped(true, false).draw(&mut list, &s);
        assert_eq!(list.texts.len(), 1);
        assert_eq!(list.texts[0].content, "4.25 m");
        assert_eq!(
            MeasureLine::new([0.0; 2], [1.0; 2]).label_rect(&mut list, &s),
            None
        );
    }

    #[test]
    fn numbers_group_like_en_us() {
        assert_eq!(group_thousands(1_234_567.0), "1,234,567");
        assert_eq!(group_thousands(999.0), "999");
        assert_eq!(group_thousands(1000.0), "1,000");
        assert_eq!(group_thousands(1234.5), "1,234.5");
        assert_eq!(group_thousands(0.25), "0.25");
        assert_eq!(group_thousands(2.0 / 3.0), "0.667");
        assert_eq!(group_thousands(-1234.0), "-1,234");
        assert_eq!(group_thousands(0.0), "0");
    }

    #[test]
    fn a_row_over_budget_or_warned_runs_hot() {
        assert!(
            StatRow::number("Tris", 1_200_000.0)
                .budget(1_000_000.0)
                .is_hot()
        );
        assert!(
            !StatRow::number("Tris", 900_000.0)
                .budget(1_000_000.0)
                .is_hot()
        );
        assert!(StatRow::text("GPU", "18 ms").warn(true).is_hot());
        assert!(
            !StatRow::text("GPU", "8 ms").budget(1.0).is_hot(),
            "text never compares"
        );
    }

    #[test]
    fn stats_right_align_values_and_colour_hot_rows() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let rows = [
            StatRow::number("Draw calls", 1840.0),
            StatRow::number("Tris", 1_200_000.0).budget(1_000_000.0),
        ];
        let stats = ViewportStats::new(&rows).title("Scene");
        let rect = stats.draw(&mut list, &s, [10.0, 10.0]);
        assert_eq!([rect.width, rect.height], stats.size(&mut list, &s));
        let find = |c: &str| list.texts.iter().find(|t| t.content == c).unwrap();
        assert_eq!(find("SCENE").color, text_color(s.ink(Ink::Glyph)));
        assert_eq!(find("1,840").color, text_color(s.ink(Ink::Value)));
        assert_eq!(
            find("1,200,000").color,
            text_color(s.color(StyleKey::WarnBannerInk))
        );
        assert_eq!(find("Tris").color, text_color(s.color(StyleKey::WarnMeta)));
        assert_eq!(
            find(" / 1,000,000").color,
            text_color(s.color(StyleKey::WarnMeta))
        );
        // Both values end at the same right edge.
        let budget = find(" / 1,000,000");
        let calls = find("1,840");
        let mut m = DrawList::new();
        let mono = s.theme().mono_font.clone();
        let budget_w = m
            .measure_text_with_font(&budget.content, STATS_SIZE, None, mono.as_ref())
            .0;
        let calls_w = m
            .measure_text_with_font(&calls.content, STATS_SIZE, None, mono.as_ref())
            .0;
        assert!((budget.x + budget_w - (calls.x + calls_w)).abs() < 1e-3);
    }
}
