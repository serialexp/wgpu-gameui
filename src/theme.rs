//! UI theming - colors, fonts, spacing.

use crate::chrome::ChromeTheme;
use crate::color::{hex, oklch, rgba8};
use crate::style::{CustomStyles, StyleKey, StyleValue};
use crate::text::{FontHandle, TextBlock};

/// UI theme with colors and styling.
#[derive(Clone)]
pub struct Theme {
    // Colors
    /// Window/screen backdrop fill behind all UI.
    pub background: [f32; 4],
    /// Fullscreen dim behind a modal/menu/pause overlay (see
    /// [`StyleKey::Scrim`](crate::StyleKey::Scrim)). Drawn over the scene (or
    /// the game world showing through a `LayerStack` base) and under the
    /// overlay's contents by [`draw_scrim`](crate::draw_scrim).
    pub scrim: [f32; 4],
    /// Panel/container surface fill.
    pub panel: [f32; 4],
    /// Border stroke around panels/containers.
    pub panel_border: [f32; 4],
    /// Button fill in its resting (idle) state.
    pub button: [f32; 4],
    /// Button fill while hovered.
    pub button_hover: [f32; 4],
    /// Button fill while pressed.
    pub button_pressed: [f32; 4],
    /// Border stroke around buttons.
    pub button_border: [f32; 4],
    /// Text-input field fill.
    pub input_background: [f32; 4],
    /// Border stroke around an unfocused text input.
    pub input_border: [f32; 4],
    /// Border stroke around a focused text input.
    pub input_focus_border: [f32; 4],
    /// Primary body text color.
    pub text: [f32; 4],
    /// Dimmed/secondary text color (labels, placeholders).
    pub text_dim: [f32; 4],
    /// Emphasized/highlighted text color.
    pub text_highlight: [f32; 4],
    /// Accent color for primary/active UI elements.
    pub accent: [f32; 4],
    /// Lighter accent for small marks on dark surfaces: menu check ticks,
    /// radio dots (Forge `--accent-tick`).
    pub accent_tick: [f32; 4],
    /// Severity accents (see [`Severity`](crate::Severity)). The themeable palette
    /// behind banners/toasts; `error` doubles as the failure severity, so there's
    /// no separate severity-error field.
    pub info: [f32; 4],
    /// Success / confirmation severity accent.
    pub success: [f32; 4],
    /// Warning / caution severity accent.
    pub warning: [f32; 4],
    /// Failure severity accent (doubles as the general error color).
    pub error: [f32; 4],
    /// Outline color drawn around the keyboard-focused widget. Bright by design
    /// so the focus ring reads clearly against any widget chrome.
    pub focus_ring: [f32; 4],

    // Tab colors
    /// Fill of an inactive (unselected) tab.
    pub tab_inactive: [f32; 4],
    /// Fill of the active (selected) tab.
    pub tab_active: [f32; 4],
    /// Fill of a hovered tab.
    pub tab_hover: [f32; 4],
    /// Border stroke around tabs.
    pub tab_border: [f32; 4],

    // Progress bar colors
    /// Progress-bar track (unfilled) color.
    pub progress_background: [f32; 4],
    /// Progress-bar fill color for normal/healthy values.
    pub progress_fill: [f32; 4],
    /// Progress-bar fill color for low/critical values (e.g. hunger critical).
    pub progress_fill_low: [f32; 4], // For low values (e.g., hunger critical)
    /// Progress-bar fill color for medium values.
    pub progress_fill_medium: [f32; 4], // For medium values

    // Sizing
    /// Default inner padding, in pixels.
    pub padding: f32,
    /// Default gap between stacked elements, in pixels.
    pub spacing: f32,
    /// Default corner radius for rounded rects, in pixels.
    pub border_radius: f32,
    /// Default border stroke width, in pixels.
    pub border_width: f32,
    /// Default body text size, in pixels.
    pub font_size: f32,
    /// Title/heading text size, in pixels.
    pub font_size_title: f32,
    /// Default button height, in pixels.
    pub button_height: f32,
    /// Default text-input height, in pixels.
    pub input_height: f32,
    /// Height of the menu bar strip, in pixels (26px in the Forge design).
    /// Read by [`AppShell`](crate::AppShell) and [`MenuBar`](crate::MenuBar)
    /// via [`StyleKey::MenuBarHeight`](crate::StyleKey::MenuBarHeight).
    pub menu_bar_height: f32,
    /// Height of one menu-item row inside a dropdown sheet, in pixels. Read by
    /// [`MenuBar`](crate::MenuBar) via
    /// [`StyleKey::MenuRowHeight`](crate::StyleKey::MenuRowHeight).
    pub menu_row_height: f32,
    /// Floor for a menu column's width, in pixels — wide enough that a column of
    /// short labels still reads as a menu rather than a strip of text.
    pub menu_item_min_width: f32,
    /// Gap between a menu item's label and its accelerator hint, in pixels.
    pub menu_accel_gap: f32,
    /// Seconds a pointer must rest on a submenu parent before opening or replacing
    /// its child column.
    pub menu_hover_delay: f32,

    /// Side length of one toolbar tool button, in pixels. Read by
    /// [`Toolbar`](crate::Toolbar) via
    /// [`StyleKey::ToolbarButtonSize`](crate::StyleKey::ToolbarButtonSize).
    pub toolbar_button_size: f32,
    /// Edge padding inside the toolbar strip, in pixels.
    pub toolbar_padding: f32,
    /// Height of a dock panel's tab header row, in pixels.
    pub dock_tab_height: f32,
    /// Width of the resize splitter between a dock panel and the viewport.
    pub dock_splitter_width: f32,
    /// Height of a movable window's title strip, in pixels.
    pub window_title_height: f32,

    /// Hover/press transition duration in seconds. The default `0.0` switches
    /// colors instantly, as the Forge design does; a positive value opts into
    /// eased transitions. Read by widgets via
    /// [`StyleKey::AnimationDuration`](crate::StyleKey::AnimationDuration) so a
    /// [`StyleOverlay`](crate::StyleOverlay) can retune it per subtree.
    ///
    /// Scrolling is separate: [`ScrollView`](crate::ScrollView) glides by its
    /// [`ScrollState::smoothing`](crate::ScrollState::smoothing) whatever this
    /// is set to.
    pub animation_duration: f32,

    // --- 4a material tokens -----------------------------------------------
    //
    // The default design language paints every control from the same small
    // vocabulary: a *face* (vertical gradient, white sheen over the base tone —
    // or the accent/danger faces for stateful tones) resting on a dark *plinth*,
    // a 1px near-black border edge, and a 1px inset highlight under the top
    // edge. Pressing is geometric: the face drops by `travel` px onto the
    // plinth. Sunken surfaces (input wells, tracks) invert the model: dark fill,
    // inset shadow under the top edge, a faint light line beneath the bottom.
    //
    // All gradient colors are premultiplied-**style** white/black overlays:
    // faces carry their own alpha, so they composite correctly over any base.
    /// Plinth fill visible beneath/beside a raised face (the "shadow" a pressed
    /// face drops onto). Near-black translucent by default.
    pub plinth: [f32; 4],
    /// How far a face drops when pressed, in pixels — the plinth's visible
    /// travel. Doubles as the raised/sunken offset of face decorations.
    pub travel: f32,
    /// Depth in pixels of the inset "sunken" shadow that fades down from a
    /// well's top edge (the design's `inset 0 2px 4px` band).
    pub inner_shadow_depth: f32,
    /// Raised face gradient top (idle). White at low alpha.
    pub face_top: [f32; 4],
    /// Raised face gradient top (hovered).
    pub face_top_hover: [f32; 4],
    /// Raised face gradient top (pressed).
    pub face_top_pressed: [f32; 4],
    /// Raised face gradient bottom (idle).
    pub face_bottom: [f32; 4],
    /// Raised face gradient bottom (hovered).
    pub face_bottom_hover: [f32; 4],
    /// Raised face gradient bottom (pressed).
    pub face_bottom_pressed: [f32; 4],
    /// 1px highlight line under a raised face's top edge (idle).
    pub edge_highlight: [f32; 4],
    /// 1px highlight line under a raised face's top edge (hovered).
    pub edge_highlight_hover: [f32; 4],
    /// 1px highlight line under a raised face's top edge (pressed).
    pub edge_highlight_pressed: [f32; 4],
    /// Inset shadow gradient for sunken surfaces (wells, tracks): strongest at
    /// the top edge, fading down.
    pub inner_shadow: [f32; 4],
    /// 1px light line beneath a sunken surface's bottom edge.
    pub edge_shadow: [f32; 4],
    /// Accent-face gradient top (idle) — for primary/stateful controls.
    pub accent_face_top: [f32; 4],
    /// Accent-face gradient top (hovered).
    pub accent_face_top_hover: [f32; 4],
    /// Accent-face gradient top (pressed).
    pub accent_face_top_pressed: [f32; 4],
    /// Accent-face gradient bottom (idle).
    pub accent_face_bottom: [f32; 4],
    /// Accent-face gradient bottom (hovered).
    pub accent_face_bottom_hover: [f32; 4],
    /// Accent-face gradient bottom (pressed).
    pub accent_face_bottom_pressed: [f32; 4],
    /// Text/icon color drawn on top of accent faces.
    pub on_accent: [f32; 4],
    /// Danger-face gradient top (idle) — destructive actions.
    pub danger_face_top: [f32; 4],
    /// Danger-face gradient top (hovered).
    pub danger_face_top_hover: [f32; 4],
    /// Danger-face gradient top (pressed).
    pub danger_face_top_pressed: [f32; 4],
    /// Danger-face gradient bottom (idle).
    pub danger_face_bottom: [f32; 4],
    /// Danger-face gradient bottom (hovered).
    pub danger_face_bottom_hover: [f32; 4],
    /// Danger-face gradient bottom (pressed).
    pub danger_face_bottom_pressed: [f32; 4],
    /// Text/icon color drawn on top of danger faces.
    pub on_danger: [f32; 4],

    // --- Forge roles -------------------------------------------------------
    /// The ink ladder, indexed by [`Ink`](crate::Ink) (`Ink::ALL` order).
    /// Read through [`StyleKey::Ink`].
    pub ink: [[f32; 4]; crate::Ink::ALL.len()],
    /// Search-match highlight on a dark surface.
    pub accent_match: [f32; 4],
    /// Zebra stripe over every other list row.
    pub row_zebra: [f32; 4],
    /// Hovered list row fill.
    pub row_hover: [f32; 4],
    /// Selected row fill in a list without keyboard focus.
    pub row_held: [f32; 4],
    /// Deep sunken surface (empty-state boxes).
    pub well_deep: [f32; 4],
    /// Hard black edge around sunken boxes.
    pub edge_hard: [f32; 4],
    /// A healthy or running state (status dots, a connected light).
    pub status_ok: [f32; 4],
    /// A state waiting on the user (status dots, warning meta text).
    pub warn_meta: [f32; 4],
    /// Something new and unseen (unread dots, dirty tabs).
    pub accent_dirty: [f32; 4],
    /// Text of a destructive action (danger menu rows).
    pub danger_text: [f32; 4],
    /// Type scale in pixels, indexed by [`TextSize`](crate::TextSize).
    pub text_sizes: [f32; crate::TextSize::ALL.len()],
    /// Letter spacing in em, indexed by [`Tracking`](crate::Tracking).
    pub tracking: [f32; crate::Tracking::ALL.len()],
    /// Height of one list, tree or section header row, in pixels.
    pub list_row_height: f32,

    /// Finite, typed chrome materials for component families.
    pub chrome: ChromeTheme,

    /// UI-wide default font. `None` resolves to the default sans-serif (the
    /// bundled IBM Plex Sans when the `bundled-font` feature is on, else the system
    /// sans-serif). Set to a loaded [`FontHandle`] to theme all widget text in a
    /// custom family; every widget that builds text through this `Theme` picks it
    /// up. Per-block `TextBlock::with_font` still overrides it.
    pub font: Option<FontHandle>,

    /// Font for technical text: counts, metadata, shortcuts, mono-caps
    /// captions. Defaults to the bundled IBM Plex Mono (registered by
    /// [`shared_font_system`](crate::shared_font_system)); `None` (the default
    /// without the `bundled-font` feature) falls back to the sans font.
    pub mono_font: Option<FontHandle>,

    /// Mod-defined style values keyed by [`StyleKey::custom`] name-hash. Built-in
    /// styles live in the typed fields above; this map holds keys the core
    /// doesn't know about, so a custom widget can theme itself without core
    /// changes. Populate via [`Theme::register_style`]; read via [`Theme::style`]
    /// or the keyed [`Theme::get`].
    custom: CustomStyles,
}

impl Default for Theme {
    fn default() -> Self {
        // Sizing values the menu metrics are derived from. Naming them keeps the
        // derivation visible instead of duplicating literals: a theme that
        // retunes `padding`/`font_size` for DPI gets proportional menu rows.
        let padding = 5.0;
        let spacing = 6.0;
        let font_size = 12.0;
        Self {
            // The "4a" design language: near-black neutral surfaces, controls
            // painted as a subtle white-sheen gradient face resting on a dark
            // plinth, accent (teal, oklch hue 200) reserved for state.
            // Colours are sRGB-encoded, exactly as the design's CSS writes
            // them (see `crate::color`). The clear colour is #0a0d0f — all
            // opaque resolved values in the design spec are composited
            // against this backdrop.
            background: hex(0x0a0d0f),
            // Pause/menu screens dim the world by half. Black keeps the dim
            // neutral; apps wanting a color cast retune it per theme.
            scrim: [0.0, 0.0, 0.0, 0.55],
            // Raised surface = #16191d at the design's panel opacity.
            panel: rgba8([0x16, 0x19, 0x1d], 0.95),
            panel_border: [0.0, 0.0, 0.0, 0.55],
            // Resting face fill. The widgets paint raised controls as a gradient
            // from `ButtonHover`-strength sheen down to this tone; see
            // `ButtonTop`/`ButtonPressed` and the widget material helper.
            button: hex(0x1f2429),
            button_hover: hex(0x21262b),
            button_pressed: hex(0x14181c),
            button_border: [0.0, 0.0, 0.0, 0.5],
            input_background: [0.0, 0.0, 0.0, 0.42],
            input_border: [0.0, 0.0, 0.0, 0.6],
            input_focus_border: oklch(0.74, 0.11, 200.0, 1.0),
            // Design body text #eef2f5.
            text: hex(0xeef2f5),
            text_dim: hex(0xadb6bd),
            // Forge `--ink-max` — the brightest neutral, used for active
            // tab/tool labels.
            text_highlight: hex(0xf1f5f9),
            // Accent = teal oklch(0.74 0.11 200), state only (selection, focus,
            // toggle-on fills), never for resting chrome.
            accent: oklch(0.74, 0.11, 200.0, 1.0),
            accent_tick: oklch(0.8, 0.1, 200.0, 1.0),
            info: [0.3692, 0.6736, 0.92, 1.0],
            success: [0.3799, 0.7093, 0.3977, 1.0],
            // Forge `--warn-rule`.
            warning: oklch(0.78, 0.14, 75.0, 1.0),
            error: [0.8413, 0.2796, 0.2724, 1.0],
            focus_ring: oklch(0.74, 0.11, 200.0, 1.0),

            // Tab colors — the design's document/in-set tabs: inactive reads as
            // a sunken well, active as a raised face, border is the black edge.
            tab_inactive: [0.0, 0.0, 0.0, 0.22],
            tab_active: hex(0x21262b),
            tab_hover: [1.0, 1.0, 1.0, 0.07],
            tab_border: [0.0, 0.0, 0.0, 0.45],

            // Progress bar colors
            progress_background: [0.0, 0.0, 0.0, 0.55],
            // Progress/slider fills are the accent gradient.
            progress_fill: oklch(0.74, 0.11, 200.0, 1.0),
            progress_fill_low: [0.8413, 0.2796, 0.2724, 1.0],
            progress_fill_medium: [0.9084, 0.6684, 0.3042, 1.0],

            // Sizing — the design: 12px body text, 28px input wells, 26px
            // text buttons, 2px "plinth travel" press motion, radius 1.
            padding,
            spacing,
            border_radius: 1.0,
            border_width: 1.0,
            font_size,
            font_size_title: 15.0,
            button_height: 26.0,
            input_height: 28.0,
            // Forge menu geometry: 26px bar, 22px rows inside a 3px-padded,
            // 218px-minimum sheet. Title/row text uses its own 11.5px scale rather
            // than the 13px body font; see the menubar paint/measure paths.
            menu_bar_height: 26.0,
            menu_row_height: 22.0,
            menu_item_min_width: 218.0,
            menu_accel_gap: 7.0,
            menu_hover_delay: 0.20,
            toolbar_button_size: 24.0,
            toolbar_padding: 3.0,
            dock_tab_height: 24.0,
            dock_splitter_width: 6.0,
            window_title_height: 24.0,
            // Forge: state changes are instant (smooth scrolling is the one
            // motion, and it does not read this).
            animation_duration: 0.0,

            // 4a material tokens (see the field docs above).
            plinth: [0.0, 0.0, 0.0, 0.6],
            travel: 2.0,
            inner_shadow_depth: 6.0,
            face_top: [1.0, 1.0, 1.0, 0.16],
            face_top_hover: [1.0, 1.0, 1.0, 0.22],
            face_top_pressed: [1.0, 1.0, 1.0, 0.09],
            face_bottom: [1.0, 1.0, 1.0, 0.06],
            face_bottom_hover: [1.0, 1.0, 1.0, 0.10],
            face_bottom_pressed: [1.0, 1.0, 1.0, 0.035],
            edge_highlight: [1.0, 1.0, 1.0, 0.18],
            edge_highlight_hover: [1.0, 1.0, 1.0, 0.26],
            edge_highlight_pressed: [1.0, 1.0, 1.0, 0.07],
            inner_shadow: [0.0, 0.0, 0.0, 0.6],
            edge_shadow: [1.0, 1.0, 1.0, 0.07],
            // Forge `--accent-key-*` and `--danger-key-*`, spelled as the
            // design's `colors.css` writes them.
            accent_face_top: oklch(0.82, 0.1, 200.0, 1.0),
            accent_face_top_hover: oklch(0.86, 0.1, 200.0, 1.0),
            accent_face_top_pressed: oklch(0.7, 0.1, 200.0, 1.0),
            accent_face_bottom: oklch(0.68, 0.12, 200.0, 1.0),
            accent_face_bottom_hover: oklch(0.72, 0.12, 200.0, 1.0),
            accent_face_bottom_pressed: oklch(0.6, 0.11, 200.0, 1.0),
            on_accent: hex(0x04171d),
            danger_face_top: oklch(0.57, 0.17, 25.0, 1.0),
            danger_face_top_hover: oklch(0.62, 0.18, 25.0, 1.0),
            danger_face_top_pressed: oklch(0.5, 0.14, 25.0, 1.0),
            danger_face_bottom: oklch(0.48, 0.17, 25.0, 1.0),
            danger_face_bottom_hover: oklch(0.52, 0.18, 25.0, 1.0),
            danger_face_bottom_pressed: oklch(0.44, 0.14, 25.0, 1.0),
            on_danger: hex(0xfdeaea),

            // Forge `colors.css` ink ladder, in `Ink::ALL` order.
            ink: [
                hex(0xffffff), // White
                hex(0xf1f5f9), // Max
                hex(0xeef2f6), // Value
                hex(0xe2e8ee), // Emph
                hex(0xdbe1e7), // Menu
                hex(0xd5dce2), // Title
                hex(0xcfd6dd), // Row
                hex(0xc6ced5), // Cell
                hex(0xbdc5cc), // Second
                hex(0xb6bec5), // Icon
                hex(0xaeb6be), // Chip
                hex(0x9aa2aa), // Glyph
                hex(0x98a0a8), // Tab
                hex(0x8d959d), // TipHint
                hex(0x8b939b), // Body2
                hex(0x7d858e), // Muted
                hex(0x78818a), // Shortcut
                hex(0x737c85), // Label
                hex(0x6a737b), // Caption
                hex(0x626b73), // Dim
                hex(0x5d656c), // Disabled
                hex(0x4f575e), // DisabledGlyph
                hex(0x464e55), // DisabledKey
                hex(0x3f474e), // Empty
                hex(0x6f7982), // Brand
                hex(0xe6e9ec), // Primary
                hex(0x294d55), // OnAccentSecond
            ],
            accent_match: oklch(0.84, 0.09, 200.0, 1.0),
            row_zebra: [1.0, 1.0, 1.0, 0.016],
            row_hover: [1.0, 1.0, 1.0, 0.055],
            row_held: [1.0, 1.0, 1.0, 0.1],
            well_deep: [0.0, 0.0, 0.0, 0.4],
            edge_hard: [0.0, 0.0, 0.0, 0.65],
            status_ok: oklch(0.7, 0.14, 145.0, 1.0),
            warn_meta: oklch(0.82, 0.13, 75.0, 1.0),
            accent_dirty: oklch(0.78, 0.12, 200.0, 1.0),
            danger_text: oklch(0.72, 0.15, 25.0, 1.0),
            // `typography.css`: caption, meta, dense, row, menu.
            text_sizes: [9.0, 10.0, 10.5, 11.0, 11.5],
            // Badge, prop, caption, section, brand.
            tracking: [0.08, 0.1, 0.14, 0.16, 0.18],
            list_row_height: 22.0,

            chrome: ChromeTheme::default(),
            font: None,
            #[cfg(feature = "bundled-font")]
            mono_font: Some(FontHandle(crate::text::BUNDLED_MONO_FAMILY.to_string())),
            #[cfg(not(feature = "bundled-font"))]
            mono_font: None,
            custom: CustomStyles::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_text_and_title_have_no_font_by_default() {
        let theme = Theme::default();
        assert!(theme.text("hi", 0.0, 0.0).font.is_none());
        assert!(theme.title("hi", 0.0, 0.0).font.is_none());
    }

    #[test]
    fn default_typography_leaves_room_for_application_content() {
        let theme = Theme::default();
        // The Forge design uses 12px body text; 15px title still scales
        // visibly above it without doubling row heights.
        assert_eq!(theme.font_size, 12.0);
        assert_eq!(theme.font_size_title, 15.0);
        assert!(theme.font_size_title > theme.font_size);
    }

    #[test]
    fn default_css_palette_is_stored_as_plain_srgb() {
        let theme = Theme::default();
        // Stored exactly as the design's CSS writes them — no gamma decode.
        assert_eq!(
            theme.background,
            [10.0 / 255.0, 13.0 / 255.0, 15.0 / 255.0, 1.0]
        );
        assert_eq!(
            theme.panel,
            [22.0 / 255.0, 25.0 / 255.0, 29.0 / 255.0, 0.95]
        );
        assert_eq!(
            theme.text,
            [238.0 / 255.0, 242.0 / 255.0, 245.0 / 255.0, 1.0]
        );
    }

    #[test]
    fn default_palette_uses_the_forge_tokens() {
        use crate::color::oklch;
        let t = Theme::default();
        let accent = oklch(0.74, 0.11, 200.0, 1.0);
        assert_eq!(crate::color::to_rgba8(accent), [0x3e, 0xbf, 0xc6, 0xff]);
        assert_eq!(t.accent, accent);
        assert_eq!(t.accent_tick, oklch(0.8, 0.1, 200.0, 1.0));
        assert_eq!(t.accent_face_top_hover, oklch(0.86, 0.1, 200.0, 1.0));
        assert_eq!(t.danger_face_top_pressed, oklch(0.5, 0.14, 25.0, 1.0));
        assert_eq!(t.danger_face_bottom_pressed, oklch(0.44, 0.14, 25.0, 1.0));
        assert_eq!(t.warning, oklch(0.78, 0.14, 75.0, 1.0));
        assert_eq!(t.text_highlight, hex(0xf1f5f9), "--ink-max");
        // --key-face-hover and the key-inset edges.
        assert_eq!((t.face_top_hover[3], t.face_bottom_hover[3]), (0.22, 0.10));
        assert_eq!(t.edge_highlight_hover[3], 0.26);
        assert_eq!(t.edge_highlight_pressed[3], 0.07);

        let splitter = t.chrome.splitter;
        assert_eq!(splitter.grip_dragging, oklch(0.82, 0.1, 200.0, 1.0));
        assert_eq!(splitter.dragging_glow.color, oklch(0.74, 0.11, 200.0, 0.65));
    }

    #[test]
    fn default_menu_metrics_match_the_forge_handoff() {
        let theme = Theme::default();
        assert_eq!(theme.menu_bar_height, 26.0);
        assert_eq!(theme.menu_row_height, 22.0);
        assert_eq!(theme.menu_item_min_width, 218.0);
        assert_eq!(theme.menu_accel_gap, 7.0);
        assert_eq!(theme.menu_hover_delay, 0.20);
    }

    #[test]
    fn builtin_keys_round_trip_through_get_set() {
        use crate::style::{COLOR_KEYS, SCALAR_KEYS};
        let mut theme = Theme::default();
        // Every color key reads back the field, and set writes it.
        for &k in COLOR_KEYS {
            let orig = theme.get(k).unwrap().as_color().unwrap();
            let probe = [orig[0] * 0.5 + 0.1, 0.2, 0.3, 1.0];
            theme.set(k, StyleValue::Color(probe));
            assert_eq!(theme.get(k).unwrap().as_color().unwrap(), probe, "{k:?}");
        }
        let roles = crate::Ink::ALL.map(StyleKey::Ink);
        for k in roles {
            let probe = [0.1, 0.2, 0.3, 1.0];
            theme.set(k, StyleValue::Color(probe));
            assert_eq!(theme.get(k).unwrap().as_color().unwrap(), probe, "{k:?}");
            assert!(k.is_color(), "{k:?}");
        }
        let sizes = crate::TextSize::ALL.map(StyleKey::TextSize);
        let tracking = crate::Tracking::ALL.map(StyleKey::Tracking);
        for &k in SCALAR_KEYS.iter().chain(&sizes).chain(&tracking) {
            theme.set(k, StyleValue::Scalar(123.5));
            assert_eq!(theme.get(k).unwrap().as_scalar().unwrap(), 123.5, "{k:?}");
            assert!(!k.is_color(), "{k:?}");
        }
    }

    #[test]
    fn forge_roles_use_the_design_values() {
        use crate::{Ink, TextSize, Tracking};
        let t = Theme::default();
        let ink = |role| t.get(StyleKey::Ink(role)).unwrap().as_color().unwrap();
        assert_eq!(ink(Ink::Max), t.text_highlight, "--ink-max");
        assert_eq!(ink(Ink::Glyph), hex(0x9aa2aa));
        assert_eq!(ink(Ink::Caption), hex(0x6a737b));
        assert_eq!(ink(Ink::Empty), hex(0x3f474e));
        assert_eq!(ink(Ink::OnAccentSecond), hex(0x294d55));
        assert_eq!(t.accent_match, oklch(0.84, 0.09, 200.0, 1.0));
        assert_eq!(t.row_zebra, [1.0, 1.0, 1.0, 0.016]);
        let size = |s| t.get(StyleKey::TextSize(s)).unwrap().as_scalar().unwrap();
        assert_eq!(size(TextSize::Caption), 9.0);
        assert_eq!(size(TextSize::Menu), 11.5);
        let track = |r| t.get(StyleKey::Tracking(r)).unwrap().as_scalar().unwrap();
        assert_eq!(track(Tracking::Section), 0.16);
        assert_eq!(t.list_row_height, 22.0);
    }

    #[test]
    fn get_set_match_typed_field_accessor() {
        let mut theme = Theme::default();
        theme.set(StyleKey::Accent, StyleValue::Color([0.1, 0.2, 0.3, 1.0]));
        assert_eq!(
            theme.accent,
            [0.1, 0.2, 0.3, 1.0],
            "set writes the typed field"
        );
        theme.button = [0.4, 0.5, 0.6, 1.0];
        assert_eq!(
            theme.get(StyleKey::Button).unwrap().as_color().unwrap(),
            [0.4, 0.5, 0.6, 1.0],
            "get reads the typed field"
        );
    }

    #[test]
    fn register_and_read_custom_style() {
        let mut theme = Theme::default();
        assert_eq!(theme.style("mywidget.glow"), None);
        theme.register_style("mywidget.glow", StyleValue::Color([1.0, 0.5, 0.0, 1.0]));
        assert_eq!(
            theme.style("mywidget.glow"),
            Some(StyleValue::Color([1.0, 0.5, 0.0, 1.0]))
        );
        // Reachable via the keyed get with the same name.
        assert_eq!(
            theme.get(StyleKey::custom("mywidget.glow")),
            Some(StyleValue::Color([1.0, 0.5, 0.0, 1.0]))
        );
    }

    #[test]
    fn animation_duration_default_and_keyed_access() {
        let theme = Theme::default();
        assert_eq!(theme.animation_duration, 0.0, "Forge: no fades");
        assert_eq!(
            theme
                .get(StyleKey::AnimationDuration)
                .unwrap()
                .as_scalar()
                .unwrap(),
            0.0
        );
    }

    #[test]
    fn theme_font_applies_to_text_and_title() {
        let mut theme = Theme::default();
        theme.font = Some(FontHandle("IBM Plex Sans".to_string()));
        assert_eq!(
            theme.text("hi", 0.0, 0.0).font.as_ref().unwrap().family(),
            "IBM Plex Sans"
        );
        assert_eq!(
            theme.title("hi", 0.0, 0.0).font.as_ref().unwrap().family(),
            "IBM Plex Sans"
        );
    }
}

impl Theme {
    /// Resolve a [`StyleKey`] to its value: built-in keys read the typed field,
    /// `Custom` keys read the [`register_style`](Self::register_style) map
    /// (`None` if unset). This is the keyed view of the theme that the
    /// [`StyleResolver`](crate::StyleResolver) reads through.
    pub fn get(&self, key: StyleKey) -> Option<StyleValue> {
        use StyleKey::*;
        let v = match key {
            // Colors
            Background => StyleValue::Color(self.background),
            Scrim => StyleValue::Color(self.scrim),
            Panel => StyleValue::Color(self.panel),
            PanelBorder => StyleValue::Color(self.panel_border),
            Button => StyleValue::Color(self.button),
            ButtonHover => StyleValue::Color(self.button_hover),
            ButtonPressed => StyleValue::Color(self.button_pressed),
            ButtonBorder => StyleValue::Color(self.button_border),
            InputBackground => StyleValue::Color(self.input_background),
            InputBorder => StyleValue::Color(self.input_border),
            InputFocusBorder => StyleValue::Color(self.input_focus_border),
            Text => StyleValue::Color(self.text),
            TextDim => StyleValue::Color(self.text_dim),
            TextHighlight => StyleValue::Color(self.text_highlight),
            Accent => StyleValue::Color(self.accent),
            AccentTick => StyleValue::Color(self.accent_tick),
            Info => StyleValue::Color(self.info),
            Success => StyleValue::Color(self.success),
            Warning => StyleValue::Color(self.warning),
            Error => StyleValue::Color(self.error),
            FocusRing => StyleValue::Color(self.focus_ring),
            TabInactive => StyleValue::Color(self.tab_inactive),
            TabActive => StyleValue::Color(self.tab_active),
            TabHover => StyleValue::Color(self.tab_hover),
            TabBorder => StyleValue::Color(self.tab_border),
            ProgressBackground => StyleValue::Color(self.progress_background),
            ProgressFill => StyleValue::Color(self.progress_fill),
            ProgressFillLow => StyleValue::Color(self.progress_fill_low),
            ProgressFillMedium => StyleValue::Color(self.progress_fill_medium),
            // 4a materials
            Plinth => StyleValue::Color(self.plinth),
            FaceTop => StyleValue::Color(self.face_top),
            FaceTopHover => StyleValue::Color(self.face_top_hover),
            FaceTopPressed => StyleValue::Color(self.face_top_pressed),
            FaceBottom => StyleValue::Color(self.face_bottom),
            FaceBottomHover => StyleValue::Color(self.face_bottom_hover),
            FaceBottomPressed => StyleValue::Color(self.face_bottom_pressed),
            EdgeHighlight => StyleValue::Color(self.edge_highlight),
            EdgeHighlightHover => StyleValue::Color(self.edge_highlight_hover),
            EdgeHighlightPressed => StyleValue::Color(self.edge_highlight_pressed),
            InnerShadow => StyleValue::Color(self.inner_shadow),
            EdgeShadow => StyleValue::Color(self.edge_shadow),
            AccentFaceTop => StyleValue::Color(self.accent_face_top),
            AccentFaceTopHover => StyleValue::Color(self.accent_face_top_hover),
            AccentFaceTopPressed => StyleValue::Color(self.accent_face_top_pressed),
            AccentFaceBottom => StyleValue::Color(self.accent_face_bottom),
            AccentFaceBottomHover => StyleValue::Color(self.accent_face_bottom_hover),
            AccentFaceBottomPressed => StyleValue::Color(self.accent_face_bottom_pressed),
            OnAccent => StyleValue::Color(self.on_accent),
            DangerFaceTop => StyleValue::Color(self.danger_face_top),
            DangerFaceTopHover => StyleValue::Color(self.danger_face_top_hover),
            DangerFaceTopPressed => StyleValue::Color(self.danger_face_top_pressed),
            DangerFaceBottom => StyleValue::Color(self.danger_face_bottom),
            DangerFaceBottomHover => StyleValue::Color(self.danger_face_bottom_hover),
            DangerFaceBottomPressed => StyleValue::Color(self.danger_face_bottom_pressed),
            OnDanger => StyleValue::Color(self.on_danger),
            // Scalars
            Padding => StyleValue::Scalar(self.padding),
            Spacing => StyleValue::Scalar(self.spacing),
            BorderRadius => StyleValue::Scalar(self.border_radius),
            BorderWidth => StyleValue::Scalar(self.border_width),
            FontSize => StyleValue::Scalar(self.font_size),
            FontSizeTitle => StyleValue::Scalar(self.font_size_title),
            ButtonHeight => StyleValue::Scalar(self.button_height),
            InputHeight => StyleValue::Scalar(self.input_height),
            AnimationDuration => StyleValue::Scalar(self.animation_duration),
            MenuBarHeight => StyleValue::Scalar(self.menu_bar_height),
            MenuRowHeight => StyleValue::Scalar(self.menu_row_height),
            MenuItemMinWidth => StyleValue::Scalar(self.menu_item_min_width),
            MenuAccelGap => StyleValue::Scalar(self.menu_accel_gap),
            MenuHoverDelay => StyleValue::Scalar(self.menu_hover_delay),
            ToolbarButtonSize => StyleValue::Scalar(self.toolbar_button_size),
            ToolbarPadding => StyleValue::Scalar(self.toolbar_padding),
            DockTabHeight => StyleValue::Scalar(self.dock_tab_height),
            DockSplitterWidth => StyleValue::Scalar(self.dock_splitter_width),
            WindowTitleHeight => StyleValue::Scalar(self.window_title_height),
            Travel => StyleValue::Scalar(self.travel),
            InnerShadowDepth => StyleValue::Scalar(self.inner_shadow_depth),
            // Forge roles
            Ink(role) => StyleValue::Color(self.ink[role as usize]),
            AccentMatch => StyleValue::Color(self.accent_match),
            RowZebra => StyleValue::Color(self.row_zebra),
            RowHover => StyleValue::Color(self.row_hover),
            RowHeld => StyleValue::Color(self.row_held),
            WellDeep => StyleValue::Color(self.well_deep),
            EdgeHard => StyleValue::Color(self.edge_hard),
            StatusOk => StyleValue::Color(self.status_ok),
            WarnMeta => StyleValue::Color(self.warn_meta),
            AccentDirty => StyleValue::Color(self.accent_dirty),
            DangerText => StyleValue::Color(self.danger_text),
            TextSize(step) => StyleValue::Scalar(self.text_sizes[step as usize]),
            Tracking(role) => StyleValue::Scalar(self.tracking[role as usize]),
            ListRowHeight => StyleValue::Scalar(self.list_row_height),
            // Custom namespace
            Custom(id) => return self.custom.get(&id).copied(),
        };
        Some(v)
    }

    /// Set a [`StyleKey`]'s value. Built-in keys write the typed field; a
    /// shape mismatch (e.g. a [`StyleValue::Scalar`] into a color field) is a
    /// `debug_assert` failure and a no-op in release. `Custom` keys write the
    /// custom map regardless of shape.
    pub fn set(&mut self, key: StyleKey, value: StyleValue) {
        use StyleKey::*;
        // Custom keys store whatever shape they're given.
        if let Custom(id) = key {
            self.custom.insert(id, value);
            return;
        }
        match (key, value) {
            (Background, StyleValue::Color(c)) => self.background = c,
            (Scrim, StyleValue::Color(c)) => self.scrim = c,
            (Panel, StyleValue::Color(c)) => self.panel = c,
            (PanelBorder, StyleValue::Color(c)) => self.panel_border = c,
            (Button, StyleValue::Color(c)) => self.button = c,
            (ButtonHover, StyleValue::Color(c)) => self.button_hover = c,
            (ButtonPressed, StyleValue::Color(c)) => self.button_pressed = c,
            (ButtonBorder, StyleValue::Color(c)) => self.button_border = c,
            (InputBackground, StyleValue::Color(c)) => self.input_background = c,
            (InputBorder, StyleValue::Color(c)) => self.input_border = c,
            (InputFocusBorder, StyleValue::Color(c)) => self.input_focus_border = c,
            (Text, StyleValue::Color(c)) => self.text = c,
            (TextDim, StyleValue::Color(c)) => self.text_dim = c,
            (TextHighlight, StyleValue::Color(c)) => self.text_highlight = c,
            (Accent, StyleValue::Color(c)) => self.accent = c,
            (AccentTick, StyleValue::Color(c)) => self.accent_tick = c,
            (Info, StyleValue::Color(c)) => self.info = c,
            (Success, StyleValue::Color(c)) => self.success = c,
            (Warning, StyleValue::Color(c)) => self.warning = c,
            (Error, StyleValue::Color(c)) => self.error = c,
            (FocusRing, StyleValue::Color(c)) => self.focus_ring = c,
            (TabInactive, StyleValue::Color(c)) => self.tab_inactive = c,
            (TabActive, StyleValue::Color(c)) => self.tab_active = c,
            (TabHover, StyleValue::Color(c)) => self.tab_hover = c,
            (TabBorder, StyleValue::Color(c)) => self.tab_border = c,
            (ProgressBackground, StyleValue::Color(c)) => self.progress_background = c,
            (ProgressFill, StyleValue::Color(c)) => self.progress_fill = c,
            (ProgressFillLow, StyleValue::Color(c)) => self.progress_fill_low = c,
            (ProgressFillMedium, StyleValue::Color(c)) => self.progress_fill_medium = c,
            // 4a materials
            (Plinth, StyleValue::Color(c)) => self.plinth = c,
            (FaceTop, StyleValue::Color(c)) => self.face_top = c,
            (FaceTopHover, StyleValue::Color(c)) => self.face_top_hover = c,
            (FaceTopPressed, StyleValue::Color(c)) => self.face_top_pressed = c,
            (FaceBottom, StyleValue::Color(c)) => self.face_bottom = c,
            (FaceBottomHover, StyleValue::Color(c)) => self.face_bottom_hover = c,
            (FaceBottomPressed, StyleValue::Color(c)) => self.face_bottom_pressed = c,
            (EdgeHighlight, StyleValue::Color(c)) => self.edge_highlight = c,
            (EdgeHighlightHover, StyleValue::Color(c)) => self.edge_highlight_hover = c,
            (EdgeHighlightPressed, StyleValue::Color(c)) => self.edge_highlight_pressed = c,
            (InnerShadow, StyleValue::Color(c)) => self.inner_shadow = c,
            (EdgeShadow, StyleValue::Color(c)) => self.edge_shadow = c,
            (AccentFaceTop, StyleValue::Color(c)) => self.accent_face_top = c,
            (AccentFaceTopHover, StyleValue::Color(c)) => self.accent_face_top_hover = c,
            (AccentFaceTopPressed, StyleValue::Color(c)) => self.accent_face_top_pressed = c,
            (AccentFaceBottom, StyleValue::Color(c)) => self.accent_face_bottom = c,
            (AccentFaceBottomHover, StyleValue::Color(c)) => self.accent_face_bottom_hover = c,
            (AccentFaceBottomPressed, StyleValue::Color(c)) => self.accent_face_bottom_pressed = c,
            (OnAccent, StyleValue::Color(c)) => self.on_accent = c,
            (DangerFaceTop, StyleValue::Color(c)) => self.danger_face_top = c,
            (DangerFaceTopHover, StyleValue::Color(c)) => self.danger_face_top_hover = c,
            (DangerFaceTopPressed, StyleValue::Color(c)) => self.danger_face_top_pressed = c,
            (DangerFaceBottom, StyleValue::Color(c)) => self.danger_face_bottom = c,
            (DangerFaceBottomHover, StyleValue::Color(c)) => self.danger_face_bottom_hover = c,
            (DangerFaceBottomPressed, StyleValue::Color(c)) => self.danger_face_bottom_pressed = c,
            (OnDanger, StyleValue::Color(c)) => self.on_danger = c,
            (Padding, StyleValue::Scalar(s)) => self.padding = s,
            (Spacing, StyleValue::Scalar(s)) => self.spacing = s,
            (BorderRadius, StyleValue::Scalar(s)) => self.border_radius = s,
            (BorderWidth, StyleValue::Scalar(s)) => self.border_width = s,
            (FontSize, StyleValue::Scalar(s)) => self.font_size = s,
            (FontSizeTitle, StyleValue::Scalar(s)) => self.font_size_title = s,
            (ButtonHeight, StyleValue::Scalar(s)) => self.button_height = s,
            (InputHeight, StyleValue::Scalar(s)) => self.input_height = s,
            (AnimationDuration, StyleValue::Scalar(s)) => self.animation_duration = s,
            (MenuBarHeight, StyleValue::Scalar(s)) => self.menu_bar_height = s,
            (MenuRowHeight, StyleValue::Scalar(s)) => self.menu_row_height = s,
            (MenuItemMinWidth, StyleValue::Scalar(s)) => self.menu_item_min_width = s,
            (MenuAccelGap, StyleValue::Scalar(s)) => self.menu_accel_gap = s,
            (MenuHoverDelay, StyleValue::Scalar(s)) => self.menu_hover_delay = s,
            (ToolbarButtonSize, StyleValue::Scalar(s)) => self.toolbar_button_size = s,
            (ToolbarPadding, StyleValue::Scalar(s)) => self.toolbar_padding = s,
            (DockTabHeight, StyleValue::Scalar(s)) => self.dock_tab_height = s,
            (DockSplitterWidth, StyleValue::Scalar(s)) => self.dock_splitter_width = s,
            (WindowTitleHeight, StyleValue::Scalar(s)) => self.window_title_height = s,
            (Travel, StyleValue::Scalar(s)) => self.travel = s,
            (InnerShadowDepth, StyleValue::Scalar(s)) => self.inner_shadow_depth = s,
            (Ink(role), StyleValue::Color(c)) => self.ink[role as usize] = c,
            (AccentMatch, StyleValue::Color(c)) => self.accent_match = c,
            (RowZebra, StyleValue::Color(c)) => self.row_zebra = c,
            (RowHover, StyleValue::Color(c)) => self.row_hover = c,
            (RowHeld, StyleValue::Color(c)) => self.row_held = c,
            (WellDeep, StyleValue::Color(c)) => self.well_deep = c,
            (EdgeHard, StyleValue::Color(c)) => self.edge_hard = c,
            (StatusOk, StyleValue::Color(c)) => self.status_ok = c,
            (WarnMeta, StyleValue::Color(c)) => self.warn_meta = c,
            (AccentDirty, StyleValue::Color(c)) => self.accent_dirty = c,
            (DangerText, StyleValue::Color(c)) => self.danger_text = c,
            (TextSize(step), StyleValue::Scalar(s)) => self.text_sizes[step as usize] = s,
            (Tracking(role), StyleValue::Scalar(s)) => self.tracking[role as usize] = s,
            (ListRowHeight, StyleValue::Scalar(s)) => self.list_row_height = s,
            (k, v) => debug_assert!(
                false,
                "Theme::set shape mismatch for {k:?}: built-in key got {v:?}"
            ),
        }
    }

    /// Register a mod-defined style value under `name` (Teardown-style
    /// `register_style`). Sugar for `set(StyleKey::custom(name), value)`; read it
    /// back with [`style`](Self::style) or `get(StyleKey::custom(name))`.
    pub fn register_style(&mut self, name: &str, value: StyleValue) {
        self.set(StyleKey::custom(name), value);
    }

    /// Read a mod-defined style value by `name` (`None` if never registered).
    pub fn style(&self, name: &str) -> Option<StyleValue> {
        self.get(StyleKey::custom(name))
    }

    /// Create a text block with theme styling
    pub fn text(&self, content: impl Into<String>, x: f32, y: f32) -> TextBlock {
        TextBlock::new(content, x, y)
            .with_size(self.font_size)
            .with_color_f32(self.text)
            .with_font_opt(self.font.clone())
    }

    /// Create a title text block
    pub fn title(&self, content: impl Into<String>, x: f32, y: f32) -> TextBlock {
        TextBlock::new(content, x, y)
            .with_size(self.font_size_title)
            .with_color_f32(self.text)
            .with_font_opt(self.font.clone())
    }
}
