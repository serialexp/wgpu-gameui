//! Text roles: what a piece of text is, and the theme's table saying how each
//! kind looks. See `docs/design/typography-roles.md`.

use std::borrow::Cow;

use cosmic_text::Weight;

use crate::shaping::LayoutSpec;
use crate::text::{Figures, FontHandle, TextBlock, TextContent};
use crate::{DrawList, Ink, StyleOverlay, StyleResolver};

/// Which of a theme's three fonts a [`TextStyle`] is set in.
///
/// [`Theme::font`](crate::Theme::font) is the sans,
/// [`Theme::mono_font`](crate::Theme::mono_font) the mono and
/// [`Theme::display_font`](crate::Theme::display_font) the display face. A
/// theme that names no display face draws display text in its sans.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontFamily {
    /// The UI face: running text, controls, rows.
    Sans,
    /// The technical face: counts, metadata, code, mono captions.
    Mono,
    /// The heading face: titles and an app's own headings and names.
    Display,
}

/// Everything about how one [`TextRole`] looks except its colour, which the
/// widget picks from the [`Ink`](crate::Ink) ladder per state.
///
/// Build one from [`sans`](Self::sans), [`mono`](Self::mono) or
/// [`display`](Self::display) and the `with_*` methods:
///
/// ```
/// use wgpu_gameui::{TextStyle, Weight};
/// let label = TextStyle::mono(9.0).with_tracking(0.14).with_caps(true);
/// let heading = TextStyle::display(22.0).with_italic(true).with_weight(Weight::MEDIUM);
/// assert_eq!(label.letter_spacing(), 9.0 * 0.14);
/// assert!(heading.italic);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    /// The font it is set in.
    pub family: FontFamily,
    /// Size in logical pixels.
    pub size: f32,
    /// Line height as a multiple of [`size`](Self::size).
    pub line_height: f32,
    /// Weight; the family must have the face (none is synthesized).
    pub weight: Weight,
    /// Set in the family's italic face.
    pub italic: bool,
    /// Letter spacing in em: [`letter_spacing`](Self::letter_spacing) is
    /// `tracking × size` pixels.
    pub tracking: f32,
    /// Drawn upper-cased. Editable text ignores it and shows what was typed.
    pub caps: bool,
    /// How digits are spaced: [`Figures::Tabular`] for clocks, counters
    /// and columns of figures, so changing numbers don't shift.
    pub figures: Figures,
}

impl TextStyle {
    /// Regular, upright text in `family` at `size`, with the default line
    /// height (1.25 × the size, as [`TextBlock::with_size`] sets), no tracking,
    /// mixed case and the font's own figures.
    pub const fn new(family: FontFamily, size: f32) -> Self {
        Self {
            family,
            size,
            line_height: crate::text::LINE_HEIGHT_RATIO,
            weight: Weight::NORMAL,
            italic: false,
            tracking: 0.0,
            caps: false,
            figures: Figures::Default,
        }
    }

    /// [`new`](Self::new) in the sans.
    pub const fn sans(size: f32) -> Self {
        Self::new(FontFamily::Sans, size)
    }

    /// [`new`](Self::new) in the mono.
    pub const fn mono(size: f32) -> Self {
        Self::new(FontFamily::Mono, size)
    }

    /// [`new`](Self::new) in the display face.
    pub const fn display(size: f32) -> Self {
        Self::new(FontFamily::Display, size)
    }

    /// With `size` in logical pixels.
    pub const fn with_size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// With a line height of `ratio × size`.
    pub const fn with_line_height(mut self, ratio: f32) -> Self {
        self.line_height = ratio;
        self
    }

    /// With `weight`.
    pub const fn with_weight(mut self, weight: Weight) -> Self {
        self.weight = weight;
        self
    }

    /// Set (or not) in the italic face.
    pub const fn with_italic(mut self, italic: bool) -> Self {
        self.italic = italic;
        self
    }

    /// With letter spacing of `em` (times the size).
    pub const fn with_tracking(mut self, em: f32) -> Self {
        self.tracking = em;
        self
    }

    /// Upper-cased (or not).
    pub const fn with_caps(mut self, caps: bool) -> Self {
        self.caps = caps;
        self
    }

    /// With digits spaced as `figures` asks.
    pub const fn with_figures(mut self, figures: Figures) -> Self {
        self.figures = figures;
        self
    }

    /// The letter spacing in pixels at this style's size.
    pub fn letter_spacing(&self) -> f32 {
        self.tracking * self.size
    }

    /// The line-box height in pixels at this style's size.
    pub fn line_height_px(&self) -> f32 {
        self.line_height * self.size
    }

    /// The cosmic-text style the face is picked by.
    pub fn font_style(&self) -> cosmic_text::Style {
        if self.italic {
            cosmic_text::Style::Italic
        } else {
            cosmic_text::Style::Normal
        }
    }
}

/// Declares [`TextRole`] from one list, so the enum, [`TextRole::ALL`] and
/// [`TextRole::name`] cannot drift apart.
macro_rules! text_roles {
    ($(
        $(#[$attr:meta])*
        $variant:ident($name:literal),
    )*) => {
        /// What a piece of text is. A widget names the role of every text it
        /// draws; the theme's [`Typography`] says how each role looks (see
        /// [`StyleResolver::role_block`](crate::StyleResolver::role_block)).
        ///
        /// The chrome roles are the ones gameui's widgets draw. `Display`,
        /// `Value` and `Numeric` have no gameui caller (and `Heading` only
        /// the settings form): they are for an app's own screens, so its
        /// text follows the theme too.
        ///
        /// Where two widgets show the same kind of text at different sizes,
        /// each has its own role, so the built-in themes keep their look and a
        /// theme can still style them apart.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum TextRole {
            $($(#[$attr])* $variant,)*
        }

        impl TextRole {
            /// Every role, in declaration order (the index into a
            /// [`Typography`]).
            pub const ALL: [TextRole; [$(TextRole::$variant,)*].len()] =
                [$(TextRole::$variant,)*];

            /// The role's name: `snake_case`, as a preset or the designer
            /// shows it.
            pub const fn name(self) -> &'static str {
                match self {
                    $(TextRole::$variant => $name,)*
                }
            }

            /// The role called `name` (see [`name`](Self::name)), if any.
            pub fn from_name(name: &str) -> Option<TextRole> {
                Self::ALL.into_iter().find(|role| role.name() == name)
            }
        }
    };
}

text_roles! {
    /// Field labels and the caps captions over a block (a status detail, a
    /// popover, the file dialog's groups): mono capitals.
    Label("label"),
    /// Panel captions and section headers.
    Section("section"),
    /// Badge text.
    Badge("badge"),
    /// Compact badges: case kept, barely tracked.
    BadgeCompact("badge_compact"),
    /// Inspector property labels.
    Property("property"),
    /// Column headers over a list (the file dialog's).
    ListHeader("list_header"),
    /// Short capital flags: an empty chart's caption, a point's flag, a
    /// diagnostic's severity.
    Flag("flag"),
    /// Axis ticks and the small mono marks beside data: units, counts,
    /// paths, kind tags.
    Axis("axis"),
    /// The time on a timeline's playhead.
    Playhead("playhead"),
    /// Shortcuts, counts, metadata and readouts.
    Meta("meta"),
    /// Technical detail under a dialog's message (an error's text).
    Detail("detail"),
    /// Code, timecodes and other technical text set at reading size.
    Code("code"),
    /// A code editor's text.
    Editor("editor"),
    /// Small secondary text: a choice's description, a notice.
    Hint("hint"),
    /// How-to text in an empty state or an empty inspector.
    Help("help"),
    /// A dialog's explanatory note (the prompt dialog's).
    Note("note"),
    /// Dense row text and filter chips.
    Dense("dense"),
    /// Panel rows, legends and tooltip text.
    Row("row"),
    /// A toast's message.
    Message("message"),
    /// Menu and context-menu items.
    MenuItem("menu_item"),
    /// List, tree and file labels.
    ListItem("list_item"),
    /// List labels set in mono (paths, ids).
    ListItemMono("list_item_mono"),
    /// The headline of a small surface: a toast, an empty state, the
    /// inspector's subject.
    Lead("lead"),
    /// A sheet's description.
    Description("description"),
    /// A title over media: a video's, its error headline.
    MediaTitle("media_title"),
    /// A sheet's title.
    Subhead("subhead"),
    /// Running text and text inputs; the root of the font stack.
    Body("body"),
    /// Button labels.
    Button("button"),
    /// Checkbox, radio, toggle, dropdown and setting labels.
    Control("control"),
    /// Combo box text, closed and open.
    Picker("picker"),
    /// Vector field cells.
    FieldCell("field_cell"),
    /// Tag input chips.
    Chip("chip"),
    /// Keycap labels.
    Keycap("keycap"),
    /// A slider's or gradient ramp's value readout.
    Readout("readout"),
    /// Labels under grid tiles (the asset grid's).
    TileLabel("tile_label"),
    /// A labeled progress bar's label.
    ProgressLabel("progress_label"),
    /// Status bar text.
    Status("status"),
    /// Breadcrumb pager text.
    Pager("pager"),
    /// [`TooltipLayer`](crate::TooltipLayer) titles.
    TipTitle("tip_title"),
    /// [`TooltipLayer`](crate::TooltipLayer) text.
    TipBody("tip_body"),
    /// [`Tabs`](crate::Tabs) labels.
    Tab("tab"),
    /// Document tab labels ([`draw_doc_tabs`](crate::draw_doc_tabs)).
    DocTab("doc_tab"),
    /// Dock panel tab labels.
    PanelTab("panel_tab"),
    /// [`SpanTabs`](crate::SpanTabs) labels.
    SpanTab("span_tab"),
    /// Table cells.
    Cell("cell"),
    /// Table column headers.
    ColumnHeader("column_header"),
    /// Window and group title bars.
    FrameTitle("frame_title"),
    /// Panel titles.
    Title("title"),
    /// Dialog titles in a header bar (the file dialog's).
    DialogTitle("dialog_title"),
    /// Section headings (a settings form's, and an app's own).
    Heading("heading"),
    /// An app's screen titles.
    Display("display"),
    /// An app's prominent values: a stat, a name, a selected item.
    Value("value"),
    /// An app's clocks, counters and quantities: tabular figures.
    Numeric("numeric"),
}

/// A theme's text styles, one per [`TextRole`]: how each kind of text looks.
///
/// Index it by role, or [`set`](Self::set) a role to restyle it:
///
/// ```
/// use wgpu_gameui::{TextRole, TextStyle, Theme};
/// let mut theme = Theme::forge();
/// theme.typography.set(TextRole::Tab, TextStyle::display(15.0).with_italic(true));
/// assert_eq!(theme.typography[TextRole::Tab].size, 15.0);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Typography {
    styles: [TextStyle; TextRole::ALL.len()],
}

impl Typography {
    /// The style of `role`.
    pub fn get(&self, role: TextRole) -> TextStyle {
        self.styles[role as usize]
    }

    /// Restyle `role`.
    pub fn set(&mut self, role: TextRole, style: TextStyle) {
        self.styles[role as usize] = style;
    }

    /// The style of `role`, to change one property of it.
    pub fn style_mut(&mut self, role: TextRole) -> &mut TextStyle {
        &mut self.styles[role as usize]
    }

    /// Every role with its style, in [`TextRole::ALL`] order.
    pub fn iter(&self) -> impl Iterator<Item = (TextRole, TextStyle)> + '_ {
        TextRole::ALL.into_iter().zip(self.styles.iter().copied())
    }

    /// Multiply every role's size by `factor`, rounded to a multiple of
    /// `step` pixels (`0.0` for no rounding). A factor of 1 changes nothing,
    /// not even a size off the rounding grid.
    pub fn scale_sizes(&mut self, factor: f32, step: f32) {
        if factor == 1.0 {
            return;
        }
        for style in &mut self.styles {
            let size = style.size * factor;
            style.size = if step > 0.0 {
                (size / step).round() * step
            } else {
                size
            };
        }
    }

    /// Every role derived from a type scale, the way the built-in themes'
    /// widgets have always sized their text.
    pub(crate) fn from_scale(scale: &TypeScale) -> Self {
        let styles = TextRole::ALL.map(|role| scale.style(role));
        Self { styles }
    }
}

impl std::ops::Index<TextRole> for Typography {
    type Output = TextStyle;

    fn index(&self, role: TextRole) -> &TextStyle {
        &self.styles[role as usize]
    }
}

/// The handful of sizes and trackings a built-in theme's roles are derived
/// from: Forge's `typography.css` scale and its body and title sizes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TypeScale {
    pub body: f32,
    pub title: f32,
    pub caption: f32,
    pub meta: f32,
    pub dense: f32,
    pub row: f32,
    pub menu: f32,
    /// Letter spacing in em.
    pub badge_tracking: f32,
    pub property_tracking: f32,
    pub label_tracking: f32,
    pub section_tracking: f32,
    pub list_header_tracking: f32,
    pub badge_compact_tracking: f32,
}

impl TypeScale {
    /// Forge's scale.
    pub const FORGE: TypeScale = TypeScale {
        body: 12.0,
        title: 15.0,
        caption: 9.0,
        meta: 10.0,
        dense: 10.5,
        row: 11.0,
        menu: 11.5,
        badge_tracking: 0.08,
        property_tracking: 0.1,
        label_tracking: 0.14,
        section_tracking: 0.16,
        list_header_tracking: 0.12,
        badge_compact_tracking: 0.02,
    };

    /// The style of `role` on this scale. Every role is listed, so a new one
    /// cannot be added without saying how the built-in themes draw it.
    fn style(&self, role: TextRole) -> TextStyle {
        use TextRole::*;
        let caps = |tracking: f32| {
            TextStyle::mono(self.caption)
                .with_tracking(tracking)
                .with_caps(true)
        };
        // The leading each prose role's widget has always set.
        let prose = |style: TextStyle, leading: f32| style.with_line_height(leading);
        match role {
            Label => caps(self.label_tracking),
            Section => caps(self.section_tracking),
            Badge => caps(self.badge_tracking),
            BadgeCompact => {
                TextStyle::mono(self.caption).with_tracking(self.badge_compact_tracking)
            }
            Property => caps(self.property_tracking),
            ListHeader => caps(self.list_header_tracking),
            Flag => caps(0.0),
            Axis => TextStyle::mono(self.caption),
            Playhead => TextStyle::mono(self.caption).with_weight(Weight::SEMIBOLD),
            Meta => TextStyle::mono(self.meta),
            Detail => prose(TextStyle::mono(self.meta), 1.5),
            Code => TextStyle::mono(self.dense),
            Editor => TextStyle::mono(self.body),
            Hint => TextStyle::sans(self.meta),
            Help => prose(TextStyle::sans(self.dense), 1.5),
            Note => prose(TextStyle::sans(self.dense), 1.45),
            Dense => TextStyle::sans(self.dense),
            Row => TextStyle::sans(self.row),
            Message => prose(TextStyle::sans(self.row), 1.45),
            MenuItem | ListItem | SpanTab | Lead => TextStyle::sans(self.menu),
            ListItemMono => TextStyle::mono(self.menu),
            Description => prose(TextStyle::sans(self.menu), 1.55),
            MediaTitle => TextStyle::sans(self.menu).with_weight(Weight::MEDIUM),
            Subhead => TextStyle::sans(self.menu + 1.0).with_weight(Weight::SEMIBOLD),
            Body | Button | Control | FrameTitle => TextStyle::sans(self.body),
            Picker | DocTab => TextStyle::sans(self.body * 0.9),
            FieldCell | Chip | Keycap | Status | Pager | TipTitle | PanelTab => {
                TextStyle::sans(self.body * 0.85)
            }
            Readout | TileLabel | Tab => TextStyle::sans(self.body * 0.8),
            ProgressLabel | Cell => TextStyle::sans(self.body * 0.75),
            // The tooltip layer's lines are 3px apart, whatever the size.
            TipBody => {
                let size = self.body * 0.75;
                prose(TextStyle::sans(size), (size + 3.0) / size)
            }
            ColumnHeader => TextStyle::sans(self.body * 0.7),
            Title | Heading => TextStyle::display(self.title),
            DialogTitle => TextStyle::sans(self.title).with_weight(Weight::BOLD),
            Display => TextStyle::display(self.body * 2.0),
            Value => TextStyle::sans(self.body),
            Numeric => TextStyle::sans(self.body).with_figures(Figures::Tabular),
        }
    }
}

impl StyleOverlay {
    /// Restyle `role` for everything resolved through this overlay. Replaces
    /// any earlier style for `role`.
    pub fn set_text_style(&mut self, role: TextRole, style: TextStyle) -> &mut Self {
        if let Some(slot) = self.typography.iter_mut().find(|(r, _)| *r == role) {
            slot.1 = style;
        } else {
            self.typography.push((role, style));
        }
        self
    }

    /// The style this overlay gives `role`, if it restyles it.
    pub fn text_style(&self, role: TextRole) -> Option<TextStyle> {
        self.typography
            .iter()
            .find(|(r, _)| *r == role)
            .map(|&(_, style)| style)
    }
}

impl<'a> StyleResolver<'a> {
    /// How `role` looks: the overlay's style for it if it has one, else the
    /// theme's.
    pub fn text_style(&self, role: TextRole) -> TextStyle {
        self.overlay()
            .and_then(|overlay| overlay.text_style(role))
            .unwrap_or_else(|| self.theme().typography[role])
    }

    /// `role`'s size in pixels, for geometry derived from it.
    pub fn role_size(&self, role: TextRole) -> f32 {
        self.text_style(role).size
    }

    /// `role`'s line-box height in pixels, for geometry derived from it.
    pub fn role_line_height(&self, role: TextRole) -> f32 {
        self.text_style(role).line_height_px()
    }

    /// The theme's face for `family`; `None` is the default sans-serif. A
    /// theme without a mono or display face sets those in its sans.
    pub fn family_font(&self, family: FontFamily) -> Option<&'a FontHandle> {
        let theme = self.theme();
        match family {
            FontFamily::Sans => theme.font.as_ref(),
            FontFamily::Mono => theme.mono_font.as_ref().or(theme.font.as_ref()),
            FontFamily::Display => theme.display_font.as_ref().or(theme.font.as_ref()),
        }
    }

    /// The face `role` is set in (see [`family_font`](Self::family_font)).
    pub fn role_font(&self, role: TextRole) -> Option<&'a FontHandle> {
        self.family_font(self.text_style(role).family)
    }

    /// `content` as `role` draws it, at (`x`, `y`), in the colour of `ink`:
    /// the role's face, size, line height, weight, style, letter spacing and
    /// case. A colour outside the ink ladder is set on the returned block.
    pub fn role_block(
        &self,
        content: impl Into<TextContent>,
        x: f32,
        y: f32,
        role: TextRole,
        ink: Ink,
    ) -> TextBlock {
        self.style_block(content, x, y, self.text_style(role), ink)
    }

    /// [`role_block`](Self::role_block) at a `size` the widget computes from
    /// its geometry (a label sized to its bar). Everything else is the
    /// role's; letter spacing and line height follow the final size.
    pub fn role_block_sized(
        &self,
        content: impl Into<TextContent>,
        x: f32,
        y: f32,
        role: TextRole,
        ink: Ink,
        size: f32,
    ) -> TextBlock {
        self.style_block(content, x, y, self.text_style(role).with_size(size), ink)
    }

    /// `content` drawn in `style`, in the colour of `ink`. For a style no role
    /// holds, such as one derived from a role at run time.
    pub fn style_block(
        &self,
        content: impl Into<TextContent>,
        x: f32,
        y: f32,
        style: TextStyle,
        ink: Ink,
    ) -> TextBlock {
        let content = content.into();
        let content = if style.caps {
            upper_content(content)
        } else {
            content
        };
        let mut block = TextBlock::new(content, x, y)
            .with_size(style.size)
            .with_color_f32(self.ink(ink))
            .with_font_opt(self.family_font(style.family).cloned());
        block.line_height = style.line_height_px();
        block.letter_spacing = style.letter_spacing();
        block.weight = style.weight;
        block.style = style.font_style();
        block.figures = style.figures;
        block
    }

    /// The single-line width of `text` as [`role_block`](Self::role_block)
    /// draws it: in its face, weight, style, spacing and case. Allocates only
    /// to upper-case text a caps role changes.
    pub fn role_width(&self, list: &mut DrawList, text: &str, role: TextRole) -> f32 {
        self.style_width(list, text, self.text_style(role))
    }

    /// The single-line width of `text` drawn in `style`.
    pub fn style_width(&self, list: &mut DrawList, text: &str, style: TextStyle) -> f32 {
        let text = if style.caps {
            upper(text)
        } else {
            Cow::Borrowed(text)
        };
        let spec = LayoutSpec {
            line_height: style.line_height_px(),
            letter_spacing: style.letter_spacing(),
            font: self.family_font(style.family),
            weight: style.weight,
            style: style.font_style(),
            figures: style.figures,
            ..LayoutSpec::plain(style.size, None)
        };
        list.text_measurer_mut().measure_spec(&text, &spec).0
    }
}

/// Whether upper-casing `text` would leave it as it is.
fn is_upper(text: &str) -> bool {
    text.chars().all(|c| {
        let mut upper = c.to_uppercase();
        upper.next() == Some(c) && upper.next().is_none()
    })
}

/// `text` upper-cased, borrowed when it already is.
fn upper(text: &str) -> Cow<'_, str> {
    if is_upper(text) {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(text.to_uppercase())
    }
}

/// `content` upper-cased, sharing its text when it already is.
fn upper_content(content: TextContent) -> TextContent {
    if is_upper(&content) {
        content
    } else {
        content.to_uppercase().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_names_are_unique_and_resolve_back() {
        for role in TextRole::ALL {
            assert_eq!(TextRole::from_name(role.name()), Some(role));
        }
        let mut names: Vec<_> = TextRole::ALL.iter().map(|r| r.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), TextRole::ALL.len());
    }

    #[test]
    fn roles_index_in_declaration_order() {
        for (i, role) in TextRole::ALL.into_iter().enumerate() {
            assert_eq!(role as usize, i);
        }
    }

    #[test]
    fn forge_scale_reproduces_the_widget_sizes() {
        let t = Typography::from_scale(&TypeScale::FORGE);
        assert_eq!(t[TextRole::Tab].size, 12.0 * 0.8);
        assert_eq!(t[TextRole::Cell].size, 9.0);
        assert_eq!(t[TextRole::MenuItem].size, 11.5);
        let label = t[TextRole::Label];
        assert_eq!(label.family, FontFamily::Mono);
        assert!(label.caps);
        assert_eq!(label.letter_spacing(), 9.0 * 0.14);
        assert_eq!(t[TextRole::Title].family, FontFamily::Display);
        assert_eq!(t[TextRole::Numeric].figures, Figures::Tabular);
        assert_eq!(t[TextRole::Body].figures, Figures::Default);
        // Roles carrying a widget's own leading, weight or tracking.
        assert_eq!(t[TextRole::Help].line_height_px(), 10.5 * 1.5);
        assert_eq!(t[TextRole::Description].line_height_px(), 11.5 * 1.55);
        assert!((t[TextRole::TipBody].line_height_px() - 12.0).abs() < 1e-4);
        assert_eq!(t[TextRole::Subhead].size, 12.5);
        assert_eq!(t[TextRole::Subhead].weight, Weight::SEMIBOLD);
        assert_eq!(t[TextRole::DialogTitle].family, FontFamily::Sans);
        assert_eq!(t[TextRole::DialogTitle].weight, Weight::BOLD);
        assert!(!t[TextRole::BadgeCompact].caps);
        assert!(t[TextRole::Flag].caps);
        assert_eq!(t[TextRole::Flag].tracking, 0.0);
        assert_eq!(t[TextRole::ListHeader].letter_spacing(), 9.0 * 0.12);
        assert_eq!(t[TextRole::Keycap].size, 12.0 * 0.85);
    }

    #[test]
    fn set_and_iter_round_trip() {
        let mut t = Typography::from_scale(&TypeScale::FORGE);
        let style = TextStyle::display(15.0).with_italic(true);
        t.set(TextRole::Tab, style);
        assert_eq!(t.get(TextRole::Tab), style);
        assert_eq!(t.iter().count(), TextRole::ALL.len());
        assert_eq!(
            t.iter().find(|(r, _)| *r == TextRole::Tab),
            Some((TextRole::Tab, style))
        );
        t.style_mut(TextRole::Tab).caps = true;
        assert!(t[TextRole::Tab].caps);
    }

    #[test]
    fn scale_sizes_rounds_each_role() {
        let mut t = Typography::from_scale(&TypeScale::FORGE);
        let before = t.clone();
        t.scale_sizes(1.0, 0.5);
        // Identity, even for sizes off the half-pixel grid (tabs, 9.6).
        assert_eq!(t, before);
        t.scale_sizes(1.1, 0.5);
        // 9.6 × 1.1 = 10.56, rounded on its own (not 13 × 0.8 = 10.4).
        assert_eq!(t[TextRole::Tab].size, 10.5);
        assert_eq!(t[TextRole::Body].size, 13.0);
        assert_eq!(t[TextRole::Label].size, 10.0);
    }

    #[test]
    fn overlay_style_wins_for_its_role_only() {
        let theme = crate::Theme::forge();
        let mut overlay = StyleOverlay::new();
        let serif = TextStyle::display(15.0).with_italic(true);
        overlay.set_text_style(TextRole::Tab, serif);
        assert!(!overlay.is_empty());
        let s = StyleResolver::with_overlay(&theme, &overlay);
        assert_eq!(s.text_style(TextRole::Tab), serif);
        assert_eq!(
            s.text_style(TextRole::Cell),
            theme.typography[TextRole::Cell]
        );
        // Setting it again replaces it.
        overlay.set_text_style(TextRole::Tab, serif.with_size(20.0));
        let s = StyleResolver::with_overlay(&theme, &overlay);
        assert_eq!(s.role_size(TextRole::Tab), 20.0);
        overlay.clear();
        assert!(overlay.is_empty());
        assert_eq!(overlay.text_style(TextRole::Tab), None);
    }

    #[test]
    fn display_and_mono_fall_back_to_the_sans() {
        let sans = FontHandle("Sans Test".into());
        let display = FontHandle("Display Test".into());
        let mut theme = crate::Theme::forge();
        theme.font = Some(sans.clone());
        theme.mono_font = None;
        let s = StyleResolver::new(&theme);
        assert_eq!(s.family_font(FontFamily::Display), Some(&sans));
        assert_eq!(s.family_font(FontFamily::Mono), Some(&sans));
        assert_eq!(s.role_font(TextRole::Title), Some(&sans));
        theme.display_font = Some(display.clone());
        let s = StyleResolver::new(&theme);
        assert_eq!(s.role_font(TextRole::Title), Some(&display));
        assert_eq!(s.role_font(TextRole::Body), Some(&sans));
    }

    #[test]
    fn role_block_applies_the_whole_style() {
        let mut theme = crate::Theme::forge();
        let style = TextStyle::sans(10.0)
            .with_weight(Weight::BOLD)
            .with_italic(true)
            .with_tracking(0.2)
            .with_caps(true)
            .with_line_height(1.5)
            .with_figures(Figures::Proportional);
        theme.typography.set(TextRole::Button, style);
        let s = StyleResolver::new(&theme);
        let block = s.role_block("Save all", 3.0, 4.0, TextRole::Button, Ink::Max);
        assert_eq!(block.content.as_str(), "SAVE ALL");
        assert_eq!((block.x, block.y), (3.0, 4.0));
        assert_eq!(block.font_size, 10.0);
        assert_eq!(block.line_height, 15.0);
        assert_eq!(block.letter_spacing, 2.0);
        assert_eq!(block.weight, Weight::BOLD);
        assert_eq!(block.style, cosmic_text::Style::Italic);
        assert_eq!(block.figures, Figures::Proportional);
        assert_eq!(block.font, theme.font);
        let sized = s.role_block_sized("x", 0.0, 0.0, TextRole::Button, Ink::Max, 20.0);
        assert_eq!(sized.font_size, 20.0);
        assert_eq!(sized.letter_spacing, 4.0);
        assert_eq!(sized.line_height, 30.0);
    }

    #[test]
    fn caps_shares_text_that_is_already_upper_case() {
        let content = TextContent::from("FPS 60");
        let cased = upper_content(content.clone());
        assert!(std::ptr::eq(content.as_str(), cased.as_str()));
        assert_eq!(upper_content("straße".into()).as_str(), "STRASSE");
        assert!(matches!(upper("OK"), Cow::Borrowed(_)));
    }

    #[cfg(feature = "bundled-font")]
    #[test]
    fn role_width_matches_the_drawn_block_in_its_own_face() {
        let mut theme = crate::Theme::forge();
        theme.display_font = theme.mono_font.clone();
        theme.typography.set(
            TextRole::Title,
            TextStyle::display(13.0).with_tracking(0.1).with_caps(true),
        );
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::with_font_system(crate::shared_font_system());
        let text = "Inspector";
        let block = s.role_block(text, 0.0, 0.0, TextRole::Title, Ink::Max);
        let drawn = list.measure_block(&block).0;
        let measured = s.role_width(&mut list, text, TextRole::Title);
        assert!((drawn - measured).abs() < 0.01, "{drawn} vs {measured}");
        // The default sans would measure it differently.
        let sans = list.measure_text(&text.to_uppercase(), 13.0, None).0;
        assert!((sans - measured).abs() > 0.5, "{sans} vs {measured}");
    }

    #[test]
    fn style_width_measures_the_styles_figures() {
        let fs = crate::shared_font_system();
        let mut theme = crate::Theme::forge();
        theme.font = Some(crate::test_fonts::spectral(&fs));
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::with_font_system(fs);
        let style = TextStyle::sans(20.0);
        let width =
            |list: &mut DrawList, figures| s.style_width(list, "1111", style.with_figures(figures));
        let proportional = width(&mut list, Figures::Proportional);
        let tabular = width(&mut list, Figures::Tabular);
        assert!(tabular - proportional > 1.0, "{proportional} vs {tabular}");
        let block = s.style_block(
            "1111",
            0.0,
            0.0,
            style.with_figures(Figures::Proportional),
            Ink::Max,
        );
        assert!((list.measure_block(&block).0 - proportional).abs() < 0.01);
    }

    #[test]
    fn style_line_height_and_spacing_follow_size() {
        let s = TextStyle::sans(10.0)
            .with_tracking(0.1)
            .with_line_height(1.5);
        assert_eq!(s.line_height_px(), 15.0);
        assert_eq!(s.letter_spacing(), 1.0);
        assert_eq!(s.with_size(20.0).letter_spacing(), 2.0);
    }
}
