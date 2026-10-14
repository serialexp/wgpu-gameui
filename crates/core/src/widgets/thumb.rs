//! Thumb — the tiny leading tile of a row (Forge `Thumb`): a project's
//! monogram, an image, a colour swatch, a glyph, or an empty slot, all with
//! the same hard 1 px edge and lit top so mixed rows line up.
//!
//! What it shows depends on what it's given, in this order:
//!
//! 1. an [`image`](Thumb::image) (a sprite on a raised plate, or edge to edge
//!    with [`ImageFit::Cover`]);
//! 2. a [`fill`](Thumb::fill) (sample paint, such as a gradient);
//! 3. a [`color`](Thumb::color) swatch;
//! 4. a [`name`](Thumb::name), shown as a monogram of its [`initials`] on a
//!    plate tinted by [`hue_of`] the name, so a project keeps its colour
//!    everywhere it appears;
//! 5. a [`glyph`](Thumb::glyph);
//! 6. otherwise, or with [`slot`](Thumb::slot), a dashed, hatched empty slot.
//!
//! The tile is drawn at its top-left corner and returns its rect.
//!
//! Gated behind the `phosphor-icons` feature (the glyph is a Phosphor icon).
//!
//! # Example
//! ```ignore
//! Thumb::new().name("agent-ui").size(14.0).draw(x, y, list, &style);
//! ```

use crate::SpriteId;
use crate::chrome::{Background, GradientAxis};
use crate::color::oklch;
use crate::layout::Rect;
use crate::render::PhosphorIcon;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{Ink, StyleKey, StyleResolver};
use crate::text::TextBlock;

use super::{DrawList, Icon, Image, ImageFit};

/// The edge of an unselected tile (`--edge-hard`).
const EDGE: [f32; 4] = [0.0, 0.0, 0.0, 0.65];
/// The edge of a tile on a selected (accent) row.
const EDGE_SELECTED: [f32; 4] = [4.0 / 255.0, 20.0 / 255.0, 24.0 / 255.0, 0.55];
/// The lit top line of a swatch or monogram (`inset 0 1px 0`).
const LIT_TOP: [f32; 4] = [1.0, 1.0, 1.0, 0.14];
/// The drop under a swatch or monogram (`0 1px 1px`).
const LIT_DROP: [f32; 4] = [0.0, 0.0, 0.0, 0.45];
/// The drop under an image or fill tile.
const PLATE_DROP: [f32; 4] = [0.0, 0.0, 0.0, 0.5];
/// The raised plate an image sits on: a white sheen over `#1d2328`.
const PLATE_BASE: [f32; 4] = [29.0 / 255.0, 35.0 / 255.0, 40.0 / 255.0, 1.0];
/// The bevel drawn over an image tile: top, left, right and bottom lines.
const SHEEN: [[f32; 4]; 4] = [
    [1.0, 1.0, 1.0, 0.3],
    [1.0, 1.0, 1.0, 0.1],
    [0.0, 0.0, 0.0, 0.25],
    [0.0, 0.0, 0.0, 0.45],
];
/// The monogram's text shadow (`0 1px 0 rgba(0,0,0,.35)`).
const MONOGRAM_SHADOW: u8 = 89;
/// The dashed edge of an empty slot, plain and on a selected row.
const SLOT_EDGE: [f32; 4] = [1.0, 1.0, 1.0, 0.16];
const SLOT_EDGE_SELECTED: [f32; 4] = [4.0 / 255.0, 20.0 / 255.0, 24.0 / 255.0, 0.45];
/// The slot's hatch lines and their spacing.
const HATCH: [f32; 4] = [1.0, 1.0, 1.0, 0.05];
const HATCH_STEP: f32 = 3.0;
/// Dash and gap of the slot's dashed edge.
const DASH: f32 = 3.0;

/// The hue (0–359) a name's monogram plate is tinted with: the Forge
/// `hueOf` hash (`h = h * 31 + unit`, wrapping at 32 bits, over the UTF-16
/// code units), so a name gets the same colour here as in the design.
pub fn hue_of(name: &str) -> f32 {
    let h = name.encode_utf16().fold(0u32, |h, unit| {
        h.wrapping_mul(31).wrapping_add(u32::from(unit))
    });
    (h % 360) as f32
}

/// A name's monogram: the first letters of its first two words, or the first
/// two characters of a single word, upper-cased. Anything that isn't an ASCII
/// letter or digit separates words. `"?"` for a name with no such characters.
pub fn initials(name: &str) -> String {
    let mut words = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty());
    let out: String = match (words.next(), words.next()) {
        (Some(a), Some(b)) => a.chars().take(1).chain(b.chars().take(1)).collect(),
        (Some(a), None) => a.chars().take(2).collect(),
        (None, _) => return "?".to_owned(),
    };
    out.to_ascii_uppercase()
}

/// A row's leading tile. See the [module docs](self).
#[derive(Clone, Copy, Debug)]
pub struct Thumb<'a> {
    image: Option<SpriteId>,
    fit: ImageFit,
    fill: Option<Background>,
    color: Option<[f32; 4]>,
    name: Option<&'a str>,
    glyph: Option<PhosphorIcon>,
    slot: bool,
    size: f32,
    round: bool,
    selected: bool,
}

impl Default for Thumb<'_> {
    fn default() -> Self {
        Self {
            image: None,
            fit: ImageFit::Cover,
            fill: None,
            color: None,
            name: None,
            glyph: None,
            slot: false,
            size: 14.0,
            round: false,
            selected: false,
        }
    }
}

/// What a [`Thumb`] ends up drawing, after the resolution order.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Face<'a> {
    Plate {
        image: Option<SpriteId>,
        fill: Option<Background>,
    },
    Swatch([f32; 4]),
    Monogram(&'a str),
    Glyph(PhosphorIcon),
    Slot,
}

impl<'a> Thumb<'a> {
    /// A 14 px tile with nothing to show yet (an empty slot).
    pub fn new() -> Self {
        Self::default()
    }

    /// Show `sprite` (takes precedence over everything else).
    #[must_use]
    pub fn image(mut self, sprite: SpriteId) -> Self {
        self.image = Some(sprite);
        self
    }

    /// How the image fits: [`Cover`](ImageFit::Cover) (the default) fills the
    /// tile inside a 1 px lip; [`Contain`](ImageFit::Contain) insets it on a
    /// raised plate, for transparent icons.
    #[must_use]
    pub fn fit(mut self, fit: ImageFit) -> Self {
        self.fit = fit;
        self
    }

    /// Paint the tile with `fill` (a sample of a material or gradient).
    #[must_use]
    pub fn fill(mut self, fill: Background) -> Self {
        self.fill = Some(fill);
        self
    }

    /// Show a flat colour swatch.
    #[must_use]
    pub fn color(mut self, color: [f32; 4]) -> Self {
        self.color = Some(color);
        self
    }

    /// Show `name`'s monogram on a plate tinted by its [`hue_of`].
    #[must_use]
    pub fn name(mut self, name: &'a str) -> Self {
        self.name = Some(name);
        self
    }

    /// Show a glyph (used when there's no image, colour or name).
    #[must_use]
    pub fn glyph(mut self, glyph: PhosphorIcon) -> Self {
        self.glyph = Some(glyph);
        self
    }

    /// Force the empty slot even when a name or glyph is set (an image, fill
    /// or colour still wins).
    #[must_use]
    pub fn slot(mut self, slot: bool) -> Self {
        self.slot = slot;
        self
    }

    /// The tile's side in pixels (14 by default).
    #[must_use]
    pub fn size(mut self, size: f32) -> Self {
        self.size = size.max(1.0);
        self
    }

    /// Draw a circle instead of a rounded square.
    #[must_use]
    pub fn round(mut self, round: bool) -> Self {
        self.round = round;
        self
    }

    /// The tile sits on a selected (accent) row: darker edges and an
    /// on-accent glyph.
    #[must_use]
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// The tile's side in pixels, for laying out what sits beside it.
    pub(crate) fn side(&self) -> f32 {
        self.size
    }

    /// The tile's corner radius.
    fn radius(&self) -> f32 {
        if self.round {
            self.size * 0.5
        } else if self.size >= 24.0 {
            3.0
        } else {
            2.0
        }
    }

    fn face(&self) -> Face<'a> {
        if self.image.is_some() || self.fill.is_some() {
            return Face::Plate {
                image: self.image,
                fill: self.fill,
            };
        }
        if let Some(color) = self.color {
            return Face::Swatch(color);
        }
        match (self.name, self.glyph) {
            (Some(name), _) if !self.slot => Face::Monogram(name),
            (_, Some(glyph)) if !self.slot => Face::Glyph(glyph),
            _ => Face::Slot,
        }
    }

    /// Draw the tile with its top-left corner at `(x, y)`; returns its rect.
    pub fn draw(&self, x: f32, y: f32, list: &mut DrawList, s: &StyleResolver) -> Rect {
        let rect = Rect::new(x, y, self.size, self.size);
        let edge = if self.selected { EDGE_SELECTED } else { EDGE };
        match self.face() {
            Face::Plate { image, fill } => self.draw_plate(list, rect, edge, image, fill),
            Face::Swatch(color) => {
                self.lit_box(list, rect, edge, Background::Solid(color));
            }
            Face::Monogram(name) => self.draw_monogram(list, s, rect, edge, name),
            Face::Glyph(glyph) => {
                let side = (self.size * 0.72).round();
                let tint = if self.selected {
                    s.ink(Ink::OnAccentSecond)
                } else {
                    s.ink(Ink::Muted)
                };
                Icon::new(glyph).tint(tint).draw(
                    Rect::new(
                        x + (self.size - side) * 0.5,
                        y + (self.size - side) * 0.5,
                        side,
                        side,
                    ),
                    list,
                );
            }
            Face::Slot => self.draw_slot(list, s, rect),
        }
        rect
    }

    /// A tile with an edge, a background, the lit top line and the drop.
    fn lit_box(&self, list: &mut DrawList, rect: Rect, edge: [f32; 4], bg: Background) {
        let radius = self.radius();
        list.box_shadow_outset(
            rect,
            CornerRadii::uniform(radius),
            BoxShadow {
                offset: [0.0, 1.0],
                blur: 1.0,
                color: LIT_DROP,
                ..BoxShadow::default()
            },
        );
        self.edged(list, rect, edge, bg);
        let inner = rect.inset(1.0);
        list.quad(inner.x, inner.y, inner.width, 1.0, LIT_TOP);
    }

    /// The edge as a 1 px ring with `bg` inside it.
    fn edged(&self, list: &mut DrawList, rect: Rect, edge: [f32; 4], bg: Background) {
        let radius = self.radius();
        list.rounded_rect(rect, radius, edge);
        list.paint_quad_background(
            rect.inset(1.0),
            bg,
            CornerRadii::uniform((radius - 1.0).max(0.0)),
        );
    }

    fn draw_monogram(
        &self,
        list: &mut DrawList,
        s: &StyleResolver,
        rect: Rect,
        edge: [f32; 4],
        name: &str,
    ) {
        let h = hue_of(name);
        self.lit_box(
            list,
            rect,
            edge,
            Background::LinearGradient {
                start: oklch(0.46, 0.06, h, 1.0),
                end: oklch(0.36, 0.05, h, 1.0),
                axis: GradientAxis::Vertical,
            },
        );
        let mut text = initials(name);
        if self.size < 12.0 {
            text.truncate(1);
        }
        let size = (self.size * 0.46).round().max(6.5);
        let mono = s.theme().mono_font.clone();
        let mut block = TextBlock::new(text, 0.0, 0.0)
            .with_size(size)
            .with_font_opt(mono)
            .with_weight(crate::Weight::SEMIBOLD)
            .with_letter_spacing(size * -0.02)
            .with_color_f32(oklch(0.93, 0.04, h, 1.0))
            .with_shadow(0, 0, 0, MONOGRAM_SHADOW, 0.0, 1.0, 0.0);
        let (w, _) = list.measure_block(&block);
        block.x = rect.x + (rect.width - w) * 0.5;
        block.y = crate::text::vcentered_line_y(rect.y, rect.height, size);
        list.text(block);
    }

    fn draw_plate(
        &self,
        list: &mut DrawList,
        rect: Rect,
        edge: [f32; 4],
        image: Option<SpriteId>,
        fill: Option<Background>,
    ) {
        let radius = self.radius();
        list.box_shadow_outset(
            rect,
            CornerRadii::uniform(radius),
            BoxShadow {
                offset: [0.0, 1.0],
                blur: 1.0,
                color: PLATE_DROP,
                ..BoxShadow::default()
            },
        );
        let plate = fill.unwrap_or(Background::LinearGradient {
            start: super::sheen_over(PLATE_BASE, [1.0, 1.0, 1.0, 0.2]),
            end: super::sheen_over(PLATE_BASE, [0.0, 0.0, 0.0, 0.12]),
            axis: GradientAxis::Vertical,
        });
        self.edged(list, rect, edge, plate);
        if let Some(sprite) = image {
            // The image never reaches the bevel: cover keeps a 1 px lip,
            // contain a margin that grows with the tile.
            let pad = if self.fit == ImageFit::Contain {
                (self.size / 8.0).round().max(2.0)
            } else {
                1.0
            };
            Image::sprite(sprite)
                .fit(self.fit)
                .draw(rect.inset(pad), list);
        }
        // The bevel sits over the image so a photo can't paint over it.
        let inner = rect.inset(1.0);
        let [top, left, right, bottom] = SHEEN;
        list.quad(inner.x, inner.y, inner.width, 1.0, top);
        list.quad(inner.x, inner.y, 1.0, inner.height, left);
        list.quad(inner.right() - 1.0, inner.y, 1.0, inner.height, right);
        list.quad(inner.x, inner.bottom() - 1.0, inner.width, 1.0, bottom);
    }

    fn draw_slot(&self, list: &mut DrawList, s: &StyleResolver, rect: Rect) {
        let radius = self.radius();
        list.rounded_rect(rect, radius, s.color(StyleKey::WellDeep));
        list.hatch(rect, HATCH_STEP, HATCH);
        let edge = if self.selected {
            SLOT_EDGE_SELECTED
        } else {
            SLOT_EDGE
        };
        list.dashed_rect_outline(rect, DASH, edge);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;
    use crate::color::text_color;

    /// Forge's `hueOf`, written out the JavaScript way for the comparison.
    fn js_hue(s: &str) -> f32 {
        let mut h: f64 = 0.0;
        for unit in s.encode_utf16() {
            h = ((h * 31.0 + f64::from(unit)) as u64 % (1u64 << 32)) as f64;
        }
        (h as u64 % 360) as f32
    }

    #[test]
    fn hue_matches_the_designs_hash() {
        for name in [
            "agent-ui",
            "sorry-pulumi2",
            "AJME-54",
            "",
            "a very long project name",
            "café",
        ] {
            assert_eq!(hue_of(name), js_hue(name), "{name:?}");
        }
        assert!(hue_of("agent-ui") < 360.0);
    }

    #[test]
    fn initials_take_two_words_or_two_letters() {
        assert_eq!(initials("agent-ui"), "AU");
        assert_eq!(initials("sorry_pulumi2"), "SP");
        assert_eq!(initials("dotfiles"), "DO");
        assert_eq!(initials(".dotfiles"), "DO");
        assert_eq!(initials("x"), "X");
        assert_eq!(initials("--"), "?");
        assert_eq!(initials(""), "?");
        assert_eq!(initials("AJME-54"), "A5");
    }

    fn draw(thumb: Thumb) -> DrawList {
        let theme = Theme::default();
        let mut list = DrawList::new();
        thumb.draw(10.0, 20.0, &mut list, &StyleResolver::new(&theme));
        list
    }

    #[test]
    fn resolution_order_prefers_image_then_fill_then_color_then_name_then_glyph() {
        let full = Thumb::new()
            .fill(Background::Solid([1.0, 0.0, 0.0, 1.0]))
            .color([0.0, 1.0, 0.0, 1.0])
            .name("agent-ui")
            .glyph(PhosphorIcon::Folder);
        assert!(matches!(full.face(), Face::Plate { .. }));
        let no_fill = Thumb::new().color([0.0, 1.0, 0.0, 1.0]).name("agent-ui");
        assert_eq!(no_fill.face(), Face::Swatch([0.0, 1.0, 0.0, 1.0]));
        let named = Thumb::new().name("agent-ui").glyph(PhosphorIcon::Folder);
        assert_eq!(named.face(), Face::Monogram("agent-ui"));
        assert_eq!(
            Thumb::new().glyph(PhosphorIcon::Folder).face(),
            Face::Glyph(PhosphorIcon::Folder)
        );
        assert_eq!(Thumb::new().face(), Face::Slot);
        assert_eq!(named.slot(true).face(), Face::Slot, "slot beats a name");
    }

    #[test]
    fn a_monogram_is_centred_mono_on_its_hue() {
        let list = draw(Thumb::new().name("agent-ui").size(14.0));
        let text = list
            .texts
            .iter()
            .find(|t| t.content == "AU")
            .expect("monogram text");
        let h = hue_of("agent-ui");
        assert_eq!(text.color, text_color(oklch(0.93, 0.04, h, 1.0)));
        assert_eq!(text.font_size, 6.5f32.max((14.0f32 * 0.46).round()));
        assert_eq!(text.weight, crate::Weight::SEMIBOLD);
        assert!(
            text.x > 10.0 && text.x < 17.0,
            "inside the tile's left half"
        );
    }

    #[test]
    fn a_small_monogram_shows_one_letter() {
        let list = draw(Thumb::new().name("agent-ui").size(10.0));
        assert!(list.texts.iter().any(|t| t.content == "A"));
    }

    #[test]
    fn size_sets_the_rect_and_radius() {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let r = Thumb::new().name("x").size(24.0).draw(
            1.0,
            2.0,
            &mut list,
            &StyleResolver::new(&theme),
        );
        assert_eq!(r, Rect::new(1.0, 2.0, 24.0, 24.0));
        assert_eq!(Thumb::new().size(24.0).radius(), 3.0);
        assert_eq!(Thumb::new().size(14.0).radius(), 2.0);
        assert_eq!(Thumb::new().size(14.0).round(true).radius(), 7.0);
    }

    #[test]
    fn a_glyph_draws_one_icon_and_a_slot_draws_none() {
        assert_eq!(
            draw(Thumb::new().glyph(PhosphorIcon::Folder))
                .icons_msdf
                .len(),
            1
        );
        let slot = draw(Thumb::new());
        assert!(slot.icons_msdf.is_empty());
        assert!(slot.texts.is_empty());
    }
}
