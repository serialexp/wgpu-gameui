//! Forge's text glyphs that the fonts and the icon set don't cover, drawn
//! with shapes so they look the same everywhere.

use crate::layout::Rect;

use super::DrawList;

/// `◧`: a square outline `side` px wide with its left half filled, centred
/// in `cell` on whole pixels. Forge uses it as the default row glyph of a
/// drag list and as a file field's file glyph.
pub(crate) fn half_square(list: &mut DrawList, cell: Rect, side: f32, color: [f32; 4]) {
    let x = (cell.x + (cell.width - side) * 0.5).round();
    let y = (cell.y + (cell.height - side) * 0.5).round();
    list.chrome_rect(Rect::new(x, y, side, side), 0.0, 1.0, [0.0; 4], color);
    list.quad(x, y, (side * 0.5).ceil(), side, color);
}

/// `▴`: a small triangle pointing up, filling `cell`'s width and height on
/// whole pixels. Forge puts it after a status zone that opens a drop-up.
pub(crate) fn up_triangle(list: &mut DrawList, cell: Rect, color: [f32; 4]) {
    let left = cell.x.round();
    let right = (cell.x + cell.width).round();
    let top = cell.y.round();
    let bottom = (cell.y + cell.height).round();
    list.triangle(
        ((left + right) * 0.5, top),
        (right, bottom),
        (left, bottom),
        color,
    );
}
