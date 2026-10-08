//! Theme presets: a handful of seeds that re-skin the default (Forge) theme,
//! plus per-token overrides. This is what the theme designer edits and
//! exports, and what an app loads to look like something other than Forge.
//!
//! [`ThemePreset::resolve`] builds a [`Theme`]: it starts from
//! `Theme::default()`, lets [`ThemeSeeds::apply`] move every colour and size,
//! then writes the overrides. With the `preset` feature a preset also reads
//! from and writes to TOML ([`ThemePreset::from_toml`],
//! [`ThemePreset::to_toml`]).
//!
//! # How seeds move colours
//!
//! Forge gives each meaning one hue and lets lightness and chroma name the
//! role. So every Forge colour belongs to a [`ColorFamily`], told apart by its
//! OKLCH coordinates: below [`NEUTRAL_CHROMA`] it is a neutral (a surface, or
//! ink if it is a text role), above it the family whose Forge hue is nearest.
//! Pure white and black — the sheens, edges and shadows — belong to no family
//! and stay as they are.
//!
//! A family's [`ColorSeed`] rotates the hue by `seed.hue - forge_hue`,
//! multiplies the chroma and offsets the lightness, keeping alpha. A seed
//! equal to [`ColorSeed::forge`] leaves its colours untouched, so the default
//! seeds give exactly `Theme::default()`.
//!
//! The design and its open gaps: `docs/design/theme-designer.md`.

use crate::Theme;
use crate::color::{HUE_ACCENT, HUE_DANGER, HUE_OK, HUE_WARN, oklch, to_oklch, to_rgba8};
use crate::shadow::CornerRadii;
use crate::style::{StyleKey, StyleShape, StyleValue};
use crate::text::FontHandle;

/// Below this OKLCH chroma a colour counts as neutral. Forge's greys measure
/// 0.005–0.018; its faintest tinted colour, `on_danger`, 0.021.
pub const NEUTRAL_CHROMA: f32 = 0.02;

/// Below this chroma a colour is pure white, black or grey: light and shade,
/// which no seed tints.
const ACHROMATIC_CHROMA: f32 = 0.001;

/// The hue Forge's neutrals lean towards (a cool blue-grey).
const HUE_NEUTRAL: f32 = 248.0;

/// The hue of Forge's info severity (`#5eaceb`).
const HUE_INFO: f32 = 245.0;

/// A group of colours one [`ColorSeed`] moves together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorFamily {
    /// Neutral surfaces, borders and washes.
    Surface,
    /// Neutral text and glyph colours: the ink ladder and the `text*` keys.
    Ink,
    /// The accent: selection, focus, toggles, primary keys.
    Accent,
    /// Destructive actions and errors.
    Danger,
    /// Warnings and things waiting on the user.
    Warning,
    /// Success and healthy states.
    Success,
    /// Informational severity.
    Info,
}

impl ColorFamily {
    /// Every family, in the order a designer lists them.
    pub const ALL: [ColorFamily; 7] = [
        ColorFamily::Surface,
        ColorFamily::Ink,
        ColorFamily::Accent,
        ColorFamily::Danger,
        ColorFamily::Warning,
        ColorFamily::Success,
        ColorFamily::Info,
    ];

    /// The family's hue in the Forge theme, in degrees.
    pub const fn forge_hue(self) -> f32 {
        match self {
            ColorFamily::Surface | ColorFamily::Ink => HUE_NEUTRAL,
            ColorFamily::Accent => HUE_ACCENT,
            ColorFamily::Danger => HUE_DANGER,
            ColorFamily::Warning => HUE_WARN,
            ColorFamily::Success => HUE_OK,
            ColorFamily::Info => HUE_INFO,
        }
    }

    /// The family's name in a preset file (`[seeds.<name>]`).
    pub const fn name(self) -> &'static str {
        match self {
            ColorFamily::Surface => "surface",
            ColorFamily::Ink => "ink",
            ColorFamily::Accent => "accent",
            ColorFamily::Danger => "danger",
            ColorFamily::Warning => "warning",
            ColorFamily::Success => "success",
            ColorFamily::Info => "info",
        }
    }

    /// The family a colour belongs to; `text` says whether it is a text role,
    /// which decides between [`Surface`](Self::Surface) and
    /// [`Ink`](Self::Ink) for a neutral. `None` for pure white, black and
    /// grey, which no seed moves.
    pub fn classify(color: [f32; 4], text: bool) -> Option<ColorFamily> {
        let [_, chroma, hue] = to_oklch(color);
        if chroma < ACHROMATIC_CHROMA {
            return None;
        }
        if chroma < NEUTRAL_CHROMA {
            return Some(if text {
                ColorFamily::Ink
            } else {
                ColorFamily::Surface
            });
        }
        let distance = |family: ColorFamily| {
            let d = (hue - family.forge_hue()).rem_euclid(360.0);
            d.min(360.0 - d)
        };
        [
            ColorFamily::Accent,
            ColorFamily::Danger,
            ColorFamily::Warning,
            ColorFamily::Success,
            ColorFamily::Info,
        ]
        .into_iter()
        .min_by(|a, b| distance(*a).total_cmp(&distance(*b)))
    }

    /// The family of a colour key's value in `template` (normally
    /// `Theme::default()`); `None` for scalar and custom keys and for
    /// achromatic colours.
    pub fn of_key(key: StyleKey, template: &Theme) -> Option<ColorFamily> {
        let color = template.get(key)?.as_color()?;
        Self::classify(color, is_text_key(key))
    }
}

/// Whether a key is a text role: its neutral value is ink, not a surface.
fn is_text_key(key: StyleKey) -> bool {
    matches!(
        key,
        StyleKey::Ink(_) | StyleKey::Text | StyleKey::TextDim | StyleKey::TextHighlight
    )
}

/// How one [`ColorFamily`] moves away from Forge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorSeed {
    /// The family's hue, in degrees. Its colours rotate by the difference to
    /// [`ColorFamily::forge_hue`], so the spread inside the family survives.
    pub hue: f32,
    /// Multiplier on each colour's chroma; 1 keeps Forge's. For the neutral
    /// families it sets the strength of the tint, and 0 makes them pure grey.
    pub chroma: f32,
    /// Offset added to each colour's OKLCH lightness; 0 keeps Forge's.
    pub lightness: f32,
}

impl ColorSeed {
    /// The seed that leaves `family` exactly as Forge has it.
    pub const fn forge(family: ColorFamily) -> Self {
        Self {
            hue: family.forge_hue(),
            chroma: 1.0,
            lightness: 0.0,
        }
    }

    /// Move one colour of `family` by this seed. A seed equal to
    /// [`forge`](Self::forge) returns `color` unchanged, bit for bit.
    pub fn apply(self, family: ColorFamily, color: [f32; 4]) -> [f32; 4] {
        if self == Self::forge(family) {
            return color;
        }
        let [l, c, h] = to_oklch(color);
        oklch(
            (l + self.lightness).clamp(0.0, 1.0),
            c * self.chroma.max(0.0),
            h + self.hue - family.forge_hue(),
            color[3],
        )
    }
}

/// Sizes [`ThemeSeeds::density`] scales: paddings, gaps, control and row
/// heights, toolbar and dock metrics.
const DENSITY_KEYS: [StyleKey; 14] = [
    StyleKey::Padding,
    StyleKey::Spacing,
    StyleKey::ButtonHeight,
    StyleKey::InputHeight,
    StyleKey::MenuBarHeight,
    StyleKey::MenuRowHeight,
    StyleKey::MenuItemMinWidth,
    StyleKey::MenuAccelGap,
    StyleKey::ToolbarButtonSize,
    StyleKey::ToolbarPadding,
    StyleKey::DockTabHeight,
    StyleKey::DockSplitterWidth,
    StyleKey::WindowTitleHeight,
    StyleKey::ListRowHeight,
];

/// The few values a preset derives the whole theme from.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeSeeds {
    /// Neutral surfaces, borders and washes.
    pub surface: ColorSeed,
    /// Neutral text and glyphs.
    pub ink: ColorSeed,
    /// The accent.
    pub accent: ColorSeed,
    /// Destructive actions and errors.
    pub danger: ColorSeed,
    /// Warnings.
    pub warning: ColorSeed,
    /// Success.
    pub success: ColorSeed,
    /// Informational severity.
    pub info: ColorSeed,
    /// Multiplier on the geometry: paddings, gaps, control and row heights,
    /// toolbar and dock metrics. Results round to whole pixels.
    pub density: f32,
    /// Multiplier on the type sizes (`font_size`, `font_size_title` and the
    /// [`TextSize`](crate::TextSize) scale). Results round to half pixels.
    pub text_scale: f32,
    /// Multiplier on `border_radius` and every chrome corner radius.
    pub radius_scale: f32,
    /// Family of the UI font ([`Theme::font`]); `None` keeps the default. The
    /// app has to have loaded it.
    pub sans_font: Option<String>,
    /// Family of the technical font ([`Theme::mono_font`]); `None` keeps the
    /// default.
    pub mono_font: Option<String>,
}

impl Default for ThemeSeeds {
    /// Forge's seeds: applying them changes nothing.
    fn default() -> Self {
        Self {
            surface: ColorSeed::forge(ColorFamily::Surface),
            ink: ColorSeed::forge(ColorFamily::Ink),
            accent: ColorSeed::forge(ColorFamily::Accent),
            danger: ColorSeed::forge(ColorFamily::Danger),
            warning: ColorSeed::forge(ColorFamily::Warning),
            success: ColorSeed::forge(ColorFamily::Success),
            info: ColorSeed::forge(ColorFamily::Info),
            density: 1.0,
            text_scale: 1.0,
            radius_scale: 1.0,
            sans_font: None,
            mono_font: None,
        }
    }
}

impl ThemeSeeds {
    /// The seed of `family`.
    pub fn color(&self, family: ColorFamily) -> ColorSeed {
        match family {
            ColorFamily::Surface => self.surface,
            ColorFamily::Ink => self.ink,
            ColorFamily::Accent => self.accent,
            ColorFamily::Danger => self.danger,
            ColorFamily::Warning => self.warning,
            ColorFamily::Success => self.success,
            ColorFamily::Info => self.info,
        }
    }

    /// The seed of `family`, to edit.
    pub fn color_mut(&mut self, family: ColorFamily) -> &mut ColorSeed {
        match family {
            ColorFamily::Surface => &mut self.surface,
            ColorFamily::Ink => &mut self.ink,
            ColorFamily::Accent => &mut self.accent,
            ColorFamily::Danger => &mut self.danger,
            ColorFamily::Warning => &mut self.warning,
            ColorFamily::Success => &mut self.success,
            ColorFamily::Info => &mut self.info,
        }
    }

    /// Re-skin `theme`, which should hold Forge's values (`Theme::default()`):
    /// every colour moves by its family's seed, the sizes by the scales, and
    /// the fonts are replaced if named. Custom style values are left alone.
    pub fn apply(&self, theme: &mut Theme) {
        for key in StyleKey::builtin().filter(|key| key.is_color()) {
            let Some(color) = theme.get(key).and_then(StyleValue::as_color) else {
                continue;
            };
            if let Some(family) = ColorFamily::classify(color, is_text_key(key)) {
                let moved = self.color(family).apply(family, color);
                theme.set(key, StyleValue::Color(moved));
            }
        }
        theme.chrome.map_colors(|color| {
            if let Some(family) = ColorFamily::classify(*color, false) {
                *color = self.color(family).apply(family, *color);
            }
        });

        if self.density != 1.0 {
            for key in DENSITY_KEYS {
                scale_scalar(theme, key, self.density, 1.0);
            }
        }
        if self.text_scale != 1.0 {
            let text_keys = [StyleKey::FontSize, StyleKey::FontSizeTitle]
                .into_iter()
                .chain(crate::TextSize::ALL.map(StyleKey::TextSize));
            for key in text_keys {
                scale_scalar(theme, key, self.text_scale, 0.5);
            }
        }
        if self.radius_scale != 1.0 {
            let k = self.radius_scale.max(0.0);
            theme.border_radius *= k;
            theme.chrome.map_corner_radii(|radii| {
                *radii = CornerRadii::new(
                    radii.top_left * k,
                    radii.top_right * k,
                    radii.bottom_right * k,
                    radii.bottom_left * k,
                );
            });
        }

        if let Some(family) = &self.sans_font {
            theme.font = Some(FontHandle(family.clone()));
        }
        if let Some(family) = &self.mono_font {
            theme.mono_font = Some(FontHandle(family.clone()));
        }
    }
}

/// Multiply a scalar key by `factor`, rounded to a multiple of `step`.
fn scale_scalar(theme: &mut Theme, key: StyleKey, factor: f32, step: f32) {
    if let Some(value) = theme.get(key).and_then(StyleValue::as_scalar) {
        let scaled = (value * factor.max(0.0) / step).round() * step;
        theme.set(key, StyleValue::Scalar(scaled));
    }
}

/// Quantise a colour to 8 bits per channel, the precision a preset file
/// stores, so a value in memory is the value that will load back.
fn quantise(color: [f32; 4]) -> [f32; 4] {
    to_rgba8(color).map(|channel| channel as f32 / 255.0)
}

/// Seeds plus per-token overrides: a whole theme in a few dozen values.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemePreset {
    /// Display name.
    pub name: String,
    /// What every colour and size derives from.
    pub seeds: ThemeSeeds,
    /// Values that replace what the seeds derive, in the order they were set.
    overrides: Vec<(StyleKey, StyleValue)>,
}

impl Default for ThemePreset {
    fn default() -> Self {
        Self::forge()
    }
}

impl ThemePreset {
    /// The Forge theme as a preset: default seeds, no overrides.
    pub fn forge() -> Self {
        Self::new("Forge")
    }

    /// A preset called `name` that starts out as Forge.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            seeds: ThemeSeeds::default(),
            overrides: Vec::new(),
        }
    }

    /// Build the theme: Forge, re-skinned by the seeds, then the overrides.
    pub fn resolve(&self) -> Theme {
        let mut theme = Theme::default();
        self.seeds.apply(&mut theme);
        for &(key, value) in &self.overrides {
            theme.set(key, value);
        }
        theme
    }

    /// The overrides, in the order they were set.
    pub fn overrides(&self) -> &[(StyleKey, StyleValue)] {
        &self.overrides
    }

    /// The override of `key`, if it has one.
    pub fn override_of(&self, key: StyleKey) -> Option<StyleValue> {
        self.overrides
            .iter()
            .find(|(k, _)| *k == key)
            .map(|&(_, value)| value)
    }

    /// Override `key` with `value`, replacing an earlier override. Colours
    /// are kept at 8 bits per channel, as the file stores them.
    ///
    /// # Panics
    ///
    /// If `key` is a custom key (a preset names keys, and a custom key only
    /// keeps a hash) or `value` is not the key's shape.
    pub fn set_override(&mut self, key: StyleKey, value: StyleValue) {
        let value = match (key.shape(), value) {
            (Some(StyleShape::Color), StyleValue::Color(color)) => {
                StyleValue::Color(quantise(color))
            }
            (Some(StyleShape::Scalar), StyleValue::Scalar(_)) => value,
            _ => panic!("{value:?} cannot override {key:?}"),
        };
        match self.overrides.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = value,
            None => self.overrides.push((key, value)),
        }
    }

    /// Drop the override of `key`; returns whether there was one.
    pub fn clear_override(&mut self, key: StyleKey) -> bool {
        let before = self.overrides.len();
        self.overrides.retain(|(k, _)| *k != key);
        self.overrides.len() != before
    }
}

#[cfg(feature = "preset")]
mod file {
    //! The TOML form of a [`ThemePreset`]. Written and read by hand rather
    //! than through serde derives, so every unknown key is an error that names
    //! its path and the output keeps a fixed, readable order.

    use std::fmt::Write as _;

    use super::{ColorFamily, ColorSeed, ThemePreset, ThemeSeeds};
    use crate::color::{oklch, to_rgba8};
    use crate::style::{StyleKey, StyleShape, StyleValue};

    /// The preset file format this build reads and writes.
    pub const PRESET_FORMAT: i64 = 1;

    /// Why a preset file could not be read.
    #[derive(Clone, Debug, PartialEq)]
    pub struct PresetError(String);

    impl std::fmt::Display for PresetError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for PresetError {}

    fn error(message: impl Into<String>) -> PresetError {
        PresetError(message.into())
    }

    fn number(value: &toml::Value, path: &str) -> Result<f32, PresetError> {
        match value {
            toml::Value::Float(f) => Ok(*f as f32),
            toml::Value::Integer(i) => Ok(*i as f32),
            other => Err(error(format!(
                "`{path}` should be a number, not {}",
                other.type_str()
            ))),
        }
    }

    fn string<'a>(value: &'a toml::Value, path: &str) -> Result<&'a str, PresetError> {
        value.as_str().ok_or_else(|| {
            error(format!(
                "`{path}` should be a string, not {}",
                value.type_str()
            ))
        })
    }

    fn table<'a>(value: &'a toml::Value, path: &str) -> Result<&'a toml::Table, PresetError> {
        value.as_table().ok_or_else(|| {
            error(format!(
                "`{path}` should be a table, not {}",
                value.type_str()
            ))
        })
    }

    /// Parse `#rrggbb`, `#rrggbbaa`, `oklch(L C H)` or `oklch(L C H / a)`.
    pub(super) fn parse_color(text: &str) -> Option<[f32; 4]> {
        let text = text.trim();
        if let Some(hex) = text.strip_prefix('#') {
            if !matches!(hex.len(), 6 | 8) || !hex.is_ascii() {
                return None;
            }
            let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
            let alpha = if hex.len() == 8 { channel(6)? } else { 255 };
            return Some([channel(0)?, channel(2)?, channel(4)?, alpha].map(|c| c as f32 / 255.0));
        }
        let inner = text.strip_prefix("oklch(")?.strip_suffix(')')?;
        let (lch, alpha) = match inner.split_once('/') {
            Some((lch, alpha)) => (lch, alpha.trim().parse().ok()?),
            None => (inner, 1.0),
        };
        let mut parts = lch.split_whitespace().map(str::parse::<f32>);
        let (l, c, h) = (
            parts.next()?.ok()?,
            parts.next()?.ok()?,
            parts.next()?.ok()?,
        );
        if parts.next().is_some() {
            return None;
        }
        Some(oklch(l, c, h, alpha))
    }

    fn format_color(color: [f32; 4]) -> String {
        let [r, g, b, a] = to_rgba8(color);
        if a == 255 {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
        }
    }

    /// A float as TOML, in the shortest form that reads back as the same
    /// `f32` (`0.1`, not `0.10000000149011612`).
    fn format_number(value: f32) -> String {
        let shortest = format!("{value}").parse::<f64>().unwrap_or(value as f64);
        toml::Value::Float(shortest).to_string()
    }

    fn format_string(value: &str) -> String {
        toml::Value::String(value.to_owned()).to_string()
    }

    fn parse_color_seed(
        value: &toml::Value,
        family: ColorFamily,
        path: &str,
    ) -> Result<ColorSeed, PresetError> {
        let mut seed = ColorSeed::forge(family);
        for (key, value) in table(value, path)? {
            let field = match key.as_str() {
                "hue" => &mut seed.hue,
                "chroma" => &mut seed.chroma,
                "lightness" => &mut seed.lightness,
                _ => return Err(error(format!("unknown key `{path}.{key}`"))),
            };
            *field = number(value, &format!("{path}.{key}"))?;
        }
        Ok(seed)
    }

    fn parse_seeds(value: &toml::Value) -> Result<ThemeSeeds, PresetError> {
        let mut seeds = ThemeSeeds::default();
        for (key, value) in table(value, "seeds")? {
            let path = format!("seeds.{key}");
            match key.as_str() {
                "density" => seeds.density = number(value, &path)?,
                "text_scale" => seeds.text_scale = number(value, &path)?,
                "radius_scale" => seeds.radius_scale = number(value, &path)?,
                "sans_font" => seeds.sans_font = Some(string(value, &path)?.to_owned()),
                "mono_font" => seeds.mono_font = Some(string(value, &path)?.to_owned()),
                name => {
                    let family = ColorFamily::ALL
                        .into_iter()
                        .find(|family| family.name() == name)
                        .ok_or_else(|| error(format!("unknown key `{path}`")))?;
                    *seeds.color_mut(family) = parse_color_seed(value, family, &path)?;
                }
            }
        }
        Ok(seeds)
    }

    fn parse_overrides(value: &toml::Value, preset: &mut ThemePreset) -> Result<(), PresetError> {
        for (name, value) in table(value, "overrides")? {
            let path = format!("overrides.{name}");
            let key = StyleKey::from_name(name)
                .ok_or_else(|| error(format!("unknown style key `{path}`")))?;
            let value = match key.shape() {
                Some(StyleShape::Color) => {
                    let text = string(value, &path)?;
                    StyleValue::Color(parse_color(text).ok_or_else(|| {
                        error(format!(
                            "`{path}` = {text:?} is not a colour (`#rrggbb`, `#rrggbbaa` \
                             or `oklch(L C H / a)`)"
                        ))
                    })?)
                }
                _ => StyleValue::Scalar(number(value, &path)?),
            };
            preset.set_override(key, value);
        }
        Ok(())
    }

    impl ThemePreset {
        /// Read a preset from TOML. Unknown keys, wrong value types and a
        /// `format` other than [`PRESET_FORMAT`] are errors; seeds that are
        /// left out keep Forge's values.
        pub fn from_toml(text: &str) -> Result<ThemePreset, PresetError> {
            let document: toml::Table = toml::from_str(text).map_err(|e| error(e.to_string()))?;
            let mut preset = ThemePreset::new("");
            let mut format = None;
            for (key, value) in &document {
                match key.as_str() {
                    "format" => format = Some(value),
                    "name" => preset.name = string(value, "name")?.to_owned(),
                    "seeds" => preset.seeds = parse_seeds(value)?,
                    "overrides" => parse_overrides(value, &mut preset)?,
                    _ => return Err(error(format!("unknown key `{key}`"))),
                }
            }
            match format {
                Some(toml::Value::Integer(PRESET_FORMAT)) => Ok(preset),
                Some(other) => Err(error(format!(
                    "preset format {other} is not supported; this build reads format \
                     {PRESET_FORMAT}"
                ))),
                None => Err(error("missing `format`")),
            }
        }

        /// Write the preset as TOML: every seed, then the overrides in
        /// [`StyleKey::builtin`] order.
        pub fn to_toml(&self) -> String {
            let mut out = String::new();
            let seeds = &self.seeds;
            // `write!` into a `String` cannot fail.
            let _ = writeln!(out, "format = {PRESET_FORMAT}");
            let _ = writeln!(out, "name = {}", format_string(&self.name));
            let _ = writeln!(out, "\n[seeds]");
            let _ = writeln!(out, "density = {}", format_number(seeds.density));
            let _ = writeln!(out, "text_scale = {}", format_number(seeds.text_scale));
            let _ = writeln!(out, "radius_scale = {}", format_number(seeds.radius_scale));
            if let Some(font) = &seeds.sans_font {
                let _ = writeln!(out, "sans_font = {}", format_string(font));
            }
            if let Some(font) = &seeds.mono_font {
                let _ = writeln!(out, "mono_font = {}", format_string(font));
            }
            for family in ColorFamily::ALL {
                let seed = seeds.color(family);
                let _ = writeln!(out, "\n[seeds.{}]", family.name());
                let _ = writeln!(out, "hue = {}", format_number(seed.hue));
                let _ = writeln!(out, "chroma = {}", format_number(seed.chroma));
                let _ = writeln!(out, "lightness = {}", format_number(seed.lightness));
            }
            let _ = writeln!(out, "\n[overrides]");
            for key in StyleKey::builtin() {
                let Some(value) = self.override_of(key) else {
                    continue;
                };
                let name = key.name();
                let name = if name.contains('.') {
                    format_string(name)
                } else {
                    name.to_owned()
                };
                let value = match value {
                    StyleValue::Color(color) => format_string(&format_color(color)),
                    StyleValue::Scalar(scalar) => format_number(scalar),
                };
                let _ = writeln!(out, "{name} = {value}");
            }
            out
        }
    }
}

#[cfg(feature = "preset")]
pub use file::{PRESET_FORMAT, PresetError};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::hex;

    /// Every keyed value, the chrome and the fonts of two themes agree.
    fn assert_same_theme(a: &Theme, b: &Theme) {
        for key in StyleKey::builtin() {
            assert_eq!(a.get(key), b.get(key), "{key:?}");
        }
        assert_eq!(a.chrome, b.chrome);
        assert_eq!(a.font, b.font);
        assert_eq!(a.mono_font, b.mono_font);
    }

    fn lch(theme: &Theme, key: StyleKey) -> [f32; 3] {
        to_oklch(theme.get(key).unwrap().as_color().unwrap())
    }

    #[test]
    fn forge_seeds_resolve_to_the_default_theme_exactly() {
        assert_same_theme(&ThemePreset::forge().resolve(), &Theme::default());
    }

    #[test]
    fn forge_colours_fall_into_their_families() {
        let t = Theme::default();
        let family = |key| ColorFamily::of_key(key, &t);
        use ColorFamily::*;
        for (key, want) in [
            (StyleKey::Accent, Some(Accent)),
            (StyleKey::AccentFaceBottom, Some(Accent)),
            (StyleKey::OnAccent, Some(Accent)),
            (StyleKey::Ink(crate::Ink::OnAccentSecond), Some(Accent)),
            (StyleKey::DangerFaceTop, Some(Danger)),
            (StyleKey::OnDanger, Some(Danger)),
            (StyleKey::Error, Some(Danger)),
            (StyleKey::Warning, Some(Warning)),
            (StyleKey::ProgressFillMedium, Some(Warning)),
            (StyleKey::WarnSoftInk, Some(Warning)),
            (StyleKey::StatusOk, Some(Success)),
            (StyleKey::Success, Some(Success)),
            (StyleKey::Info, Some(Info)),
            (StyleKey::Background, Some(Surface)),
            (StyleKey::Panel, Some(Surface)),
            (StyleKey::Button, Some(Surface)),
            (StyleKey::Text, Some(Ink)),
            (StyleKey::Ink(crate::Ink::Caption), Some(Ink)),
            (StyleKey::Ink(crate::Ink::Empty), Some(Ink)),
            // Light and shade stay put.
            (StyleKey::FaceTop, None),
            (StyleKey::EdgeHard, None),
            (StyleKey::Scrim, None),
            (StyleKey::Ink(crate::Ink::White), None),
            // Scalars have no family.
            (StyleKey::Padding, None),
        ] {
            assert_eq!(family(key), want, "{key:?}");
        }
    }

    #[test]
    fn an_accent_seed_rotates_the_accent_and_nothing_else() {
        let mut preset = ThemePreset::forge();
        preset.seeds.accent.hue = 140.0;
        let theme = preset.resolve();
        let forge = Theme::default();
        let [l, c, h] = lch(&theme, StyleKey::Accent);
        let [l0, c0, _] = lch(&forge, StyleKey::Accent);
        assert!((h - 140.0).abs() < 0.5, "hue {h}");
        assert!((l - l0).abs() < 1e-3 && (c - c0).abs() < 1e-3);
        // The spread inside the family survives: accent_tick sat at the same
        // hue as the accent in Forge, and still does.
        assert!((lch(&theme, StyleKey::AccentTick)[2] - 140.0).abs() < 0.5);
        // The chrome's accent colours move too.
        assert_ne!(
            theme.chrome.splitter.grip_dragging,
            forge.chrome.splitter.grip_dragging
        );
        for key in [
            StyleKey::Panel,
            StyleKey::Text,
            StyleKey::DangerFaceTop,
            StyleKey::Warning,
            StyleKey::FaceTop,
        ] {
            assert_eq!(theme.get(key), forge.get(key), "{key:?}");
        }
    }

    #[test]
    fn a_surface_seed_moves_surfaces_but_not_ink() {
        let mut preset = ThemePreset::forge();
        preset.seeds.surface.lightness = 0.05;
        let theme = preset.resolve();
        let forge = Theme::default();
        let panel = lch(&theme, StyleKey::Panel)[0];
        assert!((panel - (lch(&forge, StyleKey::Panel)[0] + 0.05)).abs() < 1e-3);
        assert_eq!(theme.panel[3], forge.panel[3], "alpha is kept");
        assert_eq!(theme.text, forge.text);
        assert_eq!(theme.ink, forge.ink);
        assert_ne!(theme.chrome.menu_bar.surface, forge.chrome.menu_bar.surface);
    }

    #[test]
    fn zero_chroma_makes_the_neutrals_grey() {
        let mut preset = ThemePreset::forge();
        preset.seeds.surface.chroma = 0.0;
        preset.seeds.ink.chroma = 0.0;
        let theme = preset.resolve();
        for key in [StyleKey::Panel, StyleKey::Button, StyleKey::Text] {
            assert!(lch(&theme, key)[1] < 1e-3, "{key:?} is still tinted");
        }
    }

    #[test]
    fn scales_move_geometry_type_and_radii() {
        let mut preset = ThemePreset::forge();
        preset.seeds.density = 1.5;
        preset.seeds.text_scale = 1.25;
        preset.seeds.radius_scale = 3.0;
        let theme = preset.resolve();
        let forge = Theme::default();
        assert_eq!(theme.button_height, 39.0);
        assert_eq!(theme.list_row_height, 33.0);
        // 5 × 1.5 = 7.5 rounds to a whole pixel.
        assert_eq!(theme.padding, 8.0);
        assert_eq!(theme.border_width, forge.border_width, "not geometry");
        assert_eq!(theme.font_size, 15.0);
        // 11.5 × 1.25 = 14.375 rounds to a half pixel.
        assert_eq!(theme.text_sizes[4], 14.5);
        assert_eq!(theme.border_radius, 3.0);
        assert_eq!(
            theme.chrome.dock.active_tab.corner_radii.top_left,
            forge.chrome.dock.active_tab.corner_radii.top_left * 3.0
        );
    }

    #[test]
    fn fonts_are_replaced_when_named() {
        let mut preset = ThemePreset::forge();
        preset.seeds.sans_font = Some("Alegreya Sans".into());
        let theme = preset.resolve();
        assert_eq!(theme.font, Some(FontHandle("Alegreya Sans".into())));
        assert_eq!(theme.mono_font, Theme::default().mono_font);
    }

    #[test]
    fn overrides_win_over_seeds_and_replace_each_other() {
        let mut preset = ThemePreset::forge();
        preset.seeds.accent.hue = 140.0;
        preset.set_override(StyleKey::Accent, StyleValue::Color(hex(0x93c264)));
        preset.set_override(StyleKey::Padding, StyleValue::Scalar(9.0));
        preset.set_override(StyleKey::Padding, StyleValue::Scalar(7.0));
        assert_eq!(preset.overrides().len(), 2);
        let theme = preset.resolve();
        assert_eq!(theme.accent, hex(0x93c264));
        assert_eq!(theme.padding, 7.0);
        assert!(preset.clear_override(StyleKey::Padding));
        assert!(!preset.clear_override(StyleKey::Padding));
        assert_eq!(preset.resolve().padding, Theme::default().padding);
    }

    #[test]
    fn colour_overrides_are_kept_at_eight_bits() {
        let mut preset = ThemePreset::forge();
        preset.set_override(StyleKey::Scrim, StyleValue::Color([0.0, 0.0, 0.0, 0.55]));
        let alpha = preset
            .override_of(StyleKey::Scrim)
            .unwrap()
            .as_color()
            .unwrap()[3];
        assert_eq!(alpha, 140.0 / 255.0);
    }

    #[test]
    #[should_panic]
    fn a_scalar_cannot_override_a_colour() {
        ThemePreset::forge().set_override(StyleKey::Accent, StyleValue::Scalar(1.0));
    }

    #[cfg(feature = "preset")]
    mod toml_file {
        use super::*;

        fn aldermoor() -> ThemePreset {
            let mut preset = ThemePreset::new("Aldermoor \"proof\"");
            preset.seeds.surface = ColorSeed {
                hue: 60.0,
                chroma: 1.5,
                lightness: 0.04,
            };
            preset.seeds.accent.hue = 135.0;
            preset.seeds.accent.lightness = -0.05;
            preset.seeds.density = 1.1;
            preset.seeds.radius_scale = 3.0;
            preset.seeds.sans_font = Some("Alegreya Sans".into());
            preset.set_override(StyleKey::AccentFaceTop, StyleValue::Color(hex(0x93c264)));
            preset.set_override(
                StyleKey::Ink(crate::Ink::Caption),
                StyleValue::Color([0.5, 0.4, 0.3, 0.5]),
            );
            preset.set_override(StyleKey::Padding, StyleValue::Scalar(6.0));
            preset
        }

        #[test]
        fn a_preset_round_trips_through_toml() {
            let preset = aldermoor();
            let text = preset.to_toml();
            assert_eq!(ThemePreset::from_toml(&text), Ok(preset), "{text}");
            assert!(text.contains("density = 1.1\n"), "{text}");
            assert!(text.contains("\"ink.caption\" = \"#80664d80\""), "{text}");
            let forge = ThemePreset::forge();
            assert_eq!(ThemePreset::from_toml(&forge.to_toml()), Ok(forge));
        }

        #[test]
        fn left_out_seeds_keep_forge_values() {
            let text = "format = 1\nname = \"x\"\n[seeds.accent]\nhue = 135\n";
            let preset = ThemePreset::from_toml(text).unwrap();
            assert_eq!(preset.seeds.accent.hue, 135.0);
            assert_eq!(preset.seeds.accent.chroma, 1.0);
            assert_eq!(preset.seeds.surface, ColorSeed::forge(ColorFamily::Surface));
        }

        #[test]
        fn oklch_colours_are_accepted() {
            let text = "format = 1\n[overrides]\naccent = \"oklch(0.74 0.11 140 / 0.5)\"\n";
            let preset = ThemePreset::from_toml(text).unwrap();
            let want = quantise(oklch(0.74, 0.11, 140.0, 0.5));
            assert_eq!(
                preset.override_of(StyleKey::Accent),
                Some(StyleValue::Color(want))
            );
        }

        #[test]
        fn mistakes_are_errors_that_name_the_key() {
            for (text, needle) in [
                ("name = \"x\"", "missing `format`"),
                ("format = 2", "format 2"),
                ("format = 1\ncolour = 1", "`colour`"),
                ("format = 1\n[seeds]\ndensty = 1", "`seeds.densty`"),
                ("format = 1\n[seeds.accent]\nhew = 1", "`seeds.accent.hew`"),
                (
                    "format = 1\n[overrides]\naccnt = \"#ffffff\"",
                    "`overrides.accnt`",
                ),
                (
                    "format = 1\n[overrides]\naccent = 1.0",
                    "`overrides.accent`",
                ),
                (
                    "format = 1\n[overrides]\naccent = \"#fff\"",
                    "is not a colour",
                ),
                (
                    "format = 1\n[overrides]\npadding = \"6\"",
                    "`overrides.padding`",
                ),
            ] {
                let err = ThemePreset::from_toml(text).unwrap_err().to_string();
                assert!(err.contains(needle), "{text:?} gave {err:?}");
            }
        }
    }
}
