//! Small bar readouts from Forge's app chrome.
//!
//! - [`inline_meter`]: a 5px sunken bar with one fill, sized for a line of
//!   mono text (a status bar's memory readout, a header chip's rate limit).
//!   It can carry a pace tick (where the fill "should" be by now).
//! - [`stacked_bar`]: a sunken bar split into coloured segments (a context
//!   window's categories, an output breakdown), with an optional marker line.

use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{Ink, StyleResolver};

use super::DrawList;

/// Height of an [`inline_meter`] in a line of text.
pub const INLINE_METER_HEIGHT: f32 = 5.0;

/// The sunken track every bar here sits in: `rgba(0,0,0,.55)`, an inner
/// shadow `inset 0 1px 2px rgba(0,0,0,.7)`, and a light line under it.
const TRACK: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
const TRACK_SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.7];
const TRACK_LIP: [f32; 4] = [1.0, 1.0, 1.0, 0.05];
/// The lit top line of a fill (`inset 0 1px 0 rgba(255,255,255,.2)`).
const FILL_HI: [f32; 4] = [1.0, 1.0, 1.0, 0.2];
/// The line between two segments of a stacked bar.
const SEGMENT_EDGE: [f32; 4] = [0.0, 0.0, 0.0, 0.5];
/// A segment's lit top line (`inset 0 1px 0 rgba(255,255,255,.18)`).
const SEGMENT_HI: [f32; 4] = [1.0, 1.0, 1.0, 0.18];

fn track(list: &mut DrawList, s: &StyleResolver, r: Rect, blur: f32) {
    let radius = s.scalar(crate::StyleKey::BorderRadius);
    list.chrome_rect(r, radius, 0.0, TRACK, [0.0; 4]);
    list.box_shadow_inset(
        r,
        CornerRadii::uniform(radius),
        BoxShadow {
            offset: [0.0, 1.0],
            blur,
            color: TRACK_SHADOW,
            inset: true,
            ..BoxShadow::default()
        },
    );
    list.quad(r.x, r.bottom(), r.width, 1.0, TRACK_LIP);
}

/// A fill for [`inline_meter`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterFill {
    /// How full, `0.0..=1.0` (clamped).
    pub fraction: f32,
    /// The fill's colour.
    pub color: [f32; 4],
    /// A pace tick at this fraction, if any: a 1px `--ink-row` line at 60%.
    pub pace: Option<f32>,
}

impl MeterFill {
    /// A fill of `fraction` in the neutral `--ink-glyph`.
    pub fn neutral(fraction: f32, s: &StyleResolver) -> Self {
        Self {
            fraction,
            color: s.ink(Ink::Glyph),
            pace: None,
        }
    }

    /// A fill of `fraction` in `color`.
    pub fn colored(fraction: f32, color: [f32; 4]) -> Self {
        Self {
            fraction,
            color,
            pace: None,
        }
    }

    /// Add a pace tick at `fraction`.
    pub fn pace(mut self, fraction: f32) -> Self {
        self.pace = Some(fraction);
        self
    }
}

/// Draw a meter filling `rect` (Forge's usage bars: 5px in a status bar or
/// chip, 7px in a popover): a sunken track, the fill from the left with a
/// lit top line, and the pace tick over it.
pub fn inline_meter(list: &mut DrawList, s: &StyleResolver, rect: Rect, fill: MeterFill) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    track(list, s, rect, 2.0);
    let w = rect.width * fill.fraction.clamp(0.0, 1.0);
    if w > 0.0 {
        list.quad(rect.x, rect.y, w, rect.height, fill.color);
        list.quad(rect.x, rect.y, w, 1.0, FILL_HI);
    }
    if let Some(pace) = fill.pace {
        let mut ink = s.ink(Ink::Row);
        ink[3] *= 0.6;
        let x = rect.x + rect.width * pace.clamp(0.0, 1.0);
        list.quad(x.min(rect.right() - 1.0), rect.y, 1.0, rect.height, ink);
    }
}

/// One segment of a [`stacked_bar`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarSegment {
    /// The segment's share of the bar's width.
    pub fraction: f32,
    /// Its colour.
    pub color: [f32; 4],
}

/// Draw a stacked bar filling `rect` (Forge's context bar and output
/// breakdown): segments left to right, each with a lit top line and a dark
/// line on its right; what they leave empty shows the sunken track. A
/// `marker` `(fraction, color)` draws a 1px line one pixel taller than the
/// bar at each end (the auto-compact point). Segments past the end are cut.
pub fn stacked_bar(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    segments: &[BarSegment],
    marker: Option<(f32, [f32; 4])>,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    track(list, s, rect, 3.0);
    let mut x = rect.x;
    for segment in segments {
        let w = (rect.width * segment.fraction.max(0.0)).min(rect.right() - x);
        if w <= 0.0 {
            break;
        }
        list.quad(x, rect.y, w, rect.height, segment.color);
        list.quad(x, rect.y, w, 1.0, SEGMENT_HI);
        list.quad(x + w - 1.0, rect.y, 1.0, rect.height, SEGMENT_EDGE);
        x += w;
    }
    if let Some((at, color)) = marker {
        let mx = rect.x + rect.width * at.clamp(0.0, 1.0);
        list.quad(mx, rect.y - 1.0, 1.0, rect.height + 2.0, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    /// The rects of the solid quads painted in `color`.
    fn rects_of(list: &DrawList, color: [f32; 4]) -> Vec<[f32; 4]> {
        list.chrome_instances()
            .filter(|c| c.bg == color && c.bg2 == color)
            .map(|c| c.rect)
            .collect()
    }

    #[test]
    fn a_meter_fills_its_fraction_and_marks_its_pace() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let fill = [0.2, 0.4, 0.6, 1.0];
        inline_meter(
            &mut list,
            &s,
            Rect::new(10.0, 0.0, 100.0, INLINE_METER_HEIGHT),
            MeterFill::colored(0.25, fill).pace(0.5),
        );
        assert_eq!(
            rects_of(&list, fill),
            vec![[10.0, 0.0, 25.0, INLINE_METER_HEIGHT]],
            "a quarter of the track"
        );
        let mut pace = s.ink(Ink::Row);
        pace[3] *= 0.6;
        assert_eq!(
            rects_of(&list, pace),
            vec![[60.0, 0.0, 1.0, INLINE_METER_HEIGHT]]
        );
        assert_eq!(list.shadow_instance_count(), 1, "the sunken track");
    }

    #[test]
    fn a_meter_clamps_and_skips_an_empty_fill() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let fill = [0.2, 0.4, 0.6, 1.0];
        let mut list = DrawList::new();
        inline_meter(
            &mut list,
            &s,
            Rect::new(0.0, 0.0, 50.0, 5.0),
            MeterFill::colored(0.0, fill),
        );
        assert!(rects_of(&list, fill).is_empty());
        let mut over = DrawList::new();
        inline_meter(
            &mut over,
            &s,
            Rect::new(0.0, 0.0, 50.0, 5.0),
            MeterFill::colored(3.0, fill),
        );
        assert_eq!(
            rects_of(&over, fill),
            vec![[0.0, 0.0, 50.0, 5.0]],
            "clamped to the track"
        );
    }

    #[test]
    fn segments_run_left_to_right_and_stop_at_the_end() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let a = [1.0, 0.0, 0.0, 1.0];
        let b = [0.0, 1.0, 0.0, 1.0];
        let c = [0.0, 0.0, 1.0, 1.0];
        let marker = [1.0, 1.0, 0.0, 1.0];
        stacked_bar(
            &mut list,
            &s,
            Rect::new(0.0, 0.0, 200.0, 6.0),
            &[
                BarSegment {
                    fraction: 0.25,
                    color: a,
                },
                BarSegment {
                    fraction: 0.5,
                    color: b,
                },
                BarSegment {
                    fraction: 0.5,
                    color: c,
                },
            ],
            Some((0.8, marker)),
        );
        assert_eq!(rects_of(&list, a), vec![[0.0, 0.0, 50.0, 6.0]]);
        assert_eq!(rects_of(&list, b), vec![[50.0, 0.0, 100.0, 6.0]]);
        assert_eq!(
            rects_of(&list, c),
            vec![[150.0, 0.0, 50.0, 6.0]],
            "cut at the end"
        );
        assert_eq!(
            rects_of(&list, marker),
            vec![[160.0, -1.0, 1.0, 8.0]],
            "a pixel past each end"
        );
        assert_eq!(
            rects_of(&list, SEGMENT_EDGE).len(),
            3,
            "a line right of each"
        );
    }
}
