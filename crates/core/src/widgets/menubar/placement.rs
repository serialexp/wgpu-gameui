//! Pure popup placement.
//!
//! Both functions are total and side-effect free, so the behaviour that is
//! hardest to eyeball — flipping at an edge, keeping a column inside an inset
//! viewport — is unit testable without a GPU or a frame.

use crate::layout::Rect;

use super::model::SubmenuSide;

/// Gap between a bar label and the column that drops below it. The Forge sheet
/// is physically attached to the 26px title strip, so its top edge starts exactly
/// at the strip's bottom edge.
pub(crate) const DROP_GAP: f32 = 0.0;
/// Submenus overlap their parent sheet slightly, avoiding a dead gap while the
/// pointer travels into the child.
pub(crate) const SUBMENU_OVERLAP: f32 = 3.0;

/// Where a column goes, given the label it hangs off, its own measured size, the
/// viewport it must stay inside, and the preferred side.
///
/// Returns the rect **and the side actually used**, so a caller can render a
/// directional affordance for the resolved side rather than the requested one.
///
/// The vertical rule is *flip, do not slide*: a column that does not fit below
/// its label goes above it, because sliding it up would cover the strip and
/// fight the hover-to-switch rule that keeps bar labels live. When it fits on
/// neither side (a column taller than the viewport) it is pinned to the viewport
/// top and the caller scrolls it.
///
/// The horizontal rule distinguishes preference from correction, which is what
/// [`SubmenuSide::Auto`] exists for: `Auto` flips to the other side when the
/// preferred one would overflow, while an explicit `Right`/`Left` is honoured and
/// merely *shifted* back inside the viewport. That asymmetry is deliberate and
/// the tests encode it.
pub fn place_popup(
    anchor: Rect,
    size: [f32; 2],
    viewport: Rect,
    side: SubmenuSide,
) -> (Rect, SubmenuSide) {
    let width = size[0].max(0.0);
    let height = size[1].max(0.0);

    // ---- vertical: below, flipping above rather than shifting ----
    let below = anchor.bottom() + DROP_GAP;
    let above = anchor.y - DROP_GAP - height;
    let y = if below + height <= viewport.bottom() {
        below
    } else if above >= viewport.y {
        above
    } else {
        // Taller than the room on either side: pin the top edge so the column
        // starts visible and let the scroll offset reach the rest.
        below.min(viewport.bottom() - height).max(viewport.y)
    };

    // ---- horizontal: honour the preference, flip only for `Auto` ----
    let effective = match side {
        SubmenuSide::Auto => {
            if anchor.x + width <= viewport.right() {
                SubmenuSide::Right
            } else {
                SubmenuSide::Left
            }
        }
        explicit => explicit,
    };
    let preferred_x = match effective {
        // Extends right of the anchor's left edge...
        SubmenuSide::Right | SubmenuSide::Auto => anchor.x,
        // ...or extends left of its right edge, so the column's right edge lines
        // up with the label instead of hanging off it.
        SubmenuSide::Left => anchor.right() - width,
    };
    // Only `Auto` flips; an explicit side slides.
    let x = preferred_x.min(viewport.right() - width).max(viewport.x);

    (Rect::new(x, y, width, height), effective)
}

/// Place a submenu beside its parent row. Unlike a top-level dropdown, the
/// vertical edge follows the row and slides into the viewport; horizontally it
/// overlaps the parent sheet by [`SUBMENU_OVERLAP`] and `Auto` flips per level.
pub fn place_submenu(
    parent_row: Rect,
    size: [f32; 2],
    viewport: Rect,
    side: SubmenuSide,
) -> (Rect, SubmenuSide) {
    let width = size[0].max(0.0).min(viewport.width.max(0.0));
    let height = size[1].max(0.0).min(viewport.height.max(0.0));
    let right_x = parent_row.right() - SUBMENU_OVERLAP;
    let left_x = parent_row.x - width + SUBMENU_OVERLAP;
    let effective = match side {
        SubmenuSide::Auto if right_x + width <= viewport.right() => SubmenuSide::Right,
        SubmenuSide::Auto => SubmenuSide::Left,
        explicit => explicit,
    };
    let preferred_x = if effective == SubmenuSide::Left {
        left_x
    } else {
        right_x
    };
    let x = preferred_x.min(viewport.right() - width).max(viewport.x);
    let y = parent_row.y.min(viewport.bottom() - height).max(viewport.y);
    (Rect::new(x, y, width, height), effective)
}
