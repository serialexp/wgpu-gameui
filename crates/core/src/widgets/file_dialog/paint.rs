//! The file dialog's drawn pieces that hold no behaviour: the folder and
//! page glyphs, an entry's icon, and small text helpers.

use crate::color::oklch;
use crate::layout::Rect;
use crate::shadow::CornerRadii;
use crate::style::{Ink, StyleResolver, TextSize};
use crate::text::{TextBlock, vcentered_line_y};
use crate::{Background, GradientAxis};

use super::super::{DrawList, Thumb};
use super::model::ext_of;
use super::{FileEntry, FileKind};

/// The folder glyph's hue (`oklch(… 0.06 230)` in the design).
const FOLDER_HUE: f32 = 230.0;
/// The page glyph's border on an accent row.
const PAGE_EDGE_ON: [f32; 4] = [4.0 / 255.0, 20.0 / 255.0, 24.0 / 255.0, 0.45];
const PAGE_EDGE: [f32; 4] = [1.0, 1.0, 1.0, 0.22];
const GLYPH_DROP: [f32; 4] = [0.0, 0.0, 0.0, 0.45];
const GLYPH_HI: [f32; 4] = [1.0, 1.0, 1.0, 0.3];

/// The width a glyph of `size` takes (the design's `size * 1.05`).
pub(super) fn glyph_width(size: f32) -> f32 {
    (size * 1.05).round()
}

/// A folder: a tab over a body, blue-grey, `size` tall, from `(x, y)`.
/// `on` is the glyph on an accent row.
pub(super) fn folder_glyph(
    list: &mut DrawList,
    s: &StyleResolver,
    x: f32,
    y: f32,
    size: f32,
    on: bool,
) {
    let on_ink = s.ink(Ink::OnAccentSecond);
    let w = glyph_width(size);
    let h = (size * 0.78).round();
    let tab_h = (size * 0.2).round().max(2.0);
    let tab_w = (w * 0.45).round();
    let tab_top = y + size - h - (size * 0.12).round().max(1.0);
    let radius = if size >= 28.0 { 3.0 } else { 1.5 };
    let tab = if on {
        on_ink
    } else {
        oklch(0.68, 0.06, FOLDER_HUE, 0.8)
    };
    list.paint_quad_background(
        Rect::new(x, tab_top, tab_w, tab_h),
        Background::Solid(tab),
        CornerRadii::new(1.0, 1.0, 0.0, 0.0),
    );
    let body = Rect::new(x, y + size - h, w, h);
    list.quad(
        body.x + 1.0,
        body.bottom(),
        body.width - 2.0,
        1.0,
        GLYPH_DROP,
    );
    let fill = if on {
        Background::Solid(on_ink)
    } else {
        Background::LinearGradient {
            start: oklch(0.72, 0.06, FOLDER_HUE, 1.0),
            end: oklch(0.56, 0.06, FOLDER_HUE, 1.0),
            axis: GradientAxis::Vertical,
        }
    };
    list.paint_quad_background(body, fill, CornerRadii::uniform(radius));
    list.quad(
        body.x + radius,
        body.y,
        body.width - radius * 2.0,
        1.0,
        GLYPH_HI,
    );
}

/// A page, with its extension on it when large enough.
pub(super) fn file_glyph(
    list: &mut DrawList,
    s: &StyleResolver,
    x: f32,
    y: f32,
    size: f32,
    ext: &str,
    on: bool,
) {
    let w = (size * 0.78).round();
    let page = Rect::new(x + ((glyph_width(size) - w) * 0.5).round(), y, w, size);
    let radius = if size >= 28.0 { 3.0 } else { 1.5 };
    list.quad(
        page.x + 1.0,
        page.bottom(),
        page.width - 2.0,
        1.0,
        GLYPH_DROP,
    );
    list.paint_quad_background(
        page,
        Background::LinearGradient {
            start: [1.0, 1.0, 1.0, 0.16],
            end: [1.0, 1.0, 1.0, 0.05],
            axis: GradientAxis::Vertical,
        },
        CornerRadii::uniform(radius),
    );
    list.rounded_rect_outline(page, radius, 1.0, if on { PAGE_EDGE_ON } else { PAGE_EDGE });
    if size >= 28.0 && !ext.is_empty() {
        let text_size = (size * 0.17).round().max(8.0);
        let ink = if on {
            s.color(crate::StyleKey::OnAccent)
        } else {
            s.ink(Ink::Glyph)
        };
        let mut block = TextBlock::new(ext.to_uppercase(), 0.0, 0.0)
            .with_size(text_size)
            .with_color_f32(ink)
            .with_letter_spacing(text_size * 0.04)
            .with_font_opt(s.theme().mono_font.clone())
            .bold();
        let (tw, th) = list.measure_block(&block);
        block.x = page.x + ((page.width - tw) * 0.5).round();
        block.y = page.bottom() - (size * 0.14).round() - th;
        block = block.with_max_width(page.width);
        list.text(block);
    }
}

/// An entry's icon: its thumb, or the folder / page glyph, `size` tall.
pub(super) fn entry_icon(
    list: &mut DrawList,
    s: &StyleResolver,
    entry: &FileEntry,
    x: f32,
    y: f32,
    size: f32,
    on: bool,
) {
    if let Some(thumb) = &entry.thumb {
        let mut t = Thumb::new().size(size).selected(on);
        if let Some(sprite) = thumb.sprite {
            t = t.image(sprite);
        }
        if let Some(color) = thumb.color {
            t = t.color(color);
        }
        if let Some(name) = thumb.monogram.as_deref() {
            t = t.name(name);
        }
        t.draw(x, y, list, s);
        return;
    }
    match entry.kind {
        FileKind::Folder => folder_glyph(list, s, x, y, size, on),
        FileKind::File => file_glyph(list, s, x, y, size, &ext_of(&entry.name), on),
    }
}

/// The top of a line of `step` text centred in `(top, height)`.
pub(super) fn line_y(s: &StyleResolver, top: f32, height: f32, step: TextSize) -> f32 {
    vcentered_line_y(top, height, s.text_size(step))
}

/// A mono line in `role`, its right edge at `right`.
#[allow(clippy::too_many_arguments)]
pub(super) fn mono_right(
    list: &mut DrawList,
    s: &StyleResolver,
    text: &str,
    right: f32,
    top: f32,
    height: f32,
    max_width: f32,
    color: [f32; 4],
) {
    let w = s.mono_width(list, text, TextSize::Meta).min(max_width);
    list.text(
        s.mono_block(
            text,
            right - w,
            line_y(s, top, height, TextSize::Meta),
            TextSize::Meta,
            Ink::Caption,
        )
        .with_color_f32(color)
        .with_max_width(max_width)
        .with_ellipsis(),
    );
}

/// The design's skeleton bar widths for the loading rows.
pub(super) fn skeleton_width(row: usize) -> f32 {
    70.0 + ((row * 37) % 90) as f32
}
