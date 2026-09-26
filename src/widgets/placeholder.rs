//! Placeholder — marks where real content will go while designing (Forge
//! `Placeholder`).

use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};

use super::DrawList;

/// Height of one text bar, and the gap between rows.
const BAR: f32 = 6.0;
/// Space above and below the text kind's column.
const TEXT_PAD: f32 = 2.0;
/// Text bar widths, as fractions of the width, cycling; the last bar of a
/// multi-line block is always [`LAST_BAR`].
const BAR_WIDTHS: [f32; 6] = [1.0, 0.92, 0.97, 0.64, 0.88, 0.76];
const LAST_BAR: f32 = 0.58;
const BAR_FILL: [f32; 4] = [1.0, 1.0, 1.0, 0.07];
const BAR_SHADE: [f32; 4] = [0.0, 0.0, 0.0, 0.3];
/// Caption letter spacing, in em.
const CAPTION_TRACKING: f32 = 0.08;
/// The image kind's dashed edge, hatch, and crossed frame.
const EDGE: [f32; 4] = [1.0, 1.0, 1.0, 0.16];
const DASH: f32 = 3.0;
const HATCH: [f32; 4] = [1.0, 1.0, 1.0, 0.035];
/// The hatch repeats every 6 px along its 135° axis: `6·√2` px across.
const HATCH_STEP: f32 = 6.0 * std::f32::consts::SQRT_2;
const CROSS: [f32; 4] = [1.0, 1.0, 1.0, 0.06];
/// The recess (`--well-inset-tall`): a shade inside and a lit lip below.
const RECESS: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
const LIP: [f32; 4] = [1.0, 1.0, 1.0, 0.07];
/// The caption chip on the image kind.
const CHIP_FILL: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
const CHIP_PAD: (f32, f32) = (6.0, 2.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Image { round: bool },
    Text { lines: usize },
}

/// Marks where real content will go while designing (Forge `Placeholder`).
///
/// Two kinds:
///
/// - [`image`](Placeholder::image): a hatched well with a crossed frame, a
///   dashed edge and a small mono caption (`IMAGE` unless you give one).
///   [`round`](Placeholder::round) makes it a circle, for avatars.
/// - [`text`](Placeholder::text): a few flat bars standing in for lines of
///   copy, with an optional caption above.
///
/// It is static on purpose: use [`skeleton`](super::skeleton) for "loading".
/// For a tiny empty slot inside a row, use an empty `Thumb` slot.
///
/// ```ignore
/// Placeholder::image().label("Level preview · 16:9").draw(rect, list, &style);
/// let h = Placeholder::text(3).label("Description").height(list, &style);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placeholder<'a> {
    kind: Kind,
    label: Option<&'a str>,
}

impl<'a> Placeholder<'a> {
    /// An image stand-in, captioned `IMAGE`.
    pub fn image() -> Self {
        Self {
            kind: Kind::Image { round: false },
            label: None,
        }
    }

    /// A stand-in for `lines` lines of text (at least one).
    pub fn text(lines: usize) -> Self {
        Self {
            kind: Kind::Text {
                lines: lines.max(1),
            },
            label: None,
        }
    }

    /// The caption (drawn upper-case). On an image an empty string hides
    /// the caption chip.
    #[must_use]
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Make an image stand-in a circle. No effect on text.
    #[must_use]
    pub fn round(mut self, round: bool) -> Self {
        if let Kind::Image { .. } = self.kind {
            self.kind = Kind::Image { round };
        }
        self
    }

    fn caption_block(&self, s: &StyleResolver, text: &str, role: Ink) -> crate::text::TextBlock {
        let size = s.text_size(TextSize::Caption);
        s.mono_block(text.to_uppercase(), 0.0, 0.0, TextSize::Caption, role)
            .with_letter_spacing(size * CAPTION_TRACKING)
    }

    /// The height the text kind needs; an image takes whatever it's given.
    pub fn height(&self, list: &mut DrawList, s: &StyleResolver) -> f32 {
        match self.kind {
            Kind::Image { .. } => 0.0,
            Kind::Text { lines } => {
                let caption = match self.label {
                    Some(label) if !label.is_empty() => {
                        list.measure_block(&self.caption_block(s, label, Ink::Dim))
                            .1
                            + BAR
                    }
                    _ => 0.0,
                };
                2.0 * TEXT_PAD + caption + lines as f32 * BAR * 2.0 - BAR
            }
        }
    }

    /// Draw the stand-in in `rect`. Text bars that don't fit are left out,
    /// and an empty rect draws nothing.
    pub fn draw(&self, rect: Rect, list: &mut DrawList, s: &StyleResolver) {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        list.push_debug_scope_rect(
            super::scope_name("Placeholder", self.label.unwrap_or_default()),
            rect,
        );
        match self.kind {
            Kind::Image { round } => self.draw_image(rect, round, list, s),
            Kind::Text { lines } => self.draw_text(rect, lines, list, s),
        }
        list.pop_debug_scope();
    }

    fn draw_text(&self, rect: Rect, lines: usize, list: &mut DrawList, s: &StyleResolver) {
        let mut y = rect.y + TEXT_PAD;
        if let Some(label) = self.label.filter(|l| !l.is_empty()) {
            let mut block = self.caption_block(s, label, Ink::Dim);
            block.x = rect.x;
            block.y = y;
            y += list.measure_block(&block).1 + BAR;
            list.text(block.with_clip(rect));
        }
        for i in 0..lines {
            if y + BAR > rect.bottom() {
                break;
            }
            let frac = if lines > 1 && i == lines - 1 {
                LAST_BAR
            } else {
                BAR_WIDTHS[i % BAR_WIDTHS.len()]
            };
            let bar = Rect::new(rect.x, y, (rect.width * frac).round(), BAR);
            list.rounded_rect(bar, 1.0, BAR_FILL);
            list.box_shadow_inset(
                bar,
                CornerRadii::uniform(1.0),
                BoxShadow {
                    offset: [0.0, -1.0],
                    color: BAR_SHADE,
                    inset: true,
                    ..BoxShadow::default()
                },
            );
            y += BAR * 2.0;
        }
    }

    fn draw_image(&self, rect: Rect, round: bool, list: &mut DrawList, s: &StyleResolver) {
        if rect.width < 2.0 || rect.height < 2.0 {
            return;
        }
        let radius = if round {
            rect.width.min(rect.height) * 0.5
        } else {
            s.scalar(StyleKey::BorderRadius)
        };
        let radii = CornerRadii::uniform(radius);
        list.box_shadow_outset(
            rect,
            radii,
            BoxShadow {
                offset: [0.0, 1.0],
                color: LIP,
                ..BoxShadow::default()
            },
        );
        list.rounded_rect(rect, radius, s.color(StyleKey::WellDeep));
        list.box_shadow_inset(
            rect,
            radii,
            BoxShadow {
                offset: [0.0, 2.0],
                blur: 5.0,
                color: RECESS,
                inset: true,
                ..BoxShadow::default()
            },
        );
        // Strokes run half a pixel inside the edge so their width stays in.
        let inner = rect.inset(0.5);
        let (tl, tr) = ([inner.x, inner.y], [inner.right(), inner.y]);
        let (bl, br) = ([inner.x, inner.bottom()], [inner.right(), inner.bottom()]);
        if round {
            let c = [rect.x + rect.width * 0.5, rect.y + rect.height * 0.5];
            let r = radius - 0.5;
            let mut offset = 0.0;
            while offset < rect.width + rect.height {
                let p0 = [rect.x + offset - rect.height, rect.bottom()];
                let p1 = [rect.x + offset, rect.y];
                if let Some((a, b)) = clip_to_circle(p0, p1, c, r) {
                    list.line(a, b, 1.0, HATCH);
                }
                offset += HATCH_STEP;
            }
            for (p0, p1) in [(tl, br), (tr, bl)] {
                if let Some((a, b)) = clip_to_circle(p0, p1, c, r) {
                    list.line(a, b, 1.0, CROSS);
                }
            }
            dashed_circle(list, c, r, EDGE);
        } else {
            list.hatch(rect, HATCH_STEP, HATCH);
            list.line(tl, br, 1.0, CROSS);
            list.line(tr, bl, 1.0, CROSS);
            list.dashed_rect_outline(rect, DASH, EDGE);
        }
        let caption = self.label.unwrap_or("Image");
        if !caption.is_empty() {
            let mut block = self.caption_block(s, caption, Ink::Caption);
            let (w, h) = list.measure_block(&block);
            let chip = Rect::new(
                (rect.x + (rect.width - w) * 0.5 - CHIP_PAD.0).round(),
                (rect.y + (rect.height - h) * 0.5 - CHIP_PAD.1).round(),
                w + 2.0 * CHIP_PAD.0,
                h + 2.0 * CHIP_PAD.1,
            );
            list.rounded_rect(chip, 2.0, CHIP_FILL);
            block.x = chip.x + CHIP_PAD.0;
            block.y = chip.y + CHIP_PAD.1;
            list.text(block.with_clip(rect));
        }
    }
}

/// The part of the segment `p0`–`p1` inside the circle at `c` with radius
/// `r`, or `None` when it misses.
fn clip_to_circle(p0: [f32; 2], p1: [f32; 2], c: [f32; 2], r: f32) -> Option<([f32; 2], [f32; 2])> {
    let d = [p1[0] - p0[0], p1[1] - p0[1]];
    let f = [p0[0] - c[0], p0[1] - c[1]];
    let a = d[0] * d[0] + d[1] * d[1];
    if a <= f32::EPSILON {
        return None;
    }
    let b = 2.0 * (f[0] * d[0] + f[1] * d[1]);
    let k = f[0] * f[0] + f[1] * f[1] - r * r;
    let disc = b * b - 4.0 * a * k;
    if disc <= 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let t0 = ((-b - root) / (2.0 * a)).max(0.0);
    let t1 = ((-b + root) / (2.0 * a)).min(1.0);
    if t1 <= t0 {
        return None;
    }
    let at = |t: f32| [p0[0] + d[0] * t, p0[1] + d[1] * t];
    Some((at(t0), at(t1)))
}

/// A 1 px dashed circle: [`DASH`]-long arcs with equal gaps.
fn dashed_circle(list: &mut DrawList, c: [f32; 2], r: f32, color: [f32; 4]) {
    if r <= 0.0 {
        return;
    }
    let circumference = std::f32::consts::TAU * r;
    let dashes = (circumference / (DASH * 2.0)).floor().max(1.0);
    let step = std::f32::consts::TAU / dashes;
    let on = step * 0.5;
    let point = |a: f32| [c[0] + r * a.cos(), c[1] + r * a.sin()];
    for i in 0..dashes as usize {
        let a0 = i as f32 * step;
        // Two chords per dash keep a small dash on a big circle round.
        let mid = a0 + on * 0.5;
        list.line(point(a0), point(mid), 1.0, color);
        list.line(point(mid), point(a0 + on), 1.0, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    #[test]
    fn text_height_counts_caption_and_bars() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let bare = Placeholder::text(3).height(&mut list, &s);
        assert_eq!(bare, 2.0 * TEXT_PAD + 3.0 * BAR + 2.0 * BAR);
        let captioned = Placeholder::text(3)
            .label("Description")
            .height(&mut list, &s);
        assert!(captioned > bare + BAR);
    }

    #[test]
    fn last_text_bar_is_short() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let rect = Rect::new(0.0, 0.0, 200.0, 60.0);
        Placeholder::text(3).draw(rect, &mut list, &s);
        let widths: Vec<f32> = list
            .chrome_instances()
            .filter(|c| c.rect[3] == BAR)
            .map(|c| c.rect[2])
            .collect();
        assert_eq!(widths, vec![200.0, 184.0, 116.0]);
    }

    #[test]
    fn bars_that_do_not_fit_are_left_out() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        // Room for the padding and two bars (2 + 6 + 6 + 6), not a third.
        let rect = Rect::new(0.0, 0.0, 100.0, 22.0);
        Placeholder::text(5).draw(rect, &mut list, &s);
        let bars = list.chrome_instances().filter(|c| c.rect[3] == BAR).count();
        assert_eq!(bars, 2);
        for c in list.chrome_instances() {
            assert!(c.rect[1] + c.rect[3] <= rect.bottom());
        }
        let mut list = DrawList::new();
        Placeholder::text(3).draw(Rect::new(0.0, 0.0, 100.0, 0.0), &mut list, &s);
        assert_eq!(list.chrome_instance_count(), 0);
    }

    #[test]
    fn image_caption_defaults_to_image_and_empty_hides_it() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let rect = Rect::new(0.0, 0.0, 160.0, 90.0);
        let mut list = DrawList::new();
        Placeholder::image().draw(rect, &mut list, &s);
        assert_eq!(list.texts[0].content, "IMAGE");
        let mut list = DrawList::new();
        Placeholder::image().label("").draw(rect, &mut list, &s);
        assert!(list.texts.is_empty());
    }

    #[test]
    fn circle_clip_keeps_chords_inside() {
        let c = [10.0, 10.0];
        let (a, b) = clip_to_circle([0.0, 10.0], [20.0, 10.0], c, 5.0).expect("hits");
        assert!((a[0] - 5.0).abs() < 1e-4 && (b[0] - 15.0).abs() < 1e-4);
        assert!(clip_to_circle([0.0, 0.0], [20.0, 0.0], c, 5.0).is_none());
    }
}
