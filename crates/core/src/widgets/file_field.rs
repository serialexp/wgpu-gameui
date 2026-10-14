//! File field — a linked asset shown as a value (Forge `FileField`).

use crate::layout::Rect;
use crate::style::{Ink, StyleKey, StyleResolver, TextSize, Tracking};
use crate::text::vcentered_line_y;

use super::material::{self, Tone};
use super::property_row::LABEL_GAP;
use super::{DrawContext, DrawList, FocusId, IconKey, PROPERTY_LABEL_WIDTH, glyphs};

/// The well's height.
const WELL_H: f32 = 23.0;
/// Between the well row and the meta line.
const META_GAP: f32 = 3.0;
/// Padding inside the well: 6 px before the glyph, 2 px after the keys.
const PAD_LEFT: f32 = 6.0;
const PAD_RIGHT: f32 = 2.0;
/// Between the glyph, the name and the keys.
const GAP: f32 = 6.0;
/// Between the two keys.
const KEY_GAP: f32 = 1.0;
/// The file glyph's cell and square (`◧` at 11 px).
const GLYPH_CELL: f32 = 11.0;
const GLYPH_SIDE: f32 = 8.0;
/// The meta line shown when no file is linked, unless
/// [`accept`](FileField::accept) says otherwise.
const DEFAULT_ACCEPT: &str = "png · ktx2 · exr";

/// A linked file, as a [`FileField`] shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileRef<'a> {
    /// The file's name.
    pub name: &'a str,
    /// A line about it: size, format, dimensions.
    pub meta: &'a str,
}

/// What a [`FileField`] frame reported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileFieldOutput {
    /// Open the file picker: the "…" key, or a click on the empty well.
    pub browse: bool,
    /// Unlink the file: the "×" key.
    pub clear: bool,
}

/// A linked asset shown as a value (Forge `FileField`): a label, then a
/// well with the file's glyph and name, a "…" browse key and, while a file
/// is linked, a "×" clear key; under it a meta line (the file's details,
/// what it accepts, or how the selection differs).
///
/// The field is a value, not a button: only an empty well browses when
/// clicked. Clearing keeps the row's height, so the panel doesn't move
/// under the pointer.
///
/// ```ignore
/// let file = albedo.as_ref().map(|a| FileRef { name: &a.name, meta: &a.meta });
/// let out = FileField::new("Albedo").file(file).draw(rect, &mut ctx);
/// if out.browse { open_picker(); }
/// if out.clear { albedo = None; }
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FileField<'a> {
    label: &'a str,
    file: Option<FileRef<'a>>,
    mixed: bool,
    mixed_count: Option<usize>,
    accept: &'a str,
    label_width: f32,
    focus: Option<FocusId>,
}

impl<'a> FileField<'a> {
    /// An empty field labelled `label`.
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            file: None,
            mixed: false,
            mixed_count: None,
            accept: DEFAULT_ACCEPT,
            label_width: PROPERTY_LABEL_WIDTH,
            focus: None,
        }
    }

    /// The linked file, or `None` for an empty field.
    #[must_use]
    pub fn file(mut self, file: Option<FileRef<'a>>) -> Self {
        self.file = file;
        self
    }

    /// The selection links different files: show "Mixed" and only allow
    /// browsing (which sets them all).
    #[must_use]
    pub fn mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    /// How many different files a [`mixed`](Self::mixed) selection links,
    /// for the meta line ("3 different files").
    #[must_use]
    pub fn mixed_count(mut self, count: usize) -> Self {
        self.mixed_count = Some(count);
        self
    }

    /// The meta line of an empty field: what it takes (default
    /// "png · ktx2 · exr").
    #[must_use]
    pub fn accept(mut self, accept: &'a str) -> Self {
        self.accept = accept;
        self
    }

    /// The label column's width (default [`PROPERTY_LABEL_WIDTH`]).
    #[must_use]
    pub fn label_width(mut self, width: f32) -> Self {
        self.label_width = width;
        self
    }

    /// Put the keys in the Tab ring: the clear key as `base`, the browse key
    /// as `base + 1`.
    #[must_use]
    pub fn focusable(mut self, base: FocusId) -> Self {
        self.focus = Some(base);
        self
    }

    /// The field's height: the well and the meta line under it.
    pub fn height(list: &mut DrawList, s: &StyleResolver) -> f32 {
        let meta = s.mono_block("", 0.0, 0.0, TextSize::Caption, Ink::Dim);
        WELL_H + META_GAP + list.measure_block(&meta).1.ceil()
    }

    /// The linked file, unless the selection is mixed.
    fn linked(&self) -> Option<FileRef<'a>> {
        if self.mixed { None } else { self.file }
    }

    /// Draw the field in `rect` (its height from [`height`](Self::height)).
    pub fn draw(&self, rect: Rect, ctx: &mut DrawContext) -> FileFieldOutput {
        let s = ctx.styles();
        let linked = self.linked();
        let mut out = FileFieldOutput::default();
        ctx.push_debug_scope_rect(super::scope_name("FileField", self.label), rect);

        let label_w = self.label_width.min(rect.width).max(0.0);
        let well_x = rect.x + label_w + LABEL_GAP;
        let well = Rect::new(well_x, rect.y, (rect.right() - well_x).max(0.0), WELL_H);
        let caption = s.text_size(TextSize::Caption);
        ctx.draw_list.text(
            s.caption_block(
                self.label,
                rect.x,
                vcentered_line_y(well.y, well.height, caption),
                Tracking::Prop,
                Ink::Label,
            )
            .with_max_width(label_w)
            .with_ellipsis(),
        );

        let inner = material::draw_row_well(ctx.draw_list, &s, well, false);
        let key = |glyph| {
            IconKey::glyph(glyph, IconKey::HEADER)
                .tone(Tone::Ghost)
                .travel(1.0)
        };
        let [key_w, key_h] = key("…").outer_size(&s);
        let key_y = inner.y + (inner.height - key_h) * 0.5;
        let mut right = inner.right() - PAD_RIGHT;
        let browse_x = right - key_w;
        right = browse_x;
        let clear_x = right - KEY_GAP - key_w;
        if linked.is_some() {
            right = clear_x;
        }

        // The well itself browses only while empty.
        let input = ctx.input;
        let body = Rect::new(inner.x, inner.y, (right - inner.x).max(0.0), inner.height);
        let empty = linked.is_none() && !self.mixed;
        if empty
            && input.mouse_clicked
            && !input.mouse_consumed
            && body.contains(input.mouse_x, input.mouse_y)
        {
            out.browse = true;
        }
        if empty && !input.mouse_consumed && body.contains(input.mouse_x, input.mouse_y) {
            ctx.request_cursor(crate::CursorIcon::Pointer);
        }

        let list = &mut *ctx.draw_list;
        let mut x = inner.x + PAD_LEFT;
        let glyph_ink = if linked.is_some() {
            s.color(StyleKey::AccentGlyph)
        } else {
            s.ink(Ink::DisabledGlyph)
        };
        glyphs::half_square(
            list,
            Rect::new(x, inner.y, GLYPH_CELL, inner.height),
            GLYPH_SIDE,
            glyph_ink,
        );
        x += GLYPH_CELL + GAP;

        let size = s.text_size(TextSize::Dense);
        let y = vcentered_line_y(inner.y, inner.height, size);
        let name = match linked {
            Some(file) => s.mono_block(file.name, x, y, TextSize::Dense, Ink::Value),
            None if self.mixed => s.sans_block("Mixed", x, y, TextSize::Dense, Ink::Disabled),
            None => s.sans_block(
                "No file — click to browse",
                x,
                y,
                TextSize::Dense,
                Ink::Caption,
            ),
        };
        list.text(
            name.with_max_width((right - GAP - x).max(0.0))
                .with_ellipsis(),
        );

        if linked.is_some() {
            let mut clear = key("×");
            if let Some(base) = self.focus {
                clear = clear.focusable(base);
            }
            out.clear = clear
                .draw(Rect::new(clear_x, key_y, key_w, key_h), ctx)
                .clicked;
        }
        let mut browse = key("…");
        if let Some(base) = self.focus {
            browse = browse.focusable(base + 1);
        }
        out.browse |= browse
            .draw(Rect::new(browse_x, key_y, key_w, key_h), ctx)
            .clicked;

        let (meta, ink) = match (linked, self.mixed, self.mixed_count) {
            (Some(file), _, _) => (file.meta.to_string(), Ink::Dim),
            (None, true, Some(n)) => (format!("{n} different files"), Ink::Disabled),
            (None, true, None) => ("multiple values".to_string(), Ink::Disabled),
            (None, false, _) => (self.accept.to_string(), Ink::Disabled),
        };
        let meta_y = well.bottom() + META_GAP;
        ctx.draw_list.text(
            s.mono_block(meta, well.x, meta_y, TextSize::Caption, ink)
                .with_max_width(well.width)
                .with_ellipsis(),
        );
        ctx.pop_debug_scope();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FocusState, InputState, Theme};

    const BRICK: FileRef = FileRef {
        name: "brick_wall_02.ktx2",
        meta: "2048 × 2048 · BC7 · 4.1 MB",
    };

    fn at(x: f32, y: f32) -> InputState {
        InputState {
            mouse_x: x,
            mouse_y: y,
            ..InputState::default()
        }
    }

    fn click(x: f32, y: f32) -> InputState {
        InputState {
            mouse_down: true,
            mouse_clicked: true,
            ..at(x, y)
        }
    }

    fn rect() -> Rect {
        Rect::new(0.0, 0.0, 250.0, 40.0)
    }

    fn frame(field: &FileField, input: &InputState) -> (FileFieldOutput, DrawList) {
        let theme = Theme::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, input, 800.0, 600.0);
            field.draw(rect(), &mut ctx)
        };
        (out, list)
    }

    fn has(list: &DrawList, content: &str) -> bool {
        list.texts.iter().any(|t| t.content == content)
    }

    /// Centres of the browse key and of the clear key (when shown) of a
    /// field at [`rect`].
    fn keys() -> ([f32; 2], [f32; 2]) {
        let right = rect().right() - 1.0 - PAD_RIGHT;
        let y = WELL_H * 0.5;
        let half = IconKey::HEADER * 0.5;
        (
            [right - half, y],
            [right - IconKey::HEADER - KEY_GAP - half, y],
        )
    }

    #[test]
    fn a_linked_file_shows_its_name_meta_and_both_keys() {
        let field = FileField::new("Albedo").file(Some(BRICK));
        let (_, list) = frame(&field, &at(-5.0, -5.0));
        assert!(has(&list, "ALBEDO"));
        assert!(has(&list, BRICK.name));
        assert!(has(&list, BRICK.meta));
        assert!(has(&list, "×") && has(&list, "…"));
    }

    #[test]
    fn the_keys_browse_and_clear() {
        let field = FileField::new("Albedo").file(Some(BRICK));
        let (browse, clear) = keys();
        let (out, _) = frame(&field, &click(browse[0], browse[1]));
        assert_eq!(
            out,
            FileFieldOutput {
                browse: true,
                clear: false
            }
        );
        let (out, _) = frame(&field, &click(clear[0], clear[1]));
        assert_eq!(
            out,
            FileFieldOutput {
                browse: false,
                clear: true
            }
        );
    }

    #[test]
    fn a_linked_well_is_a_value_not_a_button() {
        let field = FileField::new("Albedo").file(Some(BRICK));
        let (out, _) = frame(&field, &click(120.0, 10.0));
        assert!(!out.browse);
    }

    #[test]
    fn an_empty_field_browses_from_its_well_and_has_no_clear_key() {
        let field = FileField::new("Albedo");
        let (_, list) = frame(&field, &at(-5.0, -5.0));
        assert!(has(&list, "No file — click to browse"));
        assert!(has(&list, DEFAULT_ACCEPT));
        assert!(!has(&list, "×"));
        let (out, _) = frame(&field, &click(120.0, 10.0));
        assert!(out.browse);
        let consumed = InputState {
            mouse_consumed: true,
            ..click(120.0, 10.0)
        };
        assert!(!frame(&field, &consumed).0.browse);
        // Where the clear key would be is still the well.
        let (_, clear) = keys();
        assert!(frame(&field, &click(clear[0], clear[1])).0.browse);
    }

    #[test]
    fn a_mixed_field_says_so_and_only_browses_from_its_key() {
        let field = FileField::new("Albedo")
            .file(Some(BRICK))
            .mixed(true)
            .mixed_count(3);
        let (_, list) = frame(&field, &at(-5.0, -5.0));
        assert!(has(&list, "Mixed"));
        assert!(has(&list, "3 different files"));
        assert!(!has(&list, BRICK.name) && !has(&list, "×"));
        assert!(!frame(&field, &click(120.0, 10.0)).0.browse);
        let (browse, _) = keys();
        assert!(frame(&field, &click(browse[0], browse[1])).0.browse);

        let field = FileField::new("Albedo").mixed(true);
        let (_, list) = frame(&field, &at(-5.0, -5.0));
        assert!(has(&list, "multiple values"));
    }

    #[test]
    fn the_meta_line_hangs_under_the_well_at_the_value_column() {
        let field = FileField::new("Albedo").file(Some(BRICK));
        let (_, list) = frame(&field, &at(-5.0, -5.0));
        let meta = list.texts.iter().find(|t| t.content == BRICK.meta).unwrap();
        assert_eq!(meta.x, PROPERTY_LABEL_WIDTH + LABEL_GAP);
        assert_eq!(meta.y, WELL_H + META_GAP);
    }

    #[test]
    fn clearing_keeps_the_height() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        let h = FileField::height(&mut list, &s);
        assert!(h > WELL_H + META_GAP);
        // One height for every state: the caller's layout never moves.
        let (_, linked) = frame(&FileField::new("A").file(Some(BRICK)), &at(-5.0, -5.0));
        let (_, empty) = frame(&FileField::new("A"), &at(-5.0, -5.0));
        let bottom = |list: &DrawList| list.texts.iter().map(|t| t.y).fold(f32::MIN, f32::max);
        assert_eq!(bottom(&linked), bottom(&empty));
    }

    #[test]
    fn keys_join_the_tab_ring_in_reading_order() {
        let theme = Theme::default();
        let mut focus = FocusState::new();
        let tab = InputState {
            nav: crate::NavInput {
                next: true,
                ..Default::default()
            },
            ..at(-5.0, -5.0)
        };
        for expected in [40, 41] {
            let mut list = DrawList::new();
            focus.begin_frame(&tab);
            {
                let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &tab, 800.0, 600.0);
                FileField::new("A")
                    .file(Some(BRICK))
                    .focusable(40)
                    .draw(rect(), &mut ctx);
            }
            focus.end_frame(None);
            assert_eq!(focus.focused(), Some(expected));
        }
    }
}
