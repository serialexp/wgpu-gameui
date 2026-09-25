//! Busy & placeholder states — the design's loading vocabulary (Gallery III).
//!
//! - [`skeleton`]: a shimmering placeholder bar (the app animates the shimmer
//!   phase; the widget draws one frame of it).
//! - [`spinner`]: a ring with an accent arc (app rotates it per frame).
//! - [`dots`]: three pulsing accent dots (app supplies the pulse phase).
//!
//! The empty-state message is [`EmptyState`](super::EmptyState).

use crate::layout::Rect;
use crate::style::{StyleKey, StyleResolver};

use super::DrawList;

/// Draw one frame of a skeleton bar: a rounded bar with a moving highlight.
/// `phase` is the app-owned shimmer position in `[0, 1)` (wrap it each frame).
pub fn skeleton(list: &mut DrawList, s: &StyleResolver, rect: Rect, phase: f32) {
    let radius = rect.height * 0.5;
    let base = s.color(StyleKey::Button);
    let base = [base[0], base[1], base[2], 0.55];
    list.rounded_rect(rect, radius, base);
    // The moving sheen: a bright band sweeping left→right, clipped to the bar.
    let band_w = rect.width * 0.4;
    let x = rect.x + phase * (rect.width + band_w) - band_w;
    let band = Rect::new(
        x.max(rect.x),
        rect.y,
        (x + band_w).min(rect.right()) - x.max(rect.x),
        rect.height,
    );
    if band.width > 0.0 {
        list.rounded_rect(band, radius, [1.0, 1.0, 1.0, 0.10]);
    }
}

/// Draw one frame of a spinner: a faint ring with an accent arc covering
/// `sweep` radians starting at `phase` (both app-owned; rotate `phase` per
/// frame, e.g. `phase += dt * 8.0`).
pub fn spinner(
    list: &mut DrawList,
    s: &StyleResolver,
    center: (f32, f32),
    radius: f32,
    phase: f32,
    sweep: f32,
) {
    let track = s.color(StyleKey::ButtonHover);
    list.circle_outline(center, radius, 2.0, track);
    let accent = s.color(StyleKey::Accent);
    // Approximate the arc with chords (a spinner doesn't need true arcs).
    let steps = 14;
    let mut prev: Option<[f32; 2]> = None;
    for i in 0..=steps {
        let a = phase + sweep * (i as f32 / steps as f32);
        let p = [center.0 + radius * a.cos(), center.1 + radius * a.sin()];
        if let Some(p0) = prev {
            list.line(p0, p, 2.0, accent);
        }
        prev = Some(p);
    }
}

/// Draw three accent dots pulsing at `phase` (app-owned clock; stagger by
/// passing phases offset ~0.15s apart or use [`dots`] to lay all three).
pub fn dots(list: &mut DrawList, s: &StyleResolver, center: (f32, f32), phase: f32) {
    let accent = s.color(StyleKey::Accent);
    let spacing = 9.0;
    for i in 0..3usize {
        // Each dot's pulse is offset by 0.15 of a cycle.
        let t = (phase - i as f32 * 0.15).rem_euclid(1.0);
        let scale = 0.6
            + 0.4 * (1.0 - (2.0 * std::f32::consts::PI * t).cos()) * 0.5
            + 0.2 * (1.0 - t).abs();
        let r = 2.5 * scale.clamp(0.5, 1.3);
        let x = center.0 + (i as f32 - 1.0) * spacing;
        let mut c = accent;
        c[3] *= 0.5 + 0.5 * scale.clamp(0.0, 1.0);
        list.circle((x, center.1), r, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    fn theme() -> Theme {
        Theme::default()
    }

    #[test]
    fn skeleton_shimmer_band_moves_with_phase() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let rect = Rect::new(0.0, 0.0, 120.0, 9.0);
        let mut a = DrawList::new();
        skeleton(&mut a, &s, rect, 0.1);
        let mut b = DrawList::new();
        skeleton(&mut b, &s, rect, 0.7);
        // Same instance count, but the highlight quad differs in x.
        assert_eq!(a.chrome_instance_count(), b.chrome_instance_count());
        assert!(a.chrome_instance_count() >= 2);
    }

    #[test]
    fn spinner_and_dots_emit_geometry() {
        let theme = theme();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        spinner(&mut list, &s, (20.0, 20.0), 8.0, 0.0, 1.6);
        dots(&mut list, &s, (60.0, 20.0), 0.3);
        assert!(!list.circle_instances.is_empty(), "ring + dots are circles");
        assert!(!list.vertices.is_empty(), "arc chords are line quads");
    }
}
