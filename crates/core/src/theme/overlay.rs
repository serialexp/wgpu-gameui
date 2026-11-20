//! Forge Overlay: Forge's component vocabulary rebuilt to sit on top of a
//! game. See [`Theme::overlay`].
//!
//! It keeps Forge's raised keys, sunken wells, two-line edges, 2 px press
//! travel, instant state and semantic hues (25 / 75 / 145). What changes:
//!
//! - every neutral loses its chroma, so a panel takes on no scene's colour
//!   and fights no art direction;
//! - panels are translucent smoked glass (`rgb(20 20 22 / 0.86)`, the floor
//!   for 4.5:1 captions over a white sky);
//! - one accent per title, bone by default, from which every accent token is
//!   derived;
//! - type and hit sizes read at monitor and TV distance: body 15 px, rows
//!   34 px, keys 36 px; corners 3 px, panels 6 px;
//! - Forge's 24-step ink ladder collapses to six steps.
//!
//! Two parts of the design the renderer can't draw yet: the glass's backdrop
//! blur (a panel can blur the scene under it through
//! [`UiRenderer::blur_backdrop`](crate::UiRenderer::blur_backdrop), but no
//! panel does that on its own), and the text shadows that carve text into a
//! panel or lift it off the world.

use crate::chrome::{
    Background, ChromeTheme, Edge, EdgeStyle, EdgeWidths, FloatingSurfaceChrome, GradientAxis,
    QuadStyle, StructuralLine,
};
use crate::color::{HUE_DANGER, HUE_OK, HUE_WARN, hex, oklch, rgba8, to_oklch, with_alpha};
use crate::preset::{ColorFamily, ColorSeed, ThemeSeeds};
use crate::shadow::{BoxShadow, CornerRadii};
use crate::typography::TypeScale;
use crate::{Ink, Theme, Typography};

/// The design's default accent, a warm bone (`#ede6d8`).
pub(crate) const BONE: [f32; 4] = hex(0xede6d8);

/// The six steps of the overlay's ink ladder.
const INK_MAX: [f32; 4] = hex(0xf6f6f4);
const INK: [f32; 4] = hex(0xe8e8e5);
const INK_2: [f32; 4] = hex(0xcbcbc7);
const INK_MUTED: [f32; 4] = hex(0xb2b2ad);
const INK_CAPTION: [f32; 4] = hex(0x9e9e99);
const INK_DISABLED: [f32; 4] = hex(0x6e6e6a);
const INK_DISABLED_GLYPH: [f32; 4] = hex(0x5e5e5b);
const INK_DISABLED_KEY: [f32; 4] = hex(0x52524f);
const INK_EMPTY: [f32; 4] = hex(0x4a4a47);

/// The glass every panel is made of, at the design's default opacity.
const GLASS_ALPHA: f32 = 0.86;
const GLASS: [f32; 4] = rgba8([20, 20, 22], GLASS_ALPHA);
const GLASS_2: [f32; 4] = rgba8([32, 32, 35], GLASS_ALPHA);

/// Corner radius of keys, wells and chips, and of panels.
const RADIUS: f32 = 3.0;
const RADIUS_PANEL: f32 = 6.0;

const fn white(alpha: f32) -> [f32; 4] {
    [1.0, 1.0, 1.0, alpha]
}

const fn black(alpha: f32) -> [f32; 4] {
    [0.0, 0.0, 0.0, alpha]
}

/// CSS `color-mix(in oklab, a, b t)`: `t` of the way from `a` to `b`, both
/// opaque.
fn mix_oklab(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let lab = |color: [f32; 4]| {
        let [l, c, h] = to_oklch(color);
        let (sin, cos) = h.to_radians().sin_cos();
        [l, c * cos, c * sin]
    };
    let (p, q) = (lab(a), lab(b));
    let [l, x, y] = [0, 1, 2].map(|i| p[i] + (q[i] - p[i]) * t);
    oklch(l, x.hypot(y), y.atan2(x).to_degrees(), a[3])
}

/// CSS `filter: brightness(k)`: each sRGB channel scaled by `k`.
fn brightness(color: [f32; 4], k: f32) -> [f32; 4] {
    let [r, g, b, a] = color;
    [(r * k).min(1.0), (g * k).min(1.0), (b * k).min(1.0), a]
}

/// The accent and the tokens the design derives from it (`--accent-top`,
/// `--accent-bottom`, the wash, ring and glow), plus the ink drawn on it.
#[derive(Clone, Copy)]
struct Accent {
    base: [f32; 4],
    top: [f32; 4],
    bottom: [f32; 4],
    wash: [f32; 4],
    ring: [f32; 4],
    glow: [f32; 4],
    ink: [f32; 4],
    /// `--accent-tick`, `--accent-match`, `--accent-grip`, `--accent-glyph`.
    tick: [f32; 4],
    matched: [f32; 4],
    grip: [f32; 4],
    glyph: [f32; 4],
    /// `--accent-key-hover-*` and `--accent-key-pressed-*`.
    hover_top: [f32; 4],
    hover_bottom: [f32; 4],
    pressed_top: [f32; 4],
    pressed_bottom: [f32; 4],
}

impl Accent {
    fn new(base: [f32; 4]) -> Self {
        let base = with_alpha(base, 1.0);
        let [l, c, h] = to_oklch(base);
        // Dark ink on a light accent, light on a dark one, both carrying a
        // trace of the accent's hue as the design's `#16140f` does.
        let ink = if l >= 0.6 {
            oklch(0.2, c.min(0.01), h, 1.0)
        } else {
            oklch(0.97, c.min(0.02), h, 1.0)
        };
        Self {
            base,
            top: mix_oklab(base, white(1.0), 0.16),
            bottom: mix_oklab(base, black(1.0), 0.18),
            wash: with_alpha(base, 0.15),
            ring: with_alpha(base, 0.36),
            glow: with_alpha(base, 0.6),
            ink,
            tick: mix_oklab(base, white(1.0), 0.12),
            matched: mix_oklab(base, white(1.0), 0.20),
            grip: mix_oklab(base, white(1.0), 0.10),
            glyph: mix_oklab(base, black(1.0), 0.08),
            hover_top: mix_oklab(base, white(1.0), 0.26),
            hover_bottom: mix_oklab(base, black(1.0), 0.10),
            pressed_top: mix_oklab(base, black(1.0), 0.18),
            pressed_bottom: mix_oklab(base, black(1.0), 0.26),
        }
    }
}

impl Theme {
    /// The accent [`Theme::overlay`] uses: a warm bone, `#ede6d8`. The design
    /// also tries amber `#f0b955`, cyan `#66cfe0` and violet `#b8a2ff`.
    pub const OVERLAY_ACCENT: [f32; 4] = BONE;

    /// Forge Overlay, the default theme: Forge's components rebuilt to sit on
    /// any game.
    ///
    /// The same raised keys, sunken wells, two-line edges, press travel and
    /// instant state as [`Theme::forge`], with chroma-free smoked-glass
    /// panels in place of Forge's opaque blue-grey, a bone accent in place of
    /// its fixed teal, and a type and hit scale read at monitor and TV
    /// distance (body text 15 px, rows 34 px, keys 36 px). The accent is
    /// still for state only: selection, focus, latched and live.
    ///
    /// The design's backdrop blur is not applied: a panel is translucent, and
    /// blurring the scene under it is up to the app (see
    /// [`UiRenderer::blur_backdrop`](crate::UiRenderer::blur_backdrop)).
    pub fn overlay() -> Self {
        Self::overlay_with_accent(Self::OVERLAY_ACCENT)
    }

    /// [`Theme::overlay`] with another accent. Every accent token (key
    /// faces, focus rings, selection washes, latched keys, the chrome's
    /// grips and glows, and the ink drawn on the accent) is derived from
    /// `accent`, which is how the design lets each title set its own. The
    /// alpha of `accent` is ignored.
    pub fn overlay_with_accent(accent: [f32; 4]) -> Self {
        let accent = Accent::new(accent);
        let mut t = Self::forge();

        // Neutrals lose their chroma, and anything left of Forge's teal
        // follows the accent, in the theme and in the chrome alike. The
        // explicit values below replace most of what this derives; it is
        // what is left of Forge (editor chrome the design doesn't cover).
        let [l, c, h] = to_oklch(accent.base);
        ThemeSeeds {
            surface: ColorSeed {
                chroma: 0.0,
                ..ColorSeed::forge(ColorFamily::Surface)
            },
            ink: ColorSeed {
                chroma: 0.0,
                ..ColorSeed::forge(ColorFamily::Ink)
            },
            accent: ColorSeed {
                hue: h,
                chroma: c / 0.11,
                lightness: l - 0.74,
            },
            ..ThemeSeeds::default()
        }
        .apply(&mut t);

        // Surfaces.
        t.background = hex(0x0e0e10);
        t.panel = GLASS;
        t.panel_border = black(0.55);
        t.button = rgba8([0x2a, 0x2a, 0x2c], 1.0);
        t.button_hover = rgba8([0x33, 0x33, 0x35], 1.0);
        t.button_pressed = rgba8([0x1c, 0x1c, 0x1e], 1.0);
        t.button_border = black(0.5);
        t.input_background = black(0.36);
        t.input_border = black(0.55);
        t.input_focus_border = accent.base;

        // Ink.
        t.text = INK;
        t.text_dim = INK_MUTED;
        t.text_highlight = INK_MAX;
        t.ink = Ink::ALL.map(|role| match role {
            Ink::White => white(1.0),
            Ink::Max | Ink::Value => INK_MAX,
            Ink::Emph | Ink::Menu | Ink::Title | Ink::Row | Ink::Primary => INK,
            Ink::Second | Ink::Icon | Ink::Cell | Ink::Chip => INK_2,
            Ink::Glyph | Ink::Tab | Ink::TipHint | Ink::Body2 | Ink::Muted => INK_MUTED,
            Ink::Shortcut | Ink::Label | Ink::Caption | Ink::Dim | Ink::Brand => INK_CAPTION,
            Ink::Disabled => INK_DISABLED,
            Ink::DisabledGlyph => INK_DISABLED_GLYPH,
            Ink::DisabledKey => INK_DISABLED_KEY,
            Ink::Empty => INK_EMPTY,
            Ink::OnSelectionSecond => INK_2,
        });

        // The accent.
        t.accent = accent.base;
        t.accent_tick = accent.tick;
        t.focus_ring = accent.base;
        t.accent_face_top = accent.top;
        t.accent_face_bottom = accent.bottom;
        t.accent_face_top_hover = accent.hover_top;
        t.accent_face_bottom_hover = accent.hover_bottom;
        t.accent_face_top_pressed = accent.pressed_top;
        t.accent_face_bottom_pressed = accent.pressed_bottom;
        t.on_accent = accent.ink;
        // A selected row is the accent wash under the brightest ink
        // (`.fo-row.is-selected`), not a solid plate.
        t.selection = accent.wash;
        t.on_selection = INK_MAX;
        // `.fo-chip.is-on`, `.fo-badge.is-accent`: the wash, ringed and
        // inked in the accent.
        t.accent_held_top = accent.wash;
        t.accent_held_bottom = accent.wash;
        t.accent_held_edge = accent.ring;
        t.on_accent_held = accent.base;
        // `.fo-range` thumb.
        t.knob_top = hex(0xf4f4f1);
        t.knob_bottom = hex(0xa8a8a4);
        // A latched key is a pressed key ringed and inked in the accent.
        t.latch_face_top = white(0.07);
        t.latch_face_bottom = white(0.03);
        t.latch_shade = black(0.45);
        t.latch_highlight = accent.ring;
        t.on_latch = accent.base;
        t.accent_match = accent.matched;
        t.accent_dirty = accent.base;
        t.accent_grip = accent.grip;
        t.accent_glyph = accent.glyph;
        t.accent_ring = accent.ring;
        t.accent_glow = accent.glow;

        // Semantic colours: Forge's hues, the design's lightness and chroma.
        let danger = oklch(0.7, 0.17, HUE_DANGER, 1.0);
        let warn = oklch(0.83, 0.14, HUE_WARN, 1.0);
        let ok = oklch(0.78, 0.15, HUE_OK, 1.0);
        let danger_top = oklch(0.6, 0.17, HUE_DANGER, 1.0);
        let danger_bottom = oklch(0.5, 0.17, HUE_DANGER, 1.0);
        t.danger_face_top = danger_top;
        t.danger_face_bottom = danger_bottom;
        t.danger_face_top_hover = brightness(danger_top, 1.08);
        t.danger_face_bottom_hover = brightness(danger_bottom, 1.08);
        t.danger_face_top_pressed = danger_bottom;
        t.danger_face_bottom_pressed = danger_bottom;
        t.on_danger = hex(0xfff0ee);
        t.error = danger;
        t.danger_text = danger;
        t.warning = warn;
        t.warn_meta = warn;
        t.warn_rule = warn;
        t.success = ok;
        t.status_ok = ok;

        // Tabs are a raised key inside a well.
        t.tab_inactive = black(0.36);
        t.tab_active = white(0.25);
        t.tab_hover = white(0.05);
        t.tab_border = black(0.5);

        // Meters: a neutral fill unless something is wrong.
        t.progress_background = black(0.42);
        t.progress_fill = hex(0xc9c9c5);
        t.progress_fill_low = oklch(0.63, 0.175, HUE_DANGER, 1.0);
        t.progress_fill_medium = oklch(0.78, 0.13, HUE_WARN, 1.0);

        // Materials: `--key-face*`, `--key-inset*`, `--plinth`, the well.
        t.plinth = black(0.55);
        t.face_top = white(0.17);
        t.face_bottom = white(0.07);
        t.face_top_hover = white(0.25);
        t.face_bottom_hover = white(0.11);
        t.face_top_pressed = white(0.07);
        t.face_bottom_pressed = white(0.03);
        t.edge_highlight = white(0.22);
        t.edge_highlight_hover = white(0.32);
        t.edge_highlight_pressed = white(0.05);
        t.inner_shadow = black(0.5);
        t.edge_shadow = white(0.08);

        // Rows and boxes.
        t.row_zebra = white(0.02);
        t.row_hover = white(0.06);
        t.row_held = white(0.1);
        t.well_deep = black(0.42);
        t.edge_hard = black(0.6);

        // Geometry and type: `--h-row` 34, `--key` 36, `--h-field` 34;
        // caption 11, meta 12, row 14, body 15, title 18.
        t.padding = 8.0;
        t.spacing = 8.0;
        t.border_radius = RADIUS;
        t.panel_radius = RADIUS_PANEL;
        t.font_size = 15.0;
        t.font_size_title = 18.0;
        t.text_sizes = [11.0, 12.0, 13.0, 14.0, 14.0];
        t.typography = Typography::from_scale(&TypeScale {
            body: 15.0,
            title: 18.0,
            caption: 11.0,
            meta: 12.0,
            dense: 13.0,
            row: 14.0,
            menu: 14.0,
            ..TypeScale::FORGE
        });
        t.button_height = 36.0;
        t.input_height = 34.0;
        t.list_row_height = 34.0;
        t.menu_bar_height = 40.0;
        t.menu_row_height = 34.0;
        t.menu_item_min_width = 260.0;
        t.menu_accel_gap = 12.0;
        t.toolbar_button_size = 36.0;
        t.toolbar_padding = 4.0;
        t.dock_tab_height = 36.0;
        t.dock_splitter_width = 8.0;
        t.window_title_height = 42.0;
        t.status_bar_height = 36.0;
        t.doc_tab_height = 40.0;
        t.switch_width = 46.0;
        t.switch_height = 26.0;
        t.chip_height = 24.0;
        t.key_header_size = 28.0;
        t.key_status_size = 28.0;
        t.slider_track = 8.0;
        t.scrollbar_width = 14.0;
        t.panel_padding = 16.0;
        t.section_gap = 16.0;
        t.sheet_padding = 4.0;
        t.inspector_width = 340.0;
        t.inspector_label_width = 96.0;
        t.tab_pad_x = 14.0;
        t.key_gap = 6.0;
        t.field_gap = 10.0;
        t.chip_gap = 6.0;
        t.grid_gap = 24.0;
        t.well_pad_x = 12.0;
        t.well_pad_y = 6.0;
        t.key_pad_x = 16.0;
        t.key_pad_y = 10.0;
        t.pill_radius = 999.0;

        overlay_chrome(&mut t.chrome, &accent);
        t
    }
}

/// Re-make Forge's chrome (already neutralised and re-accented) in the
/// overlay's materials: Forge's 1 px corners become 3 px and its 2–3 px
/// panel corners 6 px, the floating surfaces become glass, and the accent's
/// places in the chrome take the design's accent tokens.
fn overlay_chrome(chrome: &mut ChromeTheme, accent: &Accent) {
    chrome.map_corner_radii(|radii| {
        let round = |r: f32| match r {
            r if r <= 0.0 => 0.0,
            r if r <= 1.0 => RADIUS,
            _ => RADIUS_PANEL,
        };
        *radii = CornerRadii::new(
            round(radii.top_left),
            round(radii.top_right),
            round(radii.bottom_right),
            round(radii.bottom_left),
        );
    });

    let glass = |background: [f32; 4]| QuadStyle {
        background: Background::Solid(background),
        border_widths: EdgeWidths::uniform(1.0),
        border_color: black(0.55),
        corner_radii: CornerRadii::uniform(RADIUS_PANEL),
    };
    let sheet_glass = QuadStyle {
        background: Background::LinearGradient {
            start: GLASS_2,
            end: GLASS,
            axis: GradientAxis::Vertical,
        },
        border_color: black(0.6),
        ..glass(GLASS)
    };
    let shadow = |y: f32, blur: f32, color: [f32; 4], inset: bool| BoxShadow {
        offset: [0.0, y],
        blur,
        spread: 0.0,
        color,
        inset,
    };
    let top_light = |alpha: f32| StructuralLine {
        edge: Edge::Top,
        offset: 1.0,
        style: EdgeStyle {
            thickness: 1.0,
            color: white(alpha),
        },
    };
    let none = StructuralLine {
        edge: Edge::Bottom,
        offset: 0.0,
        style: EdgeStyle {
            thickness: 0.0,
            color: [0.0; 4],
        },
    };
    // `--shadow-panel` and `--shadow-sheet`: the rim is the border, the lit
    // top edge a line; each keeps its largest drop.
    // The remaining layers are the softer second drop and the 1 px inner
    // rim (`inset 0 0 0 1px`).
    let ring = |alpha: f32| BoxShadow {
        offset: [0.0, 0.0],
        blur: 0.0,
        spread: 1.0,
        color: white(alpha),
        inset: true,
    };
    let panel = FloatingSurfaceChrome {
        surface: glass(GLASS),
        shadow: shadow(14.0, 36.0, black(0.4), false),
        rim: [shadow(2.0, 6.0, black(0.3), false), ring(0.045)],
        lines: [top_light(0.12), none],
    };
    let sheet = FloatingSurfaceChrome {
        surface: sheet_glass,
        shadow: shadow(24.0, 60.0, black(0.55), false),
        rim: [shadow(4.0, 12.0, black(0.35), false), ring(0.05)],
        lines: [top_light(0.14), none],
    };
    // `--surface-dropdown` and `--surface-tooltip`: near-opaque, not the
    // sheet's gradient.
    let near_opaque = glass(rgba8([24, 24, 27], 0.97));
    chrome.dropdown = FloatingSurfaceChrome {
        surface: near_opaque,
        ..sheet
    };
    chrome.popover = sheet;
    chrome.toast = sheet;
    chrome.hud = panel;
    chrome.tooltip = FloatingSurfaceChrome {
        surface: near_opaque,
        ..sheet
    };
    chrome.menu_sheet.surface = sheet_glass;
    chrome.menu_sheet.shadows = [
        shadow(24.0, 60.0, black(0.55), false),
        shadow(4.0, 12.0, black(0.35), false),
    ];
    chrome.toolbar.popup = chrome.menu_sheet;
    chrome.window.surface = glass(GLASS);
    chrome.window.shadow = shadow(14.0, 36.0, black(0.4), false);
    chrome.window.lines = [top_light(0.12), none];
    chrome.sheet.surface = sheet_glass;
    chrome.sheet.shadows = [
        shadow(24.0, 60.0, black(0.55), false),
        shadow(4.0, 12.0, black(0.35), false),
        shadow(1.0, 0.0, white(0.14), true),
        shadow(-1.0, 0.0, black(0.5), true),
    ];
    chrome.sheet.footer = Background::Solid(black(0.18));

    // The accent's places: a selected row is the accent wash, a latched tool
    // a pressed one ringed in the accent, a dragged grip the accent.
    let list = &mut chrome.group_list;
    list.selected = Background::Solid(accent.wash);
    list.selected_highlight.color = white(0.1);
    list.selected_shade.color = black(0.25);
    list.selected_ink = INK_MAX;
    list.selected_sub_ink = INK_2;
    let toolbar = &mut chrome.toolbar;
    toolbar.tool_latched.background = Background::LinearGradient {
        start: white(0.07),
        end: white(0.03),
        axis: GradientAxis::Vertical,
    };
    toolbar.tool_insets[3] = [
        shadow(2.0, 4.0, black(0.45), true),
        BoxShadow {
            offset: [0.0, 0.0],
            blur: 0.0,
            spread: 1.0,
            color: accent.ring,
            inset: true,
        },
    ];
    toolbar.grip_colors[2] = accent.base;
    chrome.splitter.grip_dragging = accent.base;
    chrome.splitter.dragging_glow.color = accent.glow;
    chrome.status_bar.latched_inset = shadow(2.0, 4.0, black(0.45), true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StyleKey;
    use crate::color::{srgb_channel_to_linear, to_rgba8};
    use crate::style::StyleValue;

    /// Within one 8-bit step of `want` on every channel.
    fn assert_near(got: [f32; 4], want: [u8; 3], what: &str) {
        let got = to_rgba8(got);
        for i in 0..3 {
            assert!(
                (got[i] as i32 - want[i] as i32).abs() <= 1,
                "{what}: {got:?}, want {want:?}"
            );
        }
    }

    /// WCAG contrast ratio of two opaque colours.
    fn contrast(a: [f32; 4], b: [f32; 4]) -> f32 {
        let luminance = |c: [f32; 4]| {
            let [r, g, b] = [c[0], c[1], c[2]].map(srgb_channel_to_linear);
            0.2126 * r + 0.7152 * g + 0.0722 * b
        };
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// `top` composited over opaque `under`, in sRGB as the renderer blends.
    fn over(top: [f32; 4], under: [f32; 4]) -> [f32; 4] {
        let a = top[3];
        let [r, g, b] = [0, 1, 2].map(|i| top[i] * a + under[i] * (1.0 - a));
        [r, g, b, 1.0]
    }

    /// Every colour a theme holds: its keys and its chrome.
    fn colours(theme: &Theme) -> Vec<[f32; 4]> {
        let mut all: Vec<_> = StyleKey::builtin()
            .filter_map(|key| theme.get(key).and_then(StyleValue::as_color))
            .collect();
        let mut chrome = theme.chrome;
        chrome.map_colors(|c| all.push(*c));
        all
    }

    #[test]
    fn accent_tokens_mix_as_css_color_mix_does() {
        // Reference values from Chrome's `color-mix(in oklab, …)`.
        let bone = Theme::overlay();
        assert_near(bone.accent_face_top, [240, 234, 222], "bone top");
        assert_near(bone.accent_face_bottom, [182, 176, 165], "bone bottom");
        let violet = Theme::overlay_with_accent(hex(0xb8a2ff));
        assert_near(violet.accent_face_top, [195, 177, 255], "violet top");
        assert_near(violet.accent_face_bottom, [140, 123, 196], "violet bottom");
        let cyan = Theme::overlay_with_accent(hex(0x66cfe0));
        assert_near(cyan.accent_face_bottom, [77, 158, 172], "cyan bottom");
        assert_near(bone.danger_face_top, [210, 76, 73], "danger top");
        assert_near(bone.error, [246, 109, 103], "danger");
    }

    #[test]
    fn the_default_theme_is_the_overlay_in_bone() {
        let t = Theme::overlay();
        assert_eq!(t.accent, hex(0xede6d8));
        assert_eq!(t.panel, rgba8([20, 20, 22], 0.86));
        assert_eq!(t.text, hex(0xe8e8e5));
        assert_eq!(t.text_highlight, hex(0xf6f6f4));
        assert_eq!(t.font_size, 15.0);
        assert_eq!(t.list_row_height, 34.0);
        assert_eq!(t.button_height, 36.0);
        assert_eq!(t.border_radius, 3.0);
        assert_eq!(t.panel_radius, 6.0);
        assert_eq!(t.accent_ring, with_alpha(hex(0xede6d8), 0.36));
    }

    #[test]
    fn neutrals_lose_forges_blue() {
        // The design's greys keep a trace of chroma (`rgb(20 20 22)`, and
        // inks as warm as `#b2b2ad` at 0.007); Forge's blue-greys reach
        // 0.018, and most of them would fail this bound.
        const BOUND: f32 = 0.008;
        let forge = Theme::forge();
        let t = Theme::overlay();
        let (mut neutrals, mut forge_over) = (0, 0);
        for key in StyleKey::builtin() {
            let family = ColorFamily::of_key(key, &forge);
            if !matches!(family, Some(ColorFamily::Surface | ColorFamily::Ink)) {
                continue;
            }
            // Keys the overlay inks in the accent on purpose.
            if t.get(key).unwrap().as_color() == Some(t.accent) {
                continue;
            }
            neutrals += 1;
            let chroma = |theme: &Theme| to_oklch(theme.get(key).unwrap().as_color().unwrap())[1];
            if chroma(&forge) >= BOUND {
                forge_over += 1;
            }
            assert!(chroma(&t) < BOUND, "{key:?} keeps chroma {}", chroma(&t));
        }
        assert!(forge_over * 2 > neutrals, "{forge_over} of {neutrals}");
    }

    #[test]
    fn no_teal_is_left_once_the_accent_moves() {
        // Violet sits far from Forge's teal (hue 200) and from every
        // semantic hue, so any colour near 200 is Forge's, left behind.
        let t = Theme::overlay_with_accent(hex(0xb8a2ff));
        for colour in colours(&t) {
            let [_, chroma, hue] = to_oklch(colour);
            let off = (hue - 200.0).abs();
            assert!(
                chroma < 0.03 || off > 25.0,
                "{:?} (chroma {chroma}, hue {hue}) is Forge's teal",
                to_rgba8(colour)
            );
        }
    }

    #[test]
    fn ink_on_the_accent_reads_for_every_suggested_accent() {
        for accent in [0xede6d8, 0xf0b955, 0x66cfe0, 0xb8a2ff] {
            let t = Theme::overlay_with_accent(hex(accent));
            for face in [t.accent_face_top, t.accent_face_bottom] {
                let ratio = contrast(t.on_accent, face);
                assert!(ratio >= 4.5, "#{accent:06x}: {ratio:.2}:1 on {face:?}");
            }
        }
        // A dark accent gets light ink.
        let t = Theme::overlay_with_accent(hex(0x3a2a6a));
        assert!(to_oklch(t.on_accent)[0] > 0.9);
    }

    #[test]
    fn captions_hold_four_and_a_half_to_one_on_glass_over_a_white_sky() {
        let t = Theme::overlay();
        let ground = over(t.panel, white(1.0));
        let caption = t
            .get(StyleKey::Ink(Ink::Caption))
            .unwrap()
            .as_color()
            .unwrap();
        let ratio = contrast(caption, ground);
        assert!(ratio >= 4.5, "{ratio:.2}:1");
    }

    #[test]
    fn a_selected_row_is_the_wash_and_its_inks_read_on_it() {
        for accent in [
            Theme::OVERLAY_ACCENT,
            hex(0xf0b955),
            hex(0x66cfe0),
            hex(0xb8a2ff),
        ] {
            let t = Theme::overlay_with_accent(accent);
            assert_eq!(t.selection, with_alpha(accent, 0.15));
            // The worst ground: the wash on the most transparent glass, over
            // a white sky.
            let ground = over(t.selection, over(t.panel, white(1.0)));
            let second = t
                .get(StyleKey::Ink(Ink::OnSelectionSecond))
                .unwrap()
                .as_color()
                .unwrap();
            for ink in [t.on_selection, second] {
                let ratio = contrast(ink, ground);
                assert!(ratio >= 4.5, "{accent:?}: {ratio:.2}:1");
            }
        }
    }

    #[test]
    fn panels_carry_the_designs_three_shadow_layers_and_forge_one() {
        let overlay = Theme::overlay().chrome;
        assert_eq!(overlay.hud.shadows().as_slice().len(), 3);
        assert_eq!(overlay.tooltip.shadows().as_slice().len(), 3);
        assert!(overlay.hud.shadows().as_slice()[2].inset);
        assert_eq!(Theme::forge().chrome.hud.shadows().as_slice().len(), 1);
    }

    #[test]
    fn the_ink_ladder_keeps_the_designs_three_disabled_steps() {
        let t = Theme::overlay();
        let ink = |role| t.get(StyleKey::Ink(role)).unwrap().as_color().unwrap();
        assert_eq!(ink(Ink::Disabled), hex(0x6e6e6a));
        assert_eq!(ink(Ink::DisabledGlyph), hex(0x5e5e5b));
        assert_eq!(ink(Ink::DisabledKey), hex(0x52524f));
        assert_eq!(ink(Ink::Empty), hex(0x4a4a47));
        assert_eq!(ink(Ink::Cell), hex(0xcbcbc7));
        assert_eq!(ink(Ink::Brand), hex(0x9e9e99));
    }

    #[test]
    fn accent_tints_mix_as_the_design_declares() {
        // `--accent-tick` white 12%, `--accent-match` 20%, `--accent-grip`
        // 10%, `--accent-glyph` black 8%, key hover white 26% / black 10%,
        // key pressed black 18% / 26%.
        let t = Theme::overlay();
        let bone = t.accent;
        assert_eq!(t.accent_tick, mix_oklab(bone, white(1.0), 0.12));
        assert_eq!(t.accent_match, mix_oklab(bone, white(1.0), 0.20));
        assert_eq!(t.accent_grip, mix_oklab(bone, white(1.0), 0.10));
        assert_eq!(t.accent_glyph, mix_oklab(bone, black(1.0), 0.08));
        assert_eq!(t.accent_face_top_hover, mix_oklab(bone, white(1.0), 0.26));
        assert_eq!(
            t.accent_face_bottom_hover,
            mix_oklab(bone, black(1.0), 0.10)
        );
        assert_eq!(t.accent_face_top_pressed, mix_oklab(bone, black(1.0), 0.18));
        assert_eq!(
            t.accent_face_bottom_pressed,
            mix_oklab(bone, black(1.0), 0.26)
        );
    }

    #[test]
    fn floating_surfaces_are_glass_with_panel_corners() {
        let chrome = Theme::overlay().chrome;
        for surface in [chrome.popover, chrome.toast, chrome.hud] {
            assert_eq!(surface.surface.corner_radii, CornerRadii::uniform(6.0));
            assert!(matches!(
                surface.surface.background,
                Background::Solid(c) | Background::LinearGradient { end: c, .. } if c == GLASS
            ));
        }
        // The dropdown and the tooltip are the design's near-opaque surface.
        for surface in [chrome.dropdown, chrome.tooltip] {
            assert_eq!(surface.surface.corner_radii, CornerRadii::uniform(6.0));
            assert_eq!(
                surface.surface.background,
                Background::Solid(rgba8([24, 24, 27], 0.97))
            );
        }
        // Forge's machined 1 px corners became 3 px.
        assert_eq!(
            chrome.dock.active_tab.corner_radii,
            CornerRadii::uniform(3.0)
        );
    }
}
