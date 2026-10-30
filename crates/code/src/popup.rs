//! The completion list's layout and paint, after Forge's `CodeEditor`: a
//! dropdown surface under the word being completed, a row per item with
//! its kind's tag, the label (what is typed of it bolder) and its detail at
//! the right, the chosen row on the accent.
//!
//! It is drawn on a tooltip layer, which never takes input, so the keys keep
//! reaching the editor while the pointer rests on the list; the editor
//! takes the clicks on its rows itself, so the list is kept inside it: as
//! many rows as fit, scrolled to keep the chosen one in view.

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    DrawList, Ink, LayerStack, StyleKey, StyleResolver, SurfacePainter, SyntaxTheme, TextBlock,
    TextSize,
};

use crate::completion::Completing;

/// A row's height, the list's padding, a row's own, the tag's column, and
/// the gaps after the tag and before the detail. Its narrowest.
pub(crate) const ROW_HEIGHT: f32 = 20.0;
const PAD: f32 = 3.0;
const ROW_PAD: f32 = 6.0;
const TAG_WIDTH: f32 = 18.0;
const GAP: f32 = 6.0;
const DETAIL_GAP: f32 = 12.0;
const MIN_WIDTH: f32 = 220.0;
const ROW_RADIUS: f32 = 2.0;
/// Its distance from the line it completes.
const OFFSET: f32 = 2.0;

/// Where the list goes, worked out when the editor draws.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Popup {
    pub(crate) rect: Rect,
    /// The rows shown, the first of them `open.items[first]`.
    pub(crate) rows: Vec<Rect>,
    pub(crate) first: usize,
    /// The rect it was kept inside: the editor's code on the screen.
    pub(crate) bounds: Rect,
}

impl Popup {
    /// The item whose row is at (`x`, `y`), if any.
    pub(crate) fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| row.contains(x, y))
            .map(|row| self.first + row)
    }
}

/// Lay out `open`'s list for the word whose start is at `word` (a line's
/// rect on the screen, from the word's start), kept inside `bounds`: under
/// the line, or over it when only that fits, its labels in line with the
/// word. Where neither fits it goes on the roomier side, with as many rows
/// as fit, from `first` (last frame's) moved only as far as keeps the
/// chosen one in view; none when not one row fits either side.
pub(crate) fn layout(
    open: &Completing,
    word: Rect,
    bounds: Rect,
    first: usize,
    styles: &StyleResolver,
    list: &mut DrawList,
) -> Option<Popup> {
    let border = styles.dropdown().surface.border_widths;
    let row_width = open
        .items
        .iter()
        .map(|item| {
            let label = styles.mono_width(list, &item.label, TextSize::Dense);
            let detail = item.detail.as_ref().map_or(0.0, |detail| {
                DETAIL_GAP + styles.mono_width(list, detail, TextSize::Meta)
            });
            ROW_PAD + TAG_WIDTH + GAP + label + detail + ROW_PAD
        })
        .fold(0.0, f32::max);
    let width = (row_width + 2.0 * PAD + border.left + border.right)
        .max(MIN_WIDTH)
        .min(bounds.width);
    let chrome = 2.0 * PAD + border.top + border.bottom;
    let below = word.bottom() + OFFSET;
    let room_below = bounds.bottom() - below;
    let room_above = word.y - OFFSET - bounds.y;
    let all = open.items.len();
    let fits = |room: f32| (((room - chrome) / ROW_HEIGHT).floor().max(0.0) as usize).min(all);
    let full = all as f32 * ROW_HEIGHT + chrome;
    let (shown, above) = if full <= room_below {
        (all, false)
    } else if full <= room_above {
        (all, true)
    } else if room_above > room_below {
        (fits(room_above), true)
    } else {
        (fits(room_below), false)
    };
    if shown == 0 {
        return None;
    }
    let first = first
        .min(open.chosen)
        .max((open.chosen + 1).saturating_sub(shown))
        .min(all - shown);
    let height = shown as f32 * ROW_HEIGHT + chrome;
    let y = if above {
        word.y - OFFSET - height
    } else {
        below
    };
    let x = word.x - (border.left + PAD + ROW_PAD + TAG_WIDTH + GAP);
    let x = x.min(bounds.right() - width).max(bounds.x);
    let rect = Rect::new(x, y, width, height);
    let rows = (0..shown)
        .map(|index| {
            Rect::new(
                rect.x + border.left + PAD,
                rect.y + border.top + PAD + index as f32 * ROW_HEIGHT,
                width - border.left - border.right - 2.0 * PAD,
                ROW_HEIGHT,
            )
        })
        .collect();
    Some(Popup {
        rect,
        rows,
        first,
        bounds,
    })
}

/// Paint `open`'s list where `popup` says, on a tooltip layer of `layers`.
pub(crate) fn paint(
    layers: &mut LayerStack,
    styles: &StyleResolver,
    syntax: &SyntaxTheme,
    open: &Completing,
    popup: &Popup,
    viewport: Rect,
) {
    let index = layers.push_tooltip(viewport);
    let list = layers.current_mut();
    let chrome = styles.dropdown();
    let rect = popup.rect;
    let mut surface = SurfacePainter::new(
        list,
        rect,
        rect.inset(chrome.surface.border_widths.left),
        chrome.surface.corner_radii,
        chrome.surface,
        std::slice::from_ref(&chrome.shadow),
        &chrome.lines,
    );
    surface.paint_pre_content();
    let list = surface.draw_list();
    let theme = styles.theme();
    let font = theme.mono_font.as_ref();
    let dense = styles.text_size(TextSize::Dense);
    let baseline_ratio = list.font_vmetrics(font).baseline_ratio;
    // A list narrowed to its editor cuts its longest rows short.
    list.push_clip(rect.inset(chrome.surface.border_widths.left));
    let shown = open.items.iter().enumerate().skip(popup.first);
    for ((index, item), row) in shown.zip(&popup.rows) {
        let chosen = index == open.chosen;
        if chosen {
            list.rounded_rect(*row, ROW_RADIUS, styles.color(StyleKey::Accent));
        }
        let ink = |role: Ink| styles.ink(role);
        let on_accent = styles.color(StyleKey::OnAccent);
        // One baseline for the row's three sizes, in its middle.
        let baseline =
            row.y + ((ROW_HEIGHT + dense) / 2.0 - dense * (1.0 - baseline_ratio)).round();
        let at = |size: TextSize| baseline - styles.text_size(size) * baseline_ratio;

        let tag_ink = if chosen {
            ink(Ink::OnAccentSecond)
        } else {
            item.kind.ink(syntax)
        };
        let tag = styles
            .mono_block(item.kind.tag(), 0.0, 0.0, TextSize::Caption, Ink::Caption)
            .with_color_f32(tag_ink);
        let (tag_width, _) = list.measure_block(&tag);
        let tag_x = row.x + ROW_PAD + (TAG_WIDTH - tag_width) / 2.0;
        list.text(placed(tag, tag_x, at(TextSize::Caption)));

        let label_x = row.x + ROW_PAD + TAG_WIDTH + GAP;
        let typed = typed_len(&item.label, &open.prefix);
        let (head, tail) = item.label.split_at(typed);
        let head_ink = if chosen { on_accent } else { ink(Ink::Max) };
        let tail_ink = if chosen { on_accent } else { ink(Ink::Menu) };
        let head = styles
            .mono_block(head, 0.0, 0.0, TextSize::Dense, Ink::Max)
            .with_color_f32(head_ink)
            .bold();
        let (head_width, _) = list.measure_block(&head);
        list.text(placed(head, label_x, at(TextSize::Dense)));
        let tail = styles
            .mono_block(tail, 0.0, 0.0, TextSize::Dense, Ink::Menu)
            .with_color_f32(tail_ink);
        let (tail_width, _) = list.measure_block(&tail);
        list.text(placed(tail, label_x + head_width, at(TextSize::Dense)));
        let label_end = label_x + head_width + tail_width;

        if let Some(detail) = &item.detail {
            let detail_ink = if chosen {
                ink(Ink::OnAccentSecond)
            } else {
                ink(Ink::Caption)
            };
            let block = styles
                .mono_block(detail.clone(), 0.0, 0.0, TextSize::Meta, Ink::Caption)
                .with_color_f32(detail_ink);
            let (width, _) = list.measure_block(&block);
            let x = row.right() - ROW_PAD - width;
            // A list narrowed to its editor leaves out a detail that would
            // run into the label.
            if x >= label_end + DETAIL_GAP {
                list.text(placed(block, x, at(TextSize::Meta)));
            }
        }
    }
    list.pop_clip();
    surface.paint_post_content();
    layers.layers_mut()[index].rect = rect;
    layers.pop_layer();
}

/// `block` with its top left at `x`, `y`.
fn placed(block: TextBlock, x: f32, y: f32) -> TextBlock {
    TextBlock { x, y, ..block }
}

/// How many bytes of `label` the typed `prefix` covers, whatever its case:
/// the part drawn bolder.
fn typed_len(label: &str, prefix: &str) -> usize {
    let mut covered = 0;
    let mut typed = prefix.chars();
    for c in label.chars() {
        match typed.next() {
            Some(t) if t.to_lowercase().eq(c.to_lowercase()) => covered += c.len_utf8(),
            _ => break,
        }
    }
    covered
}

#[cfg(test)]
#[path = "popup_tests.rs"]
mod tests;
