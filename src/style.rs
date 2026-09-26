//! Typed, extensible styling: keyed style values, scoped overrides, and the
//! resolver widgets read through.
//!
//! The flat [`Theme`](crate::Theme) struct stays the source of built-in values;
//! this module adds three things on top of it:
//!
//! - [`StyleKey`] / [`StyleValue`] — a typed address space over every theme
//!   field plus a [`StyleKey::Custom`] namespace for mod-defined keys (so a
//!   custom widget can carry its own style without core changes).
//! - [`StyleOverlay`] — a caller-owned sparse set of overrides.
//! - [`StyleResolver`] — the single read path: overlay first, then theme. A
//!   scoped overlay thus recolors everything drawn under it **without cloning
//!   the theme**.

use std::collections::HashMap;

use crate::Theme;
use crate::text::TextBlock;

/// A single resolved style datum — either a color or a scalar.
///
/// Theme values are one of these two shapes (`[f32; 4]` RGBA colors or `f32`
/// sizes), so the keyed map is uniform without boxing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StyleValue {
    /// An RGBA color.
    Color([f32; 4]),
    /// A scalar size/length.
    Scalar(f32),
}

impl StyleValue {
    /// The color, if this is a [`StyleValue::Color`].
    pub fn as_color(self) -> Option<[f32; 4]> {
        match self {
            StyleValue::Color(c) => Some(c),
            StyleValue::Scalar(_) => None,
        }
    }

    /// The scalar, if this is a [`StyleValue::Scalar`].
    pub fn as_scalar(self) -> Option<f32> {
        match self {
            StyleValue::Scalar(s) => Some(s),
            StyleValue::Color(_) => None,
        }
    }
}

/// 64-bit FNV-1a hash of `name`. Used to address [`StyleKey::Custom`] keys by
/// name without a global interner: the hash is a pure function, stable across
/// themes and runs, so `StyleKey::custom("x")` always denotes the same key.
pub(crate) const fn fnv1a64(name: &str) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let bytes = name.as_bytes();
    let mut hash = OFFSET;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(PRIME);
        i += 1;
    }
    hash
}

/// A typed address for a style value: one variant per built-in [`Theme`] field,
/// plus [`StyleKey::Custom`] for mod-defined keys.
///
/// Colors and scalars share the enum; [`StyleResolver::color`] /
/// [`StyleResolver::scalar`] pick the matching shape (a built-in key always
/// resolves to its declared shape).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StyleKey {
    // --- Colors ---
    /// Window/screen backdrop fill.
    Background,
    /// Fullscreen dim drawn behind a modal, menu screen, or pause overlay —
    /// the translucent darkening layer that pushes the scene back. Read by
    /// [`draw_scrim`](crate::draw_scrim); a good value is black at ~0.5 alpha.
    Scrim,
    /// Panel/container surface fill.
    Panel,
    /// Border stroke around panels/containers.
    PanelBorder,
    /// Button fill in its resting state.
    Button,
    /// Button fill while hovered.
    ButtonHover,
    /// Button fill while pressed.
    ButtonPressed,
    /// Border stroke around buttons.
    ButtonBorder,
    /// Text-input field fill.
    InputBackground,
    /// Border stroke around an unfocused text input.
    InputBorder,
    /// Border stroke around a focused text input.
    InputFocusBorder,
    /// Primary body text color.
    Text,
    /// Dimmed/secondary text color.
    TextDim,
    /// Emphasized/highlighted text color.
    TextHighlight,
    /// Accent color for primary/active elements.
    Accent,
    /// Lighter accent for small marks on dark surfaces (menu ticks, radio
    /// dots).
    AccentTick,
    /// Informational severity accent (see [`Severity`](crate::Severity)). Themeable
    /// palette entry; the severity→key mapping is the only fixed policy.
    Info,
    /// Success / confirmation severity accent.
    Success,
    /// Warning / caution severity accent.
    Warning,
    /// Failure severity accent (shared with the general error color).
    Error,
    /// Outline color of the keyboard-focus ring.
    FocusRing,
    /// Fill of an inactive (unselected) tab.
    TabInactive,
    /// Fill of the active (selected) tab.
    TabActive,
    /// Fill of a hovered tab.
    TabHover,
    /// Border stroke around tabs.
    TabBorder,
    /// Progress-bar track (unfilled) color.
    ProgressBackground,
    /// Progress-bar fill color for normal/healthy values.
    ProgressFill,
    /// Progress-bar fill color for low/critical values.
    ProgressFillLow,
    /// Progress-bar fill color for medium values.
    ProgressFillMedium,
    // --- Scalars ---
    /// Default inner padding, in pixels.
    Padding,
    /// Default gap between stacked elements, in pixels.
    Spacing,
    /// Default corner radius, in pixels.
    BorderRadius,
    /// Default border stroke width, in pixels.
    BorderWidth,
    /// Default body text size, in pixels.
    FontSize,
    /// Title/heading text size, in pixels.
    FontSizeTitle,
    /// Default button height, in pixels.
    ButtonHeight,
    /// Default text-input height, in pixels.
    InputHeight,
    /// Hover/press transition duration in seconds (see
    /// [`AnimationState`](crate::AnimationState)). `0.0` disables animation
    /// (instant color switches).
    AnimationDuration,
    /// Height of the menu bar strip, in pixels (26px in the Forge design).
    MenuBarHeight,
    /// Height of one menu-item row inside a dropdown sheet, in pixels.
    MenuRowHeight,
    /// Floor for a menu column's width, in pixels.
    MenuItemMinWidth,
    /// Gap between a menu item's label and its accelerator hint, in pixels.
    MenuAccelGap,
    /// Seconds a pointer must rest on a submenu parent before it opens or replaces
    /// the currently-open child column.
    MenuHoverDelay,
    // --- 4a material keys (colors) ---
    /// Plinth fill beneath a raised face (see [`Theme::plinth`]).
    Plinth,
    /// Raised face gradient top, idle state (see [`Theme::face_top`]).
    FaceTop,
    /// Raised face gradient top, hovered.
    FaceTopHover,
    /// Raised face gradient top, pressed.
    FaceTopPressed,
    /// Raised face gradient bottom, idle state (see [`Theme::face_bottom`]).
    FaceBottom,
    /// Raised face gradient bottom, hovered.
    FaceBottomHover,
    /// Raised face gradient bottom, pressed.
    FaceBottomPressed,
    /// Top-edge highlight line on a raised face, idle.
    EdgeHighlight,
    /// Top-edge highlight line on a raised face, hovered.
    EdgeHighlightHover,
    /// Top-edge highlight line on a raised face, pressed.
    EdgeHighlightPressed,
    /// Inset shadow color for sunken surfaces (wells, tracks).
    InnerShadow,
    /// Bottom-edge light line under a sunken surface.
    EdgeShadow,
    /// Accent-face gradient top, idle (primary controls).
    AccentFaceTop,
    /// Accent-face gradient top, hovered.
    AccentFaceTopHover,
    /// Accent-face gradient top, pressed.
    AccentFaceTopPressed,
    /// Accent-face gradient bottom, idle.
    AccentFaceBottom,
    /// Accent-face gradient bottom, hovered.
    AccentFaceBottomHover,
    /// Accent-face gradient bottom, pressed.
    AccentFaceBottomPressed,
    /// Text/icon color on accent faces.
    OnAccent,
    /// Danger-face gradient top, idle (destructive controls).
    DangerFaceTop,
    /// Danger-face gradient top, hovered.
    DangerFaceTopHover,
    /// Danger-face gradient top, pressed.
    DangerFaceTopPressed,
    /// Danger-face gradient bottom, idle.
    DangerFaceBottom,
    /// Danger-face gradient bottom, hovered.
    DangerFaceBottomHover,
    /// Danger-face gradient bottom, pressed.
    DangerFaceBottomPressed,
    /// Text/icon color on danger faces.
    OnDanger,
    // --- 4a material keys (scalars) ---
    /// Press travel: how far a face drops when pressed, in pixels.
    Travel,
    /// Depth of the inset "sunken" shadow fading down from a well's top edge,
    /// in pixels (the design's `inset 0 2px 4px` band).
    InnerShadowDepth,
    // --- Toolbar / dock / shell scalars ---
    /// Side length of one toolbar tool button, in pixels.
    ToolbarButtonSize,
    /// Edge padding inside the toolbar strip, in pixels.
    ToolbarPadding,
    /// Height of a dock panel's tab header row, in pixels.
    DockTabHeight,
    /// Width (or height, for horizontal) of the resize splitter between a dock
    /// panel and the viewport, in pixels.
    DockSplitterWidth,
    /// Height of a movable [`Window`](crate::Window)'s title strip, in pixels.
    WindowTitleHeight,
    // --- Forge roles (colors) ---
    /// One step of Forge's ink ladder: the closed set of text and glyph
    /// colours (see [`Ink`]).
    Ink(Ink),
    /// Search-match highlight on a dark surface (Forge `--accent-match`).
    AccentMatch,
    /// Every other row of a list, over its background (`--row-zebra`).
    RowZebra,
    /// A hovered list row (`--row-hover`).
    RowHover,
    /// A selected row in a list that doesn't have the keyboard: a neutral
    /// held fill, so the focused list's accent selection stands out.
    RowHeld,
    /// Deep sunken surface, e.g. an empty-state box (`--well-deep`).
    WellDeep,
    /// Hard black edge around sunken boxes (`--edge-hard`).
    EdgeHard,
    /// A healthy or running state: a status dot, a connected light (`--ok`).
    StatusOk,
    /// A state waiting on the user, as a dot or meta text (`--warn-meta`).
    WarnMeta,
    /// Something new and unseen: an unread dot, a dirty tab (`--accent-dirty`).
    AccentDirty,
    /// A handle being dragged: a scrubbed property label, a splitter grip
    /// (`--accent-grip`).
    AccentGrip,
    /// A linked file's glyph (`--accent-glyph`).
    AccentGlyph,
    /// Text of a destructive action, such as a danger menu row
    /// (`--danger-text`).
    DangerText,
    // --- Forge roles (scalars) ---
    /// One step of Forge's type scale, in pixels (see [`TextSize`]).
    TextSize(TextSize),
    /// Letter spacing for a text role, in em (see [`Tracking`]).
    Tracking(Tracking),
    /// Height of one list, tree or dock-section header row, in pixels
    /// (`--h-list-row`).
    ListRowHeight,
    /// A mod-defined key, addressed by the FNV-1a hash of its name (see
    /// [`StyleKey::custom`]). Lives in [`Theme`]'s custom map / a [`StyleOverlay`].
    Custom(u64),
}

impl StyleKey {
    /// A custom key addressed by `name`. Equal names always produce the same
    /// key; distinct names (barring an astronomically unlikely 64-bit collision)
    /// produce distinct keys. No registration or global interner required.
    pub fn custom(name: &str) -> Self {
        StyleKey::Custom(fnv1a64(name))
    }

    /// Whether this key denotes a color (vs a scalar). Built-in keys have a
    /// fixed shape; `Custom` keys are shapeless (whatever value was stored), so
    /// this returns `false` for them.
    pub fn is_color(self) -> bool {
        use StyleKey::*;
        matches!(
            self,
            Background
                | Panel
                | PanelBorder
                | Button
                | ButtonHover
                | ButtonPressed
                | ButtonBorder
                | InputBackground
                | InputBorder
                | InputFocusBorder
                | Text
                | TextDim
                | TextHighlight
                | Accent
                | AccentTick
                | Info
                | Success
                | Warning
                | Error
                | FocusRing
                | TabInactive
                | TabActive
                | TabHover
                | TabBorder
                | ProgressBackground
                | ProgressFill
                | ProgressFillLow
                | ProgressFillMedium
                | Plinth
                | FaceTop
                | FaceTopHover
                | FaceTopPressed
                | FaceBottom
                | FaceBottomHover
                | FaceBottomPressed
                | EdgeHighlight
                | EdgeHighlightHover
                | EdgeHighlightPressed
                | InnerShadow
                | EdgeShadow
                | AccentFaceTop
                | AccentFaceTopHover
                | AccentFaceTopPressed
                | AccentFaceBottom
                | AccentFaceBottomHover
                | AccentFaceBottomPressed
                | OnAccent
                | DangerFaceTop
                | DangerFaceTopHover
                | DangerFaceTopPressed
                | DangerFaceBottom
                | DangerFaceBottomHover
                | DangerFaceBottomPressed
                | OnDanger
                | Ink(_)
                | AccentMatch
                | RowZebra
                | RowHover
                | RowHeld
                | WellDeep
                | EdgeHard
                | StatusOk
                | WarnMeta
                | AccentDirty
                | AccentGrip
                | AccentGlyph
                | DangerText
        )
    }
}

/// Forge's ink ladder: every text and glyph colour in the design is one of
/// these roles, from the brightest (`Max`) down to the empty-state glyph
/// (`Empty`). Ask for the role, never restate its colour at a call site.
/// Resolved through [`StyleKey::Ink`]; the values live in [`Theme::ink`].
///
/// [`Theme::ink`]: crate::Theme::ink
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ink {
    /// `--ink-white`.
    White,
    /// Active tab, pressed label, latched icon (`--ink-max`).
    Max,
    /// Input value, key icon hover (`--ink-value`).
    Value,
    /// Key label, row emphasis (`--ink-emph`).
    Emph,
    /// Menu and sheet items (`--ink-menu`).
    Menu,
    /// Menu titles, list and tree labels (`--ink-title`).
    Title,
    /// Panel row labels (`--ink-row`).
    Row,
    /// Table cells, ghost keys (`--ink-cell`).
    Cell,
    /// Secondary values, pressed key labels (`--ink-2`).
    Second,
    /// Idle key icons (`--ink-icon`).
    Icon,
    /// Neutral chips (`--ink-chip`).
    Chip,
    /// Dim input values, panel glyphs, section titles (`--ink-glyph`).
    Glyph,
    /// Idle tabs (`--ink-tab`).
    Tab,
    /// Tooltip shortcuts (`--ink-tip-hint`).
    TipHint,
    /// Secondary body text, idle header keys, carets (`--ink-body-2`).
    Body2,
    /// Status bar, descriptions, list glyphs (`--ink-muted`).
    Muted,
    /// Menu shortcuts (`--ink-shortcut`).
    Shortcut,
    /// Mono-caps field labels (`--ink-label`).
    Label,
    /// Panel captions and row metadata (`--ink-caption`).
    Caption,
    /// Dim metadata, units, counts (`--ink-dim`).
    Dim,
    /// Disabled labels (`--ink-disabled`).
    Disabled,
    /// Disabled glyphs and metadata (`--ink-disabled-glyph`).
    DisabledGlyph,
    /// Disabled key labels (`--ink-disabled-key`).
    DisabledKey,
    /// Empty-state glyphs (`--ink-empty`).
    Empty,
    /// The wordmark (`--ink-brand`).
    Brand,
    /// Page body text (`--ink-primary`).
    Primary,
    /// Secondary text on an accent row (`--ink-on-accent-2`). The primary
    /// on-accent ink is [`StyleKey::OnAccent`].
    OnAccentSecond,
}

impl Ink {
    /// Every role, in ladder order (the index into [`Theme::ink`]).
    ///
    /// [`Theme::ink`]: crate::Theme::ink
    pub const ALL: [Ink; 27] = [
        Ink::White,
        Ink::Max,
        Ink::Value,
        Ink::Emph,
        Ink::Menu,
        Ink::Title,
        Ink::Row,
        Ink::Cell,
        Ink::Second,
        Ink::Icon,
        Ink::Chip,
        Ink::Glyph,
        Ink::Tab,
        Ink::TipHint,
        Ink::Body2,
        Ink::Muted,
        Ink::Shortcut,
        Ink::Label,
        Ink::Caption,
        Ink::Dim,
        Ink::Disabled,
        Ink::DisabledGlyph,
        Ink::DisabledKey,
        Ink::Empty,
        Ink::Brand,
        Ink::Primary,
        Ink::OnAccentSecond,
    ];
}

/// Forge's type scale below body text. Body text itself is
/// [`StyleKey::FontSize`]. Resolved through [`StyleKey::TextSize`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextSize {
    /// Mono capitals: field labels, captions, badges, section titles (9px).
    Caption,
    /// Mono metadata: shortcuts, counts, readouts (10px).
    Meta,
    /// Dense row text (10.5px).
    Dense,
    /// Panel rows, tabs, tooltips, chips, key labels (11px).
    Row,
    /// Menu items, buttons, list labels: the chrome ceiling (11.5px).
    Menu,
}

impl TextSize {
    /// Every step, smallest first (the index into `Theme::text_sizes`).
    pub const ALL: [TextSize; 5] = [
        TextSize::Caption,
        TextSize::Meta,
        TextSize::Dense,
        TextSize::Row,
        TextSize::Menu,
    ];
}

/// Letter spacing per text role, in em (multiply by the font size for
/// pixels). Resolved through [`StyleKey::Tracking`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tracking {
    /// Badges (0.08em).
    Badge,
    /// Inspector property labels (0.1em).
    Prop,
    /// Field labels (0.14em).
    Caption,
    /// Panel captions and section headers (0.16em).
    Section,
    /// Wordmark and page headings (0.18em).
    Brand,
}

impl Tracking {
    /// Every role (the index into `Theme::tracking`).
    pub const ALL: [Tracking; 5] = [
        Tracking::Badge,
        Tracking::Prop,
        Tracking::Caption,
        Tracking::Section,
        Tracking::Brand,
    ];
}

/// A caller-owned sparse set of style overrides layered over a [`Theme`].
///
/// Build one, [`set`](Self::set) the keys you want to override, then hand it to
/// a widget via [`DrawContext::with_style`](crate::DrawContext::with_style) (or
/// a [`StyleResolver`]). Anything resolved finds the overlay value first and the
/// theme otherwise — so a single overlay can recolor a subtree without touching
/// or cloning the theme.
///
/// Backed by a `Vec` rather than a `HashMap`: override sets are tiny (a handful
/// of keys), so a linear scan is faster and allocation-lighter than hashing.
#[derive(Clone, Debug, Default)]
pub struct StyleOverlay {
    entries: Vec<(StyleKey, StyleValue)>,
    menu_bar: Option<crate::MenuBarChrome>,
    menu_sheet: Option<crate::MenuSheetChrome>,
    toolbar: Option<crate::ToolbarChrome>,
    dock: Option<crate::DockChrome>,
    dock_section: Option<crate::DockSectionChrome>,
    group_list: Option<crate::GroupListChrome>,
    scrollbar: Option<crate::ScrollbarChrome>,
    splitter: Option<crate::SplitterChrome>,
    status_bar: Option<crate::StatusBarChrome>,
    dropdown: Option<crate::FloatingSurfaceChrome>,
    popover: Option<crate::FloatingSurfaceChrome>,
    tooltip: Option<crate::FloatingSurfaceChrome>,
    toast: Option<crate::FloatingSurfaceChrome>,
    curve_key: Option<crate::FloatingSurfaceChrome>,
    sheet: Option<crate::SheetChrome>,
    inspector: Option<crate::InspectorChrome>,
    window: Option<crate::WindowChrome>,
}

impl StyleOverlay {
    /// An empty overlay (resolves to the theme for every key).
    pub fn new() -> Self {
        Self::default()
    }

    /// Override `key` with `value`. Replaces any existing entry for `key`.
    /// Returns `&mut self` for chaining.
    pub fn set(&mut self, key: StyleKey, value: StyleValue) -> &mut Self {
        if let Some(slot) = self.entries.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            self.entries.push((key, value));
        }
        self
    }

    /// Convenience for `set(key, StyleValue::Color(c))`.
    pub fn set_color(&mut self, key: StyleKey, c: [f32; 4]) -> &mut Self {
        self.set(key, StyleValue::Color(c))
    }

    /// Convenience for `set(key, StyleValue::Scalar(s))`.
    pub fn set_scalar(&mut self, key: StyleKey, s: f32) -> &mut Self {
        self.set(key, StyleValue::Scalar(s))
    }

    /// The overridden value for `key`, if any.
    pub fn get(&self, key: StyleKey) -> Option<StyleValue> {
        self.entries
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
    }

    /// Override menu-bar chrome.
    pub fn set_menu_bar(&mut self, value: crate::MenuBarChrome) -> &mut Self {
        self.menu_bar = Some(value);
        self
    }
    /// Return the menu-bar override.
    pub fn menu_bar(&self) -> Option<crate::MenuBarChrome> {
        self.menu_bar
    }
    /// Override menu-sheet chrome.
    pub fn set_menu_sheet(&mut self, value: crate::MenuSheetChrome) -> &mut Self {
        self.menu_sheet = Some(value);
        self
    }
    /// Return the menu-sheet override.
    pub fn menu_sheet(&self) -> Option<crate::MenuSheetChrome> {
        self.menu_sheet
    }
    /// Override toolbar chrome.
    pub fn set_toolbar(&mut self, value: crate::ToolbarChrome) -> &mut Self {
        self.toolbar = Some(value);
        self
    }
    /// Return the toolbar override.
    pub fn toolbar(&self) -> Option<crate::ToolbarChrome> {
        self.toolbar
    }
    /// Override dock-panel chrome.
    pub fn set_dock(&mut self, value: crate::DockChrome) -> &mut Self {
        self.dock = Some(value);
        self
    }
    /// Return the dock-panel override.
    pub fn dock(&self) -> Option<crate::DockChrome> {
        self.dock
    }
    /// Override dock-section chrome.
    pub fn set_dock_section(&mut self, value: crate::DockSectionChrome) -> &mut Self {
        self.dock_section = Some(value);
        self
    }
    /// Return the dock-section override.
    pub fn dock_section(&self) -> Option<crate::DockSectionChrome> {
        self.dock_section
    }
    /// Override grouped-list chrome.
    pub fn set_group_list(&mut self, value: crate::GroupListChrome) -> &mut Self {
        self.group_list = Some(value);
        self
    }
    /// Return the grouped-list override.
    pub fn group_list(&self) -> Option<crate::GroupListChrome> {
        self.group_list
    }
    /// Override scrollbar chrome.
    pub fn set_scrollbar(&mut self, value: crate::ScrollbarChrome) -> &mut Self {
        self.scrollbar = Some(value);
        self
    }
    /// Return the scrollbar override.
    pub fn scrollbar(&self) -> Option<crate::ScrollbarChrome> {
        self.scrollbar
    }
    /// Override splitter chrome.
    pub fn set_splitter(&mut self, value: crate::SplitterChrome) -> &mut Self {
        self.splitter = Some(value);
        self
    }
    /// Return the splitter override.
    pub fn splitter(&self) -> Option<crate::SplitterChrome> {
        self.splitter
    }
    /// Override status-bar chrome.
    pub fn set_status_bar(&mut self, value: crate::StatusBarChrome) -> &mut Self {
        self.status_bar = Some(value);
        self
    }
    /// Return the status-bar override.
    pub fn status_bar(&self) -> Option<crate::StatusBarChrome> {
        self.status_bar
    }
    /// Override dropdown chrome.
    pub fn set_dropdown(&mut self, value: crate::FloatingSurfaceChrome) -> &mut Self {
        self.dropdown = Some(value);
        self
    }
    /// Return the dropdown override.
    pub fn dropdown(&self) -> Option<crate::FloatingSurfaceChrome> {
        self.dropdown
    }
    /// Override popover chrome.
    pub fn set_popover(&mut self, value: crate::FloatingSurfaceChrome) -> &mut Self {
        self.popover = Some(value);
        self
    }
    /// Return the popover override.
    pub fn popover(&self) -> Option<crate::FloatingSurfaceChrome> {
        self.popover
    }
    /// Override tooltip chrome.
    pub fn set_tooltip(&mut self, value: crate::FloatingSurfaceChrome) -> &mut Self {
        self.tooltip = Some(value);
        self
    }
    /// Return the tooltip override.
    pub fn tooltip(&self) -> Option<crate::FloatingSurfaceChrome> {
        self.tooltip
    }
    /// Override toast chrome.
    pub fn set_toast(&mut self, value: crate::FloatingSurfaceChrome) -> &mut Self {
        self.toast = Some(value);
        self
    }
    /// Return the toast override.
    pub fn toast(&self) -> Option<crate::FloatingSurfaceChrome> {
        self.toast
    }
    /// Override curve-editor key chrome.
    pub fn set_curve_key(&mut self, value: crate::FloatingSurfaceChrome) -> &mut Self {
        self.curve_key = Some(value);
        self
    }
    /// Return the curve-editor key override.
    pub fn curve_key(&self) -> Option<crate::FloatingSurfaceChrome> {
        self.curve_key
    }
    /// Override dialog-sheet chrome.
    pub fn set_sheet(&mut self, value: crate::SheetChrome) -> &mut Self {
        self.sheet = Some(value);
        self
    }
    /// Return the dialog-sheet override.
    pub fn sheet(&self) -> Option<crate::SheetChrome> {
        self.sheet
    }
    /// Override property-inspector chrome.
    pub fn set_inspector(&mut self, value: crate::InspectorChrome) -> &mut Self {
        self.inspector = Some(value);
        self
    }
    /// Return the property-inspector override.
    pub fn inspector(&self) -> Option<crate::InspectorChrome> {
        self.inspector
    }
    /// Override movable-window chrome.
    pub fn set_window(&mut self, value: crate::WindowChrome) -> &mut Self {
        self.window = Some(value);
        self
    }
    /// Return the movable-window override.
    pub fn window(&self) -> Option<crate::WindowChrome> {
        self.window
    }

    /// Whether the overlay has no scalar, color, or component overrides.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
            && self.menu_bar.is_none()
            && self.menu_sheet.is_none()
            && self.toolbar.is_none()
            && self.dock.is_none()
            && self.dock_section.is_none()
            && self.group_list.is_none()
            && self.scrollbar.is_none()
            && self.splitter.is_none()
            && self.status_bar.is_none()
            && self.dropdown.is_none()
            && self.popover.is_none()
            && self.tooltip.is_none()
            && self.toast.is_none()
            && self.curve_key.is_none()
            && self.sheet.is_none()
            && self.inspector.is_none()
            && self.window.is_none()
    }

    /// Drop all scalar, color, and typed component overrides while retaining
    /// the scalar/color entry allocation for reuse.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.menu_bar = None;
        self.menu_sheet = None;
        self.toolbar = None;
        self.dock = None;
        self.dock_section = None;
        self.group_list = None;
        self.scrollbar = None;
        self.splitter = None;
        self.status_bar = None;
        self.dropdown = None;
        self.popover = None;
        self.tooltip = None;
        self.toast = None;
        self.curve_key = None;
        self.sheet = None;
        self.inspector = None;
        self.window = None;
    }
}

/// The single style read path: an optional [`StyleOverlay`] layered over a
/// [`Theme`]. Borrows both; holds no state and is cheap to construct on demand.
///
/// Resolution precedence is **overlay → theme**. Built-in keys always resolve
/// (the theme has a value for each); `Custom` keys resolve only if set on the
/// overlay or registered on the theme, so read those with [`color_or`](Self::color_or)
/// / [`scalar_or`](Self::scalar_or).
#[derive(Clone, Copy)]
pub struct StyleResolver<'a> {
    theme: &'a Theme,
    overlay: Option<&'a StyleOverlay>,
}

impl<'a> StyleResolver<'a> {
    /// A resolver over `theme` with no overrides.
    pub fn new(theme: &'a Theme) -> Self {
        Self {
            theme,
            overlay: None,
        }
    }

    /// A resolver over `theme` with `overlay` taking precedence.
    pub fn with_overlay(theme: &'a Theme, overlay: &'a StyleOverlay) -> Self {
        Self {
            theme,
            overlay: Some(overlay),
        }
    }

    /// A resolver over `theme` with an optional overlay (convenience for call
    /// sites that hold an `Option<&StyleOverlay>`).
    pub fn with_overlay_opt(theme: &'a Theme, overlay: Option<&'a StyleOverlay>) -> Self {
        Self { theme, overlay }
    }

    /// The theme this resolver reads from (for non-style fields like `font`).
    pub fn theme(&self) -> &'a Theme {
        self.theme
    }

    /// Resolve `key` to its value: overlay first, then theme. `None` only for a
    /// `Custom` key that's set in neither.
    pub fn get(&self, key: StyleKey) -> Option<StyleValue> {
        if let Some(v) = self.overlay.and_then(|o| o.get(key)) {
            return Some(v);
        }
        self.theme.get(key)
    }

    /// Resolve a color key. Built-in color keys always resolve; a missing/
    /// mismatched value falls back to opaque magenta as a loud "unset" sentinel
    /// (only reachable by misusing a `Custom`/scalar key here — use
    /// [`color_or`](Self::color_or) for those).
    pub fn color(&self, key: StyleKey) -> [f32; 4] {
        self.color_or(key, [1.0, 0.0, 1.0, 1.0])
    }

    /// Resolve a color key, falling back to `default` when unset or non-color.
    pub fn color_or(&self, key: StyleKey, default: [f32; 4]) -> [f32; 4] {
        self.get(key)
            .and_then(StyleValue::as_color)
            .unwrap_or(default)
    }

    /// Resolve a scalar key. Built-in scalar keys always resolve; otherwise `0.0`
    /// (use [`scalar_or`](Self::scalar_or) for `Custom` keys).
    pub fn scalar(&self, key: StyleKey) -> f32 {
        self.scalar_or(key, 0.0)
    }

    /// Resolve a scalar key, falling back to `default` when unset or non-scalar.
    pub fn scalar_or(&self, key: StyleKey, default: f32) -> f32 {
        self.get(key)
            .and_then(StyleValue::as_scalar)
            .unwrap_or(default)
    }

    /// Resolve menu-bar chrome in O(1).
    pub fn menu_bar(&self) -> crate::MenuBarChrome {
        self.overlay
            .and_then(StyleOverlay::menu_bar)
            .unwrap_or(self.theme.chrome.menu_bar)
    }
    /// Resolve menu-sheet chrome in O(1).
    pub fn menu_sheet(&self) -> crate::MenuSheetChrome {
        self.overlay
            .and_then(StyleOverlay::menu_sheet)
            .unwrap_or(self.theme.chrome.menu_sheet)
    }
    /// Resolve toolbar chrome in O(1).
    pub fn toolbar(&self) -> crate::ToolbarChrome {
        self.overlay
            .and_then(StyleOverlay::toolbar)
            .unwrap_or(self.theme.chrome.toolbar)
    }
    /// Resolve dock-panel chrome in O(1).
    pub fn dock(&self) -> crate::DockChrome {
        self.overlay
            .and_then(StyleOverlay::dock)
            .unwrap_or(self.theme.chrome.dock)
    }
    /// Resolve dock-section chrome in O(1).
    pub fn dock_section(&self) -> crate::DockSectionChrome {
        self.overlay
            .and_then(StyleOverlay::dock_section)
            .unwrap_or(self.theme.chrome.dock_section)
    }
    /// Resolve grouped-list chrome in O(1).
    pub fn group_list(&self) -> crate::GroupListChrome {
        self.overlay
            .and_then(StyleOverlay::group_list)
            .unwrap_or(self.theme.chrome.group_list)
    }
    /// Resolve scrollbar chrome in O(1).
    pub fn scrollbar(&self) -> crate::ScrollbarChrome {
        self.overlay
            .and_then(StyleOverlay::scrollbar)
            .unwrap_or(self.theme.chrome.scrollbar)
    }
    /// Resolve splitter chrome in O(1).
    pub fn splitter(&self) -> crate::SplitterChrome {
        self.overlay
            .and_then(StyleOverlay::splitter)
            .unwrap_or(self.theme.chrome.splitter)
    }
    /// Resolve status-bar chrome in O(1).
    pub fn status_bar(&self) -> crate::StatusBarChrome {
        self.overlay
            .and_then(StyleOverlay::status_bar)
            .unwrap_or(self.theme.chrome.status_bar)
    }
    /// Resolve dropdown chrome in O(1).
    pub fn dropdown(&self) -> crate::FloatingSurfaceChrome {
        self.overlay
            .and_then(StyleOverlay::dropdown)
            .unwrap_or(self.theme.chrome.dropdown)
    }
    /// Resolve popover chrome in O(1).
    pub fn popover(&self) -> crate::FloatingSurfaceChrome {
        self.overlay
            .and_then(StyleOverlay::popover)
            .unwrap_or(self.theme.chrome.popover)
    }
    /// Resolve tooltip chrome in O(1).
    pub fn tooltip(&self) -> crate::FloatingSurfaceChrome {
        self.overlay
            .and_then(StyleOverlay::tooltip)
            .unwrap_or(self.theme.chrome.tooltip)
    }
    /// Resolve toast chrome in O(1).
    pub fn toast(&self) -> crate::FloatingSurfaceChrome {
        self.overlay
            .and_then(StyleOverlay::toast)
            .unwrap_or(self.theme.chrome.toast)
    }
    /// Resolve curve-editor key chrome in O(1).
    pub fn curve_key(&self) -> crate::FloatingSurfaceChrome {
        self.overlay
            .and_then(StyleOverlay::curve_key)
            .unwrap_or(self.theme.chrome.curve_key)
    }
    /// Resolve dialog-sheet chrome in O(1).
    pub fn sheet(&self) -> crate::SheetChrome {
        self.overlay
            .and_then(StyleOverlay::sheet)
            .unwrap_or(self.theme.chrome.sheet)
    }
    /// Resolve property-inspector chrome in O(1).
    pub fn inspector(&self) -> crate::InspectorChrome {
        self.overlay
            .and_then(StyleOverlay::inspector)
            .unwrap_or(self.theme.chrome.inspector)
    }
    /// Resolve movable-window chrome in O(1).
    pub fn window(&self) -> crate::WindowChrome {
        self.overlay
            .and_then(StyleOverlay::window)
            .unwrap_or(self.theme.chrome.window)
    }

    /// A body [`TextBlock`] styled through the resolver: [`FontSize`](StyleKey::FontSize)
    /// size, [`Text`](StyleKey::Text) color, and the theme font. The overlay-aware
    /// counterpart of [`Theme::text`](crate::Theme::text).
    pub fn text_block(&self, content: impl Into<String>, x: f32, y: f32) -> TextBlock {
        let c = self.color(StyleKey::Text);
        TextBlock::new(content, x, y)
            .with_size(self.scalar(StyleKey::FontSize))
            .with_color_f32(c)
            .with_font_opt(self.theme.font.clone())
    }

    /// The colour of an ink-ladder role.
    pub fn ink(&self, role: Ink) -> [f32; 4] {
        self.color(StyleKey::Ink(role))
    }

    /// A step of the type scale, in pixels.
    pub fn text_size(&self, step: TextSize) -> f32 {
        self.scalar(StyleKey::TextSize(step))
    }

    /// A sans [`TextBlock`] at a type-scale `step` in an ink `role`, in the
    /// theme font.
    pub fn sans_block(
        &self,
        content: impl Into<String>,
        x: f32,
        y: f32,
        step: TextSize,
        role: Ink,
    ) -> TextBlock {
        TextBlock::new(content, x, y)
            .with_size(self.text_size(step))
            .with_color_f32(self.ink(role))
            .with_font_opt(self.theme.font.clone())
    }

    /// A mono [`TextBlock`] at a type-scale `step` in an ink `role`, in the
    /// theme's [`mono_font`](crate::Theme::mono_font): counts, metadata,
    /// readouts.
    pub fn mono_block(
        &self,
        content: impl Into<String>,
        x: f32,
        y: f32,
        step: TextSize,
        role: Ink,
    ) -> TextBlock {
        TextBlock::new(content, x, y)
            .with_size(self.text_size(step))
            .with_color_f32(self.ink(role))
            .with_font_opt(self.theme.mono_font.clone())
    }

    /// The single-line width of `text` as [`mono_block`](Self::mono_block)
    /// draws it at `step`. Measuring mono text with
    /// [`DrawList::measure_text`](crate::DrawList::measure_text) uses the
    /// default font and comes out too narrow. No allocation.
    pub fn mono_width(&self, list: &mut crate::DrawList, text: &str, step: TextSize) -> f32 {
        list.measure_text_with_font(
            text,
            self.text_size(step),
            None,
            self.theme.mono_font.as_ref(),
        )
        .0
    }

    /// The single-line width of `text` as [`sans_block`](Self::sans_block)
    /// draws it at `step`. No allocation.
    pub fn sans_width(&self, list: &mut crate::DrawList, text: &str, step: TextSize) -> f32 {
        list.measure_text_with_font(text, self.text_size(step), None, self.theme.font.as_ref())
            .0
    }

    /// A mono-capitals caption: `content` upper-cased, at the caption size,
    /// letter-spaced by `tracking`, in an ink `role`. Section titles, field
    /// labels, badges.
    pub fn caption_block(
        &self,
        content: &str,
        x: f32,
        y: f32,
        tracking: Tracking,
        role: Ink,
    ) -> TextBlock {
        let size = self.text_size(TextSize::Caption);
        self.mono_block(content.to_uppercase(), x, y, TextSize::Caption, role)
            .with_letter_spacing(size * self.scalar(StyleKey::Tracking(tracking)))
    }

    /// A title [`TextBlock`] styled through the resolver: [`FontSizeTitle`](StyleKey::FontSizeTitle)
    /// size, [`Text`](StyleKey::Text) color, and the theme font. The overlay-aware
    /// counterpart of [`Theme::title`](crate::Theme::title).
    pub fn title_block(&self, content: impl Into<String>, x: f32, y: f32) -> TextBlock {
        let c = self.color(StyleKey::Text);
        TextBlock::new(content, x, y)
            .with_size(self.scalar(StyleKey::FontSizeTitle))
            .with_color_f32(c)
            .with_font_opt(self.theme.font.clone())
    }
}

/// All built-in color keys, paired with the value the default theme stores —
/// used by the round-trip test and handy for tooling.
#[cfg(test)]
pub(crate) const COLOR_KEYS: &[StyleKey] = &[
    StyleKey::Background,
    StyleKey::Scrim,
    StyleKey::Panel,
    StyleKey::PanelBorder,
    StyleKey::Button,
    StyleKey::ButtonHover,
    StyleKey::ButtonPressed,
    StyleKey::ButtonBorder,
    StyleKey::InputBackground,
    StyleKey::InputBorder,
    StyleKey::InputFocusBorder,
    StyleKey::Text,
    StyleKey::TextDim,
    StyleKey::TextHighlight,
    StyleKey::Accent,
    StyleKey::AccentTick,
    StyleKey::Info,
    StyleKey::Success,
    StyleKey::Warning,
    StyleKey::Error,
    StyleKey::FocusRing,
    StyleKey::TabInactive,
    StyleKey::TabActive,
    StyleKey::TabHover,
    StyleKey::TabBorder,
    StyleKey::ProgressBackground,
    StyleKey::ProgressFill,
    StyleKey::ProgressFillLow,
    StyleKey::ProgressFillMedium,
    StyleKey::Plinth,
    StyleKey::FaceTop,
    StyleKey::FaceTopHover,
    StyleKey::FaceTopPressed,
    StyleKey::FaceBottom,
    StyleKey::FaceBottomHover,
    StyleKey::FaceBottomPressed,
    StyleKey::EdgeHighlight,
    StyleKey::EdgeHighlightHover,
    StyleKey::EdgeHighlightPressed,
    StyleKey::InnerShadow,
    StyleKey::EdgeShadow,
    StyleKey::AccentFaceTop,
    StyleKey::AccentFaceTopHover,
    StyleKey::AccentFaceTopPressed,
    StyleKey::AccentFaceBottom,
    StyleKey::AccentFaceBottomHover,
    StyleKey::AccentFaceBottomPressed,
    StyleKey::OnAccent,
    StyleKey::DangerFaceTop,
    StyleKey::DangerFaceTopHover,
    StyleKey::DangerFaceTopPressed,
    StyleKey::DangerFaceBottom,
    StyleKey::DangerFaceBottomHover,
    StyleKey::DangerFaceBottomPressed,
    StyleKey::OnDanger,
    StyleKey::AccentMatch,
    StyleKey::RowZebra,
    StyleKey::RowHover,
    StyleKey::RowHeld,
    StyleKey::WellDeep,
    StyleKey::EdgeHard,
    StyleKey::StatusOk,
    StyleKey::WarnMeta,
    StyleKey::AccentDirty,
    StyleKey::AccentGrip,
    StyleKey::AccentGlyph,
    StyleKey::DangerText,
];

#[cfg(test)]
pub(crate) const SCALAR_KEYS: &[StyleKey] = &[
    StyleKey::Padding,
    StyleKey::Spacing,
    StyleKey::BorderRadius,
    StyleKey::BorderWidth,
    StyleKey::FontSize,
    StyleKey::FontSizeTitle,
    StyleKey::ButtonHeight,
    StyleKey::InputHeight,
    StyleKey::AnimationDuration,
    StyleKey::MenuBarHeight,
    StyleKey::MenuRowHeight,
    StyleKey::MenuItemMinWidth,
    StyleKey::MenuAccelGap,
    StyleKey::MenuHoverDelay,
    StyleKey::Travel,
    StyleKey::InnerShadowDepth,
    StyleKey::ListRowHeight,
];

/// Internal helper for [`Theme`]'s custom map type (kept here so the key/value
/// types live together).
pub(crate) type CustomStyles = HashMap<u64, StyleValue>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_value_accessors() {
        assert_eq!(
            StyleValue::Color([1.0, 2.0, 3.0, 4.0]).as_color(),
            Some([1.0, 2.0, 3.0, 4.0])
        );
        assert_eq!(StyleValue::Color([1.0, 2.0, 3.0, 4.0]).as_scalar(), None);
        assert_eq!(StyleValue::Scalar(7.0).as_scalar(), Some(7.0));
        assert_eq!(StyleValue::Scalar(7.0).as_color(), None);
    }

    #[test]
    fn custom_key_is_stable_and_name_sensitive() {
        assert_eq!(StyleKey::custom("widget.bg"), StyleKey::custom("widget.bg"));
        assert_ne!(StyleKey::custom("widget.bg"), StyleKey::custom("widget.fg"));
        // Built-in keys are not colored-coded as custom.
        assert!(StyleKey::Accent.is_color());
        assert!(!StyleKey::Padding.is_color());
    }

    #[test]
    fn overlay_set_get_and_replace() {
        let mut o = StyleOverlay::new();
        assert!(o.is_empty());
        o.set_color(StyleKey::Button, [0.1, 0.2, 0.3, 1.0]);
        assert_eq!(
            o.get(StyleKey::Button),
            Some(StyleValue::Color([0.1, 0.2, 0.3, 1.0]))
        );
        // Replace, not duplicate.
        o.set_color(StyleKey::Button, [0.9, 0.9, 0.9, 1.0]);
        assert_eq!(o.entries.len(), 1);
        assert_eq!(
            o.get(StyleKey::Button),
            Some(StyleValue::Color([0.9, 0.9, 0.9, 1.0]))
        );
        o.clear();
        assert!(o.is_empty());
    }

    #[test]
    fn resolver_precedence_overlay_then_theme() {
        let theme = Theme::default();
        // No overlay: built-in resolves to the theme field.
        let r = StyleResolver::new(&theme);
        assert_eq!(r.color(StyleKey::Accent), theme.accent);
        assert_eq!(r.scalar(StyleKey::Padding), theme.padding);

        // With overlay: the override wins, other keys still come from the theme.
        let mut o = StyleOverlay::new();
        o.set_color(StyleKey::Accent, [0.0, 0.0, 0.0, 1.0]);
        let r = StyleResolver::with_overlay(&theme, &o);
        assert_eq!(r.color(StyleKey::Accent), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(r.color(StyleKey::Button), theme.button);
    }

    #[test]
    fn resolver_custom_key_uses_default_when_unset() {
        let theme = Theme::default();
        let r = StyleResolver::new(&theme);
        let key = StyleKey::custom("missing");
        assert_eq!(r.color_or(key, [0.5, 0.5, 0.5, 1.0]), [0.5, 0.5, 0.5, 1.0]);
        assert_eq!(r.scalar_or(key, 3.0), 3.0);
    }

    #[test]
    fn typed_component_override_resolves_and_clear_covers_it() {
        let theme = Theme::default();
        let mut override_value = theme.chrome.splitter;
        override_value.grip_idle = [0.1, 0.2, 0.3, 0.4];
        let mut overlay = StyleOverlay::new();
        overlay.set_splitter(override_value);

        assert!(!overlay.is_empty());
        assert_eq!(
            StyleResolver::with_overlay(&theme, &overlay)
                .splitter()
                .grip_idle,
            [0.1, 0.2, 0.3, 0.4]
        );
        assert_eq!(StyleResolver::new(&theme).splitter(), theme.chrome.splitter);

        overlay.clear();
        assert!(overlay.is_empty());
        assert_eq!(overlay.splitter(), None);

        let mut window = theme.chrome.window;
        window.grip_colors[0] = [0.4, 0.3, 0.2, 1.0];
        overlay.set_window(window);
        assert!(!overlay.is_empty());
        assert_eq!(
            StyleResolver::with_overlay(&theme, &overlay).window(),
            window
        );
        assert_eq!(StyleResolver::new(&theme).window(), theme.chrome.window);
        overlay.clear();
        assert_eq!(overlay.window(), None);

        let mut section = theme.chrome.dock_section;
        section.top_rule.color = [0.4, 0.3, 0.2, 1.0];
        overlay.set_dock_section(section);
        assert!(!overlay.is_empty());
        assert_eq!(
            StyleResolver::with_overlay(&theme, &overlay).dock_section(),
            section
        );
        assert_eq!(
            StyleResolver::new(&theme).dock_section(),
            theme.chrome.dock_section
        );
        overlay.clear();
        assert_eq!(overlay.dock_section(), None);

        let mut scrollbar = theme.chrome.scrollbar;
        scrollbar.thumb[1].border_color = [0.4, 0.3, 0.2, 1.0];
        overlay.set_scrollbar(scrollbar);
        assert!(!overlay.is_empty());
        assert_eq!(
            StyleResolver::with_overlay(&theme, &overlay).scrollbar(),
            scrollbar
        );
        assert_eq!(
            StyleResolver::new(&theme).scrollbar(),
            theme.chrome.scrollbar
        );
        overlay.clear();
        assert_eq!(overlay.scrollbar(), None);
    }

    #[test]
    fn overlay_component_reaches_surface_painter() {
        use crate::{CornerRadii, DrawList, SurfacePainter, layout::Rect};

        let theme = Theme::default();
        let mut menu = theme.chrome.menu_bar;
        menu.surface.background = crate::Background::Solid([0.7, 0.6, 0.5, 1.0]);
        let mut overlay = StyleOverlay::new();
        overlay.set_menu_bar(menu);
        let resolved = StyleResolver::with_overlay(&theme, &overlay).menu_bar();
        let mut list = DrawList::new();
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        let mut painter = SurfacePainter::new(
            &mut list,
            rect,
            rect.inset(1.0),
            CornerRadii::default(),
            resolved.surface,
            &resolved.shadows,
            &resolved.lines,
        );
        painter.paint_pre_content();
        painter.paint_post_content();

        assert_eq!(list.chrome_instance(0).unwrap().bg, [0.7, 0.6, 0.5, 1.0]);
    }
}
