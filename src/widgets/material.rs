//! The "4a" control material: a face plate resting on a plinth.
//!
//! Every raised control in the default design language is painted from the same
//! small vocabulary (see [`Theme`]'s material tokens):
//!
//! - a near-black **plinth** filling the control's full rect,
//! - a **face** inset `travel` px from the bottom (unpressed) — a vertical
//!   gradient (white sheen idly, or the accent/danger gradient for stateful
//!   tones) with a 1px near-black border edge,
//! - a 1px **inset highlight** under the face's top edge — replaced while
//!   pressed by a short dark shadow where the face has dropped onto the plinth,
//!
//! Pressing is geometric, not a color swap: the face drops by
//! [`StyleKey::Travel`] px so the plinth peeks out above it. Sunken surfaces
//! (input wells, tracks, the sunken tone) invert the model: dark fill, an inset
//! shadow under the top edge, and a faint light line beneath the bottom edge.
//!
//! Colors resolve through the [`StyleResolver`] (so a [`StyleOverlay`] retunes
//! any piece per-subtree). The inset shadow's depth is the
//! [`StyleKey::InnerShadowDepth`] scalar; the remaining band widths and border
//! alphas are language constants of the design, not theme fields.

use crate::DrawList;
use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{StyleKey, StyleResolver};

/// Which face a control wears.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    /// Raised neutral face (the standard button).
    #[default]
    Default,
    /// Raised accent-gradient face (primary/stateful controls). Text should
    /// resolve to [`StyleKey::OnAccent`].
    Accent,
    /// Raised danger-gradient face (destructive controls).
    Danger,
    /// A transparent face on the plinth: at rest the key is a dark square,
    /// and hover/press only lighten the face a little (header, field and
    /// inline keys). With [`Material::hollow`] there is no plinth either, so
    /// the face only appears on hover/press.
    Ghost,
    /// Sunken: dark fill with an inset shadow (checkbox troughs, toggled-off
    /// chips, pressed-in strips).
    Sunken,
}

/// One control's resolved material state.
pub struct Material {
    /// Which face the control wears.
    pub tone: Tone,
    /// Disabled controls fade to a fraction of their material (see
    /// `DISABLED_ALPHA`).
    pub enabled: bool,
    /// Hovered: face resolves the hover tokens.
    pub hovered: bool,
    /// Pressed: the face drops `travel` px onto the plinth.
    pub pressed: bool,
    /// How far the face drops, in px. `None` uses [`StyleKey::Travel`].
    pub travel: Option<f32>,
    /// No plinth under the face (the design's `hollow` keys: steppers and
    /// keys sunk in a well). The face still drops by `travel` while pressed.
    pub hollow: bool,
}

impl Material {
    /// A material in `tone`, enabled, at rest.
    #[must_use]
    pub fn new(tone: Tone) -> Self {
        Self {
            tone,
            enabled: true,
            hovered: false,
            pressed: false,
            travel: None,
            hollow: false,
        }
    }

    /// Leave out the plinth (builder style); see [`hollow`](Self::hollow).
    #[must_use]
    pub fn hollow(mut self, hollow: bool) -> Self {
        self.hollow = hollow;
        self
    }

    /// Set the enabled flag (builder style).
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Set the hovered flag (builder style).
    #[must_use]
    pub fn hovered(mut self, hovered: bool) -> Self {
        self.hovered = hovered;
        self
    }

    /// Set the pressed flag (builder style).
    #[must_use]
    pub fn pressed(mut self, pressed: bool) -> Self {
        self.pressed = pressed;
        self
    }

    /// Drop the face `travel` px instead of the theme's (builder style).
    #[must_use]
    pub fn travel(mut self, travel: f32) -> Self {
        self.travel = Some(travel);
        self
    }
}

/// 1px inset highlight/shadow band under a face's top edge.
const BAND_H: f32 = 1.0;
/// Offset and blur of the shadow a pressed face drops into (the design's
/// `--key-inset-pressed` `inset 0 2px 3px`).
const PRESS_SHADOW: ([f32; 2], f32) = ([0.0, 2.0], 3.0);
/// Face border alpha (the design's `1px solid rgba(0,0,0,0.5)`).
const FACE_EDGE_ALPHA: f32 = 0.5;
/// Ghost-tone border alphas: fainter idle, near-face-edge when active.
const GHOST_EDGE_IDLE: f32 = 0.25;
const GHOST_EDGE_ACTIVE: f32 = 0.4;
/// Ghost-tone face: a flat white wash while hovered or pressed (the design's
/// `rgba(255,255,255,0.08)` / `0.04`), transparent at rest.
const GHOST_FILL_HOVER: f32 = 0.08;
const GHOST_FILL_PRESSED: f32 = 0.04;
/// Ghost-tone insets: a 1px white line under the top edge while hovered
/// (`inset 0 1px 0 rgba(255,255,255,0.1)`), and a soft dark one while
/// pressed (`inset 0 1px 2px rgba(0,0,0,0.3)`).
const GHOST_HIGHLIGHT: f32 = 0.1;
const GHOST_PRESS_SHADOW: ([f32; 2], f32, f32) = ([0.0, 1.0], 2.0, 0.3);
/// Sunken border alpha.
const SUNKEN_EDGE_ALPHA: f32 = 0.6;
/// Disabled controls fade to this fraction of their material (the design's
/// `opacity: 0.45`). [`Pressable`](super::Pressable) fades its content by
/// the same amount.
pub(crate) const DISABLED_ALPHA: f32 = 0.45;

/// Draw the complete face-over-plinth material for `rect`.
///
/// Returns the rect the **face** occupies (the label should center in the face,
/// and while pressed it sits `travel` px lower).
pub fn draw(list: &mut DrawList, s: &StyleResolver, rect: Rect, m: &Material) -> Rect {
    draw_with_radius(list, s, rect, s.scalar(StyleKey::BorderRadius), m)
}

/// [`draw`] with an explicit corner radius (the design's stepper keys pass
/// `travel: 1, radius: 0`-style overrides).
///
/// Returns the face rect.
pub fn draw_with_radius(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    radius: f32,
    m: &Material,
) -> Rect {
    let travel = m.travel.unwrap_or_else(|| s.scalar(StyleKey::Travel));
    // The face is always `travel` px shorter than the plinth: at rest it sits
    // at the top (plinth visible beneath), pressed it drops to the bottom
    // (plinth visible above). That's the design's plinth model — the gap
    // trades sides, the face never changes size.
    let edge = if m.enabled && m.pressed { travel } else { 0.0 };
    let face = Rect::new(
        rect.x,
        rect.y + edge,
        rect.width,
        (rect.height - travel).max(0.0),
    );

    match m.tone {
        Tone::Sunken => draw_sunken(list, s, rect, radius, m),
        Tone::Ghost => {
            draw_plinth(list, s, rect, radius, m);
            draw_ghost(list, face, radius, m);
        }
        tone => {
            // The face covers the plinth except in the `travel` gap.
            if travel > 0.0 {
                draw_plinth(list, s, rect, radius, m);
            }
            draw_raised(list, s, face, radius, m, tone);
        }
    }
    face
}

/// The dark slab a face rests on, over the whole rect; nothing when
/// [`Material::hollow`].
fn draw_plinth(list: &mut DrawList, s: &StyleResolver, rect: Rect, radius: f32, m: &Material) {
    if m.hollow {
        return;
    }
    let mut plinth = s.color(StyleKey::Plinth);
    if !m.enabled {
        plinth[3] *= DISABLED_ALPHA;
    }
    list.chrome_rect(rect, radius, 0.0, plinth, [0.0; 4]);
}

/// Draw the **inset shadow** shared by every sunken surface: a band fading
/// down from the top edge (dark `InnerShadow` color to transparent), plus the
/// 1px light `EdgeShadow` line under the bottom edge (the "lit from above"
/// counter-edge).
///
/// `depth` is the band height; the outline stays inside `rect` by `inset` px
/// on every side (the surface's own border width — pass 0.0 for borderless
/// callers). Skips geometry cleanly when the band would be degenerate.
pub(crate) fn draw_inset_shadow(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    depth: f32,
    inset: f32,
) {
    let shadow = s.color(StyleKey::InnerShadow);
    let band_h = depth.max(1.0).min((rect.height - 2.0 * inset).max(0.0));
    let radius = s.scalar(StyleKey::BorderRadius);
    if band_h > 0.0 {
        list.chrome_rect_gradient(
            Rect::new(
                rect.x + inset,
                rect.y + inset,
                (rect.width - 2.0 * inset).max(0.0),
                band_h,
            ),
            radius,
            0.0,
            shadow,
            [shadow[0], shadow[1], shadow[2], 0.0],
            [0.0; 4],
        );
    }
    // The 1px light line under the bottom edge.
    let under = s.color(StyleKey::EdgeShadow);
    let line_h = BAND_H;
    let y = rect.y + rect.height - inset - line_h;
    if y > rect.y + inset {
        list.quad(
            rect.x + inset,
            y,
            (rect.width - 2.0 * inset).max(0.0),
            line_h,
            under,
        );
    }
}

/// Raised tones: a sheen gradient face (over the plinth the caller drew).
///
/// The neutral face resolves its **base** from the state keys (`Button` /
/// `ButtonHover` / `ButtonPressed`) and composites the white-sheen tokens
/// (`FaceTop*` / `FaceBottom*`) over it — so an overlay retuning
/// [`StyleKey::Button`](StyleKey::Button) still recolors buttons, while the
/// sheen stays themeable separately. Accent/danger faces use their own opaque
/// gradients.
fn draw_raised(
    list: &mut DrawList,
    s: &StyleResolver,
    face: Rect,
    radius: f32,
    m: &Material,
    tone: Tone,
) {
    let dim = |mut c: [f32; 4]| {
        if !m.enabled {
            c[3] *= DISABLED_ALPHA;
        }
        c
    };

    let pressed = m.enabled && m.pressed;
    let hovered = m.enabled && m.hovered;

    let (top, bottom, highlight) = match tone {
        Tone::Accent if pressed => (
            s.color(StyleKey::AccentFaceTopPressed),
            s.color(StyleKey::AccentFaceBottomPressed),
            s.color(StyleKey::EdgeHighlightPressed),
        ),
        Tone::Accent if hovered => (
            s.color(StyleKey::AccentFaceTopHover),
            s.color(StyleKey::AccentFaceBottomHover),
            s.color(StyleKey::EdgeHighlightHover),
        ),
        Tone::Accent => (
            s.color(StyleKey::AccentFaceTop),
            s.color(StyleKey::AccentFaceBottom),
            s.color(StyleKey::EdgeHighlight),
        ),
        Tone::Danger if pressed => (
            s.color(StyleKey::DangerFaceTopPressed),
            s.color(StyleKey::DangerFaceBottomPressed),
            s.color(StyleKey::EdgeHighlightPressed),
        ),
        Tone::Danger if hovered => (
            s.color(StyleKey::DangerFaceTopHover),
            s.color(StyleKey::DangerFaceBottomHover),
            s.color(StyleKey::EdgeHighlightHover),
        ),
        Tone::Danger => (
            s.color(StyleKey::DangerFaceTop),
            s.color(StyleKey::DangerFaceBottom),
            s.color(StyleKey::EdgeHighlight),
        ),
        // Neutral: state base under the sheen gradient.
        _ => {
            let base = if pressed {
                s.color(StyleKey::ButtonPressed)
            } else if hovered {
                s.color(StyleKey::ButtonHover)
            } else {
                s.color(StyleKey::Button)
            };
            let sheen_top = if pressed {
                s.color(StyleKey::FaceTopPressed)
            } else if hovered {
                s.color(StyleKey::FaceTopHover)
            } else {
                s.color(StyleKey::FaceTop)
            };
            let sheen_bottom = if pressed {
                s.color(StyleKey::FaceBottomPressed)
            } else if hovered {
                s.color(StyleKey::FaceBottomHover)
            } else {
                s.color(StyleKey::FaceBottom)
            };
            let hl = if pressed {
                s.color(StyleKey::EdgeHighlightPressed)
            } else if hovered {
                s.color(StyleKey::EdgeHighlightHover)
            } else {
                s.color(StyleKey::EdgeHighlight)
            };
            (
                sheen_over(base, sheen_top),
                sheen_over(base, sheen_bottom),
                hl,
            )
        }
    };

    let border = [0.0, 0.0, 0.0, FACE_EDGE_ALPHA];
    list.chrome_rect_gradient(face, radius, 1.0, dim(top), dim(bottom), dim(border));

    // Top edge decoration: at rest a 1px white highlight; pressed, the short
    // shadow of the face sitting in the plinth (plus its own faint highlight).
    if pressed {
        let (offset, blur) = PRESS_SHADOW;
        face_inset(
            list,
            face,
            radius,
            offset,
            blur,
            dim(s.color(StyleKey::InnerShadow)),
        );
    }
    face_inset(list, face, radius, [0.0, BAND_H], 0.0, dim(highlight));
}

/// Paint one of a face's inset box shadows (`inset x y blur color`) inside
/// its 1 px border, so highlights and press shadows follow the corner
/// rounding however round the key is.
fn face_inset(
    list: &mut DrawList,
    face: Rect,
    radius: f32,
    offset: [f32; 2],
    blur: f32,
    color: [f32; 4],
) {
    let radius = radius.min(face.width.min(face.height) * 0.5);
    list.box_shadow_inset(
        face.inset(1.0),
        CornerRadii::uniform((radius - 1.0).max(0.0)),
        BoxShadow {
            offset,
            blur,
            color,
            inset: true,
            ..BoxShadow::default()
        },
    );
}

/// Composite a translucent `sheen` color over an opaque-ish `base`
/// (source-over, per channel). The neutral face is a white sheen over the
/// state base, so this is the per-draw blend of the two token layers.
pub(crate) fn sheen_over(base: [f32; 4], sheen: [f32; 4]) -> [f32; 4] {
    let a = sheen[3];
    let out_a = a + base[3] * (1.0 - a);
    if out_a <= 0.0 {
        return [0.0; 4];
    }
    [
        (sheen[0] * a + base[0] * base[3] * (1.0 - a)) / out_a,
        (sheen[1] * a + base[1] * base[3] * (1.0 - a)) / out_a,
        (sheen[2] * a + base[2] * base[3] * (1.0 - a)) / out_a,
        out_a,
    ]
}

/// Derive a stable [`FocusId`](crate::FocusId) from a name + rect, for
/// stateless widgets that still want click-to-focus (the id is stable across
/// frames as long as the field doesn't move; movement merely re-keys it).
pub(crate) fn focus_id_for(name: &str, rect: &Rect) -> crate::FocusId {
    let mut h = crate::style::fnv1a64(name);
    for v in [rect.x, rect.y, rect.width, rect.height] {
        for b in v.to_bits().to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
    h
}

/// Ghost tone's face (over the plinth the caller drew): transparent at rest
/// on a faint border; hover and press lay a flat white wash on it, with the
/// design's own insets.
fn draw_ghost(list: &mut DrawList, face: Rect, radius: f32, m: &Material) {
    let dim = |mut c: [f32; 4]| {
        if !m.enabled {
            c[3] *= DISABLED_ALPHA;
        }
        c
    };
    let pressed = m.enabled && m.pressed;
    let hovered = m.enabled && m.hovered && !pressed;
    let fill = if pressed {
        GHOST_FILL_PRESSED
    } else if hovered {
        GHOST_FILL_HOVER
    } else {
        0.0
    };
    let edge_alpha = if pressed || hovered {
        GHOST_EDGE_ACTIVE
    } else {
        GHOST_EDGE_IDLE
    };
    let fill = [1.0, 1.0, 1.0, fill];
    list.chrome_rect(
        face,
        radius,
        1.0,
        dim(fill),
        dim([0.0, 0.0, 0.0, edge_alpha]),
    );
    if pressed {
        let (offset, blur, alpha) = GHOST_PRESS_SHADOW;
        face_inset(
            list,
            face,
            radius,
            offset,
            blur,
            dim([0.0, 0.0, 0.0, alpha]),
        );
    } else if hovered {
        let highlight = [1.0, 1.0, 1.0, GHOST_HIGHLIGHT];
        face_inset(list, face, radius, [0.0, BAND_H], 0.0, dim(highlight));
    }
}

/// Sunken tone: dark trough with an inset shadow and a light line beneath.
fn draw_sunken(list: &mut DrawList, s: &StyleResolver, rect: Rect, radius: f32, m: &Material) {
    let dim = |mut c: [f32; 4]| {
        if !m.enabled {
            c[3] *= DISABLED_ALPHA;
        }
        c
    };
    let fill = dim(s.color(StyleKey::InputBackground));
    let border = [0.0, 0.0, 0.0, SUNKEN_EDGE_ALPHA];
    list.chrome_rect(rect, radius, 1.0, fill, dim(border));
    draw_inset_shadow(list, s, rect, s.scalar(StyleKey::InnerShadowDepth), 1.0);
}

/// Draw an input **well**: the sunken field every text entry sits in, plus the
/// focus treatment (accent border + soft outer ring). `focused`/`invalid`
/// switch the border to the accent/danger color and add the outer ring.
pub fn draw_well(list: &mut DrawList, s: &StyleResolver, rect: Rect, focused: bool, invalid: bool) {
    draw_well_simple(list, s, rect, focused, invalid);

    // Focus/invalid ring: a 1px soft outline just inside the border (kept
    // within the widget's declared rect so debug lints don't flag it).
    if focused || invalid {
        let radius = s.scalar(StyleKey::BorderRadius);
        let ring = if invalid {
            s.color(StyleKey::Error)
        } else {
            s.color(StyleKey::Accent)
        };
        let alpha = WELL_RING_ALPHA[usize::from(invalid)];
        let ring_out = [ring[0], ring[1], ring[2], alpha];
        list.rounded_rect_outline(rect, radius + 0.5, 1.0, ring_out);
    }
}

/// [`draw_well`] alias — the sunken field with the accent-when-focused border,
/// no outer ring (embedders that draw their own focus treatment: caret widgets,
/// cell-grids).
pub fn draw_well_simple(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    focused: bool,
    invalid: bool,
) {
    let radius = s.scalar(StyleKey::BorderRadius);
    let border_w = s.scalar(StyleKey::BorderWidth);
    let fill = if focused {
        let mut c = s.color(StyleKey::InputBackground);
        c[3] = (c[3] + 0.08).min(1.0);
        c
    } else {
        s.color(StyleKey::InputBackground)
    };
    let border = if invalid {
        s.color(StyleKey::Error)
    } else if focused {
        s.color(StyleKey::Accent)
    } else {
        s.color(StyleKey::InputBorder)
    };
    list.chrome_rect(rect, radius, border_w, fill, border);
    draw_inset_shadow(
        list,
        s,
        rect,
        s.scalar(StyleKey::InnerShadowDepth),
        border_w,
    );
}

/// The deep well's recess (`--well-inset-tall`'s `inset 0 2px 5px`) and lit
/// lower lip (`0 1px 0 rgba(255,255,255,.07)`).
const DEEP_WELL_RECESS: ([f32; 2], f32, [f32; 4]) = ([0.0, 2.0], 5.0, [0.0, 0.0, 0.0, 0.6]);
/// The lit lip under a deep well's bottom edge (`0 1px 0` white at 7%).
const DEEP_WELL_LIP: BoxShadow = BoxShadow {
    offset: [0.0, 1.0],
    blur: 0.0,
    spread: 0.0,
    color: [1.0, 1.0, 1.0, 0.07],
    inset: false,
};

/// Draw a **deep well**: the tall sunken box that holds a block of content
/// rather than one value (an alert's detail text, a drag list's rows).
/// `--well-deep` fill, a hard edge, a tall recess inside and a lit lip under
/// the bottom edge. Returns the area inside the border, where the content
/// goes. The lip paints just outside `rect`; [`deep_well_ink`] is the whole
/// painted area, for a debug scope to declare.
pub fn draw_deep_well(list: &mut DrawList, s: &StyleResolver, rect: Rect) -> Rect {
    let radius = s.scalar(StyleKey::BorderRadius);
    let border = s.scalar(StyleKey::BorderWidth);
    list.box_shadow_outset(rect, CornerRadii::uniform(radius), DEEP_WELL_LIP);
    list.chrome_rect(
        rect,
        radius,
        border,
        s.color(StyleKey::WellDeep),
        s.color(StyleKey::EdgeHard),
    );
    let inner = rect.inset(border);
    let (offset, blur, color) = DEEP_WELL_RECESS;
    list.box_shadow_inset(
        inner,
        CornerRadii::uniform((radius - border).max(0.0)),
        BoxShadow {
            offset,
            blur,
            color,
            inset: true,
            ..BoxShadow::default()
        },
    );
    inner
}

/// Everything [`draw_deep_well`] paints for a well at `rect`: the box and
/// the lip under it.
pub fn deep_well_ink(rect: Rect) -> Rect {
    rect.union(DEEP_WELL_LIP.ink_rect(rect))
}

/// Offset and blur of the well's inner shadow (`--well-inset`'s
/// `inset 0 2px 4px`).
#[cfg(feature = "phosphor-icons")]
const WELL_INSET_SHADOW: ([f32; 2], f32) = ([0.0, 2.0], 4.0);
/// Spread of the focus / invalid ring around a well (`--well-focus-ring`'s
/// `0 0 0 2px`).
#[cfg(feature = "phosphor-icons")]
const WELL_RING_SPREAD: f32 = 2.0;
/// The ring's alpha: Forge `--accent-ring` is .16, `--danger-ring` .18.
const WELL_RING_ALPHA: [f32; 2] = [0.16, 0.18];

/// Draw an input well with corner `radius`, painted with the design's box
/// shadows so every layer follows the rounding: the round search well
/// (`--radius-search`) as much as a square one. Resting, the well has a
/// light line under its bottom edge (`--well-inset`); focused or invalid,
/// the border turns accent or danger and a soft 2 px ring replaces that line
/// (`--well-focus-ring`, `--well-invalid-ring`). `radius` is clamped to half
/// the shorter side.
///
/// Only the search field paints one so far, hence the feature gate; see the
/// TODO on moving [`draw_well`] onto these shadows.
#[cfg(feature = "phosphor-icons")]
pub fn draw_well_rounded(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    radius: f32,
    focused: bool,
    invalid: bool,
) {
    let radius = radius.min(rect.width.min(rect.height) * 0.5).max(0.0);
    let border_w = s.scalar(StyleKey::BorderWidth);
    let ring = if invalid {
        Some((s.color(StyleKey::Error), WELL_RING_ALPHA[1]))
    } else if focused {
        Some((s.color(StyleKey::Accent), WELL_RING_ALPHA[0]))
    } else {
        None
    };
    let outset = match ring {
        Some((c, alpha)) => BoxShadow {
            spread: WELL_RING_SPREAD,
            color: [c[0], c[1], c[2], alpha],
            ..BoxShadow::default()
        },
        None => BoxShadow {
            offset: [0.0, BAND_H],
            color: s.color(StyleKey::EdgeShadow),
            ..BoxShadow::default()
        },
    };
    list.box_shadow_outset(rect, CornerRadii::uniform(radius), outset);

    let mut fill = s.color(StyleKey::InputBackground);
    if focused {
        // `--well-focus` is .5 over `--well`'s .42.
        fill[3] = (fill[3] + 0.08).min(1.0);
    }
    let border = match ring {
        Some((c, _)) => c,
        None => s.color(StyleKey::InputBorder),
    };
    list.chrome_rect(rect, radius, border_w, fill, border);

    let (offset, blur) = WELL_INSET_SHADOW;
    list.box_shadow_inset(
        rect.inset(border_w),
        CornerRadii::uniform((radius - border_w).max(0.0)),
        BoxShadow {
            offset,
            blur,
            color: s.color(StyleKey::InnerShadow),
            inset: true,
            ..BoxShadow::default()
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Rect;
    use crate::{StyleResolver, Theme};
    use std::sync::OnceLock;

    fn styles() -> &'static StyleResolver<'static> {
        // Leak a theme so the resolver's lifetime is 'static in tests; the
        // process exits before it matters.
        static S: OnceLock<StyleResolver<'static>> = OnceLock::new();
        S.get_or_init(|| StyleResolver::new(Box::leak(Box::new(Theme::default()))))
    }

    #[test]
    fn face_drops_by_travel_when_pressed() {
        let s = styles();
        let rect = Rect::new(10.0, 10.0, 60.0, 24.0);
        let mut idle = DrawList::new();
        let f = draw(&mut idle, s, rect, &Material::new(Tone::Default));
        assert_eq!(
            (f.y - rect.y, f.height),
            (0.0, 22.0),
            "at rest the face sits at the top, `travel` short of the control"
        );

        let mut pressed = DrawList::new();
        let f = draw(
            &mut pressed,
            s,
            rect,
            &Material::new(Tone::Default).pressed(true),
        );
        assert_eq!(
            (f.y - rect.y, f.height),
            (2.0, 22.0),
            "pressing drops the face onto the plinth by `travel`"
        );
    }

    #[test]
    fn highlights_and_press_shadows_follow_the_face_rounding() {
        let s = styles();
        // A round 16 px key: the face is 16×14, so its radius clamps to 7.
        let rect = Rect::new(0.0, 0.0, 16.0, 16.0);
        // A pressed raised key keeps its highlight under the press shadow; a
        // ghost key has one or the other.
        for (tone, pressed, shadows) in [
            (Tone::Default, true, 2),
            (Tone::Ghost, true, 1),
            (Tone::Ghost, false, 1),
        ] {
            let mut list = DrawList::new();
            let m = Material::new(tone).hovered(true).pressed(pressed);
            let face = draw_with_radius(&mut list, s, rect, 8.0, &m);
            assert_eq!(list.shadow_instance_count(), shadows, "{tone:?}");
            for shadow in list.shadow_instances() {
                assert_eq!(
                    shadow.element_rect,
                    [
                        face.x + 1.0,
                        face.y + 1.0,
                        face.width - 2.0,
                        face.height - 2.0
                    ],
                    "{tone:?}: painted inside the face's border"
                );
                assert_eq!(
                    shadow.element_radii, [6.0; 4],
                    "{tone:?}: rounded like the face"
                );
            }
        }
    }

    #[test]
    fn raised_paints_plinth_then_sheen_face() {
        let s = styles();
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        let mut list = DrawList::new();
        draw(&mut list, s, rect, &Material::new(Tone::Default));
        // [0] plinth (flat dark), [1] face gradient = sheen over the state base.
        assert_eq!(
            list.chrome_instance(0).unwrap().bg,
            s.color(StyleKey::Plinth)
        );
        let expected_top = sheen_over(s.color(StyleKey::Button), s.color(StyleKey::FaceTop));
        assert_eq!(list.chrome_instance(1).unwrap().bg, expected_top);
        let expected_bot = sheen_over(s.color(StyleKey::Button), s.color(StyleKey::FaceBottom));
        assert_eq!(list.chrome_instance(1).unwrap().bg2, expected_bot);
        // Hovered composites the sheen over the hover base instead.
        let mut hov = DrawList::new();
        draw(
            &mut hov,
            s,
            rect,
            &Material::new(Tone::Default).hovered(true),
        );
        let expected_hover = sheen_over(
            s.color(StyleKey::ButtonHover),
            s.color(StyleKey::FaceTopHover),
        );
        assert_eq!(hov.chrome_instance(1).unwrap().bg, expected_hover);
    }

    #[test]
    fn overlay_recoloring_button_base_reaches_the_face() {
        // The seam: an overlay retuning `StyleKey::Button` shifts the neutral
        // face even though the sheen tokens stay in place.
        let s = styles();
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        let mut overlay = crate::StyleOverlay::new();
        overlay.set_color(StyleKey::Button, [0.7, 0.1, 0.2, 1.0]);
        let styled = StyleResolver::with_overlay(s.theme(), &overlay);
        let mut list = DrawList::new();
        draw(&mut list, &styled, rect, &Material::new(Tone::Default));
        let expected = sheen_over([0.7, 0.1, 0.2, 1.0], s.color(StyleKey::FaceTop));
        assert_eq!(list.chrome_instance(1).unwrap().bg, expected);
    }

    #[test]
    fn ghost_is_a_transparent_face_on_the_plinth() {
        let s = styles();
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        let mut list = DrawList::new();
        draw(&mut list, s, rect, &Material::new(Tone::Ghost));
        assert_eq!(list.chrome_instance_count(), 2, "plinth, then face");
        let plinth = list.chrome_instance(0).unwrap();
        assert_eq!(plinth.bg, s.color(StyleKey::Plinth));
        assert_eq!(plinth.rect, [0.0, 0.0, 40.0, 20.0], "the whole rect");
        let face = list.chrome_instance(1).unwrap();
        assert_eq!(face.bg[3], 0.0, "idle ghost face is fully transparent");
        assert_eq!(face.border, [0.0, 0.0, 0.0, GHOST_EDGE_IDLE]);
        assert_eq!(list.shadow_instance_count(), 0, "no inset at rest");
    }

    #[test]
    fn ghost_hover_and_press_wash_the_face_flat_white() {
        let s = styles();
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        for (m, alpha) in [
            (Material::new(Tone::Ghost).hovered(true), GHOST_FILL_HOVER),
            (
                Material::new(Tone::Ghost).hovered(true).pressed(true),
                GHOST_FILL_PRESSED,
            ),
        ] {
            let mut list = DrawList::new();
            draw(&mut list, s, rect, &m);
            let face = list.chrome_instance(1).unwrap();
            assert_eq!(face.bg, [1.0, 1.0, 1.0, alpha]);
            assert_eq!(face.bg, face.bg2, "flat, not a gradient");
            assert_eq!(face.border, [0.0, 0.0, 0.0, GHOST_EDGE_ACTIVE]);
        }
    }

    #[test]
    fn a_hollow_key_has_no_plinth() {
        let s = styles();
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        for tone in [Tone::Ghost, Tone::Default] {
            let mut list = DrawList::new();
            draw(&mut list, s, rect, &Material::new(tone).hollow(true));
            assert_eq!(list.chrome_instance_count(), 1, "{tone:?}: the face only");
            assert_ne!(
                list.chrome_instance(0).unwrap().bg,
                s.color(StyleKey::Plinth),
                "{tone:?}"
            );
        }
    }

    #[test]
    fn a_disabled_ghost_fades_its_plinth_too() {
        let s = styles();
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        let mut list = DrawList::new();
        draw(
            &mut list,
            s,
            rect,
            &Material::new(Tone::Ghost).enabled(false),
        );
        let plinth = list.chrome_instance(0).unwrap().bg;
        assert_eq!(plinth[3], s.color(StyleKey::Plinth)[3] * DISABLED_ALPHA);
    }

    #[test]
    fn accent_tone_resolves_accent_faces_and_disabled_dims() {
        let s = styles();
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        let mut list = DrawList::new();
        draw(&mut list, s, rect, &Material::new(Tone::Accent));
        assert_eq!(
            list.chrome_instance(1).unwrap().bg,
            s.color(StyleKey::AccentFaceTop)
        );

        let mut off = DrawList::new();
        draw(
            &mut off,
            s,
            rect,
            &Material::new(Tone::Accent).enabled(false),
        );
        let face = off.chrome_instance(1).unwrap().bg;
        let full = s.color(StyleKey::AccentFaceTop);
        assert!(
            (face[3] - full[3] * DISABLED_ALPHA).abs() < 1e-4,
            "disabled alpha fades the face"
        );
    }

    #[test]
    fn sunken_paints_fill_shadow_and_underline() {
        let s = styles();
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        let mut list = DrawList::new();
        draw(&mut list, s, rect, &Material::new(Tone::Sunken));
        assert_eq!(
            list.chrome_instance(0).unwrap().bg,
            s.color(StyleKey::InputBackground)
        );
    }

    #[test]
    fn well_focus_ring_appears_only_when_focused() {
        let s = styles();
        let rect = Rect::new(0.0, 0.0, 60.0, 22.0);
        let mut idle = DrawList::new();
        draw_well(&mut idle, s, rect, false, false);
        let mut focus = DrawList::new();
        draw_well(&mut focus, s, rect, true, false);
        assert!(
            focus.chrome_instance_count() > idle.chrome_instance_count(),
            "the focus ring is an extra stroke instance"
        );
    }
}
