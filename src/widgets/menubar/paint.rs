//! Driving the open chain: its own keyboard handling, its column chrome and rows
//! (check gutter, label, hint, chevron, separator), its hit regions, and the
//! activation it resolves.
//!
//! The bar strip itself is drawn by [`MenuBar::draw`](super::MenuBar::draw),
//! because the bar-level keyboard transitions need the menus and their label
//! rects. This module owns everything that happens *inside* the popup layers.

use crate::color::{opaque_srgb8, srgb_to_linear};
use crate::layout::Rect;
use crate::{
    Affine2, CornerRadii, DrawContext, Edge, HitShape, InteractionScene, LayerStack, PointerPolicy,
    StyleKey, SurfacePainter,
};

use super::model::{
    ActivatedItem, Menu, MenuBarId, MenuItem, blocker_region_id, column_blocker_id, row_path_id,
};
use super::placement;
use super::state::{MenuBarState, MenuDrawEnv, MenuLayers, activation_id_for_path};

/// Check-mark stroke width, as a fraction of the row height.
const CHECK_STROKE: f32 = 0.10;
/// Half-extent of the submenu chevron, as a fraction of the row height. Matches
/// the gutter reserved by the geometry pass.
const CHEVRON: f32 = 0.18;

/// Register the viewport blocker as scene regions at the blocker layer.
///
/// `InteractionScene` dispatches among *registered regions* only, so a popup
/// layer alone does not stop a scene-backed base widget from winning pointer
/// dispatch underneath the chain. These regions are the scene half of the
/// blocker, and they exclude the bar strip so the bar's hover-to-switch stays
/// live.
fn register_viewport_blocker(
    scene: &mut InteractionScene,
    bar: MenuBarId,
    layer: usize,
    viewport: Rect,
    bar_rect: Rect,
) {
    let mut regions = [Rect::new(0.0, 0.0, 0.0, 0.0); 4];
    let count = placement::blocker_regions(viewport, bar_rect, &mut regions);
    // `OrderKey.layer` is the popup layer's index + 1, matching what
    // `DrawContext::interact` derives from `active_layer`.
    let scene_layer = layer as u32 + 1;
    for (index, region) in regions[..count].iter().enumerate() {
        scene.register(
            blocker_region_id(bar, index),
            HitShape::Rect(*region),
            Affine2::IDENTITY,
            None,
            scene_layer,
            true,
            PointerPolicy::Target,
        );
    }
}

/// Drive the open chain, and return the item it activated, if any.
///
/// Activation is reported from exactly here — never from the bar — so there is no
/// ambiguity about which result to read.
///
/// The input half runs whenever a chain is open, whether or not this frame has a
/// paintable column: the geometry is deliberately a frame behind the state (the
/// layer rects that block the base layer can only come from the *previous*
/// frame's measurement), so the frame after a switch has nothing to paint — and
/// it must still take the keyboard, or the arrow key that switched the menu would
/// swallow the next one.
pub(super) fn draw_columns<'a>(
    state: &mut MenuBarState,
    bar: MenuBarId,
    menus: &'a [Menu<'a>],
    layers: &mut LayerStack,
    slots: Option<MenuLayers>,
    env: &mut MenuDrawEnv<'_>,
) -> Option<ActivatedItem<'a>> {
    let menu_index = state.open?;
    let Some(menu) = menus.get(menu_index) else {
        // The bar shrank under the open chain: there is no menu left to draw it
        // from, so drop the chain rather than index past the list.
        state.close();
        return None;
    };
    // Only a chain that was *already* open at frame-top takes input: the edge that
    // opened it must not also move in it.
    let navigable = state.was_open == state.open;

    // ---- input ----
    // Rows move the highlight; the bar's own left/right walk happens in
    // `MenuBar::draw`, which runs earlier and can still measure the column it
    // switches to.
    let mut keyed = false;
    if navigable {
        let level = state.open_levels().saturating_sub(1);
        let items = state.items_at_level(menu, level).unwrap_or(&[]);
        if state.up {
            keyed |= state.step_item(items, level, -1);
        }
        if state.down {
            keyed |= state.step_item(items, level, 1);
        }
        let highlighted = state.highlights.get(level).copied().flatten();
        if (state.right || state.confirm)
            && let Some(parent) = highlighted
            && items
                .get(parent)
                .is_some_and(|item| item.is_enabled() && item.is_submenu())
        {
            keyed |= state.open_child(menu, level, parent);
        }
        if (state.left && level > 0) || state.cancel {
            if state.unwind_level() {
                forget_geometry(state);
            } else if state.cancel {
                state.open = None;
                state.highlighted_item = None;
                state.scroll = 0.0;
                forget_geometry(state);
            }
            return None;
        }
    }

    // ---- current-pointer submenu intent ----
    // InteractionScene deliberately resolves presses against retained geometry,
    // but submenu hover must not inherit that one-frame latency. The promoted
    // columns are already visible, so hit-test their rows against the current
    // pointer and update the path before selecting what this call paints.
    let geom = state
        .geom
        .take()
        .filter(|geom| geom.menu_index == menu_index);
    let Some(geom) = geom else {
        state.columns.clear();
        return None;
    };
    let path_changed = navigable && !keyed && reconcile_pointer_path(state, menu);
    if path_changed {
        state.recollect_open_chain(menus, layers.base_mut(), env.theme, env.style, geom);
    }

    // ---- paint ----
    // The columns are taken out of the state for the duration of the paint: the
    // rows are mutated (their measured text is `take`n) while the rest of the state
    // stays reachable, and putting the buffer back is what lets the next frame's
    // measurement reuse its capacity.
    let mut columns = std::mem::take(&mut state.columns);

    // The bar's rect from this frame if it was drawn, else the promoted one.
    let bar_rect = if state.bar_rect.is_empty() {
        geom.bar_rect
    } else {
        state.bar_rect
    };
    let viewport = if state.viewport.is_empty() {
        geom.viewport
    } else {
        state.viewport
    };
    if let Some(slots) = slots {
        register_viewport_blocker(env.interactions, bar, slots.blocker, viewport, bar_rect);
    }

    let mut hovered_row: Option<(usize, usize)> = None;
    let mut clicked_row: Option<(usize, usize)> = None;
    let mut column_click = false;

    for (level, column) in columns.iter_mut().enumerate() {
        state.scroll_into_view(column, level);
        let level_scroll = state.scrolls.get(level).copied().unwrap_or(0.0);

        // ---- register this column's hit regions ----
        let index = match slots.filter(|slots| level < slots.count) {
            Some(slots) => slots.blocker + 1 + level,
            None => {
                // A host that drew the bar but never pushed the layers still gets
                // a visible, interactive column; it just cannot block input this
                // frame. This also covers a child opened during this popup pass:
                // append its layer now so it paints immediately, while the already
                // reserved viewport blocker protects lower content.
                let index = layers.push_popup(column.rect);
                layers.pop_layer();
                index
            }
        };
        let rect = column.rect;
        let row_h = column.row_h;
        let row_x = rect.x + column.sheet_padding;
        let row_width = (rect.width - column.sheet_padding * 2.0).max(0.0);
        let label_avail = column.label_avail;
        let hint_right = column.hint_right;
        let menu_id = menus.get(column.menu_index).and_then(Menu::id_value);

        {
            let mut ctx = DrawContext::new(
                &mut layers.layers_mut()[index].list,
                env.focus,
                env.theme,
                env.input,
                env.screen_width,
                env.screen_height,
            );
            ctx.active_layer = Some(index);
            ctx.style = env.style;
            ctx.animations = env.animations.as_deref_mut();
            ctx.cursor = env.cursor.as_deref_mut();
            ctx.interactions = Some(&mut *env.interactions);

            // The column's own blocker, registered *before* the rows so the rows
            // win dispatch within the layer: a scene-backed base widget under the
            // column's padding, border or a separator would otherwise still win
            // hover and click. Include the full branch path: retained responses
            // from an equal-depth sibling column must not transfer to its replacement.
            ctx.interact(
                column_blocker_id(bar, column.menu_index, &column.path[..level], level),
                rect,
                true,
            );

            ctx.draw_list.push_debug_scope_rect("Menu column", rect);

            // Rows: register for dispatch (culling the scrolled-out band and
            // everything that can never be highlighted) and collect what the
            // previous frame resolved.
            for row in column.rows.iter() {
                if row.separator || row.disabled {
                    continue;
                }
                let y = rect.y + column.sheet_padding + row.y - level_scroll;
                if y + row.height <= rect.y + column.sheet_padding
                    || y >= rect.bottom() - column.sheet_padding
                {
                    continue;
                }
                let response = ctx.interact(
                    row_path_id(
                        bar,
                        column.menu_index,
                        menu_id,
                        &column.path[..level],
                        row.item_index,
                        level,
                    ),
                    Rect::new(row_x, y, row_width, row.height),
                    true,
                );
                if response.hovered {
                    hovered_row = Some((level, row.item_index));
                    ctx.request_cursor(crate::CursorIcon::Pointer);
                }
                if response.clicked {
                    clicked_row = Some((level, row.item_index));
                }
            }

            // The pointer takes the highlight back only when it actually moved,
            // so a stationary pointer over the column doesn't fight arrow keys.
            if state.pointer_moved
                && !keyed
                && let Some((hover_level, item)) = hovered_row
                && hover_level == level
            {
                if let Some(slot) = state.highlights.get_mut(level) {
                    *slot = Some(item);
                }
                if level == 0 {
                    state.highlighted_item = Some(item);
                }
            }

            // ---- paint ----
            let s = ctx.styles();
            let chrome = s.menu_sheet();
            let target = &mut *ctx.draw_list;
            let padding_box = sheet_padding_box(rect, chrome.surface.border_widths);
            let mut painter = SurfacePainter::new(
                target,
                rect,
                padding_box,
                CornerRadii::default(),
                chrome.surface,
                &chrome.shadows,
                &chrome.lines,
            );
            painter.paint_pre_content();
            let list = painter.draw_list();
            // TextBlock colours are encoded 8-bit sRGB, unlike DrawList geometry
            // colours (linear floats), so these handoff CSS values stay literal.
            let disabled_text = (0x5d, 0x65, 0x6c);
            let text = (0xdb, 0xe1, 0xe7);
            let selected_bg = opaque_srgb8([0x79, 0xc6, 0xd8]);
            let check_w = column.check_w;

            // Only row content is clipped to the sheet padding box. The sheet and
            // both analytic shadows above remain unclipped so their falloff can
            // paint beyond the popup rect.
            list.push_clip_viewport(padding_box);
            for row in column.rows.iter_mut() {
                let y = rect.y + column.sheet_padding + row.y - level_scroll;
                if y + row.height <= rect.y + column.sheet_padding
                    || y >= rect.bottom() - column.sheet_padding
                {
                    continue;
                }
                if row.separator {
                    let rule_y = y + 3.0;
                    let rule_x = row_x + 6.0;
                    let rule_w = (row_width - 12.0).max(0.0);
                    let rule = Rect::new(rule_x, rule_y, rule_w, row.height - 3.0);
                    list.edge_line(
                        rule,
                        Edge::Top,
                        chrome.separator[0].thickness,
                        chrome.separator[0].color,
                    );
                    list.edge_line(
                        Rect::new(
                            rule.x,
                            rule.y + chrome.separator[0].thickness,
                            rule.width,
                            rule.height,
                        ),
                        Edge::Top,
                        chrome.separator[1].thickness,
                        chrome.separator[1].color,
                    );
                    continue;
                }
                let highlighted = state.highlights.get(level).copied().flatten()
                    == Some(row.item_index)
                    && !row.disabled;
                if highlighted {
                    paint_accent_row(
                        list,
                        Rect::new(row_x, y, row_width, row.height),
                        selected_bg,
                    );
                }
                if row.checked {
                    let stroke = (row_h * CHECK_STROKE).max(1.0);
                    let (x, w, h) = (row_x + 8.0, check_w, row_h);
                    let color = if highlighted {
                        srgb_to_linear([4.0 / 255.0, 20.0 / 255.0, 24.0 / 255.0, 1.0])
                    } else {
                        srgb_to_linear([0.4265, 0.8174, 0.8336, 1.0])
                    };
                    list.line(
                        [x + w * 0.16, y + h * 0.52],
                        [x + w * 0.40, y + h * 0.76],
                        stroke,
                        color,
                    );
                    list.line(
                        [x + w * 0.40, y + h * 0.76],
                        [x + w * 0.84, y + h * 0.22],
                        stroke,
                        color,
                    );
                }
                if row.submenu {
                    let half = row_h * CHEVRON;
                    let cx = row_x + row_width - 8.0 - half * 0.6;
                    let cy = y + row_h * 0.5;
                    list.triangle(
                        (cx - half * 0.5, cy - half),
                        (cx - half * 0.5, cy + half),
                        (cx + half * 0.7, cy),
                        if highlighted {
                            srgb_to_linear([4.0 / 255.0, 20.0 / 255.0, 24.0 / 255.0, 1.0])
                        } else {
                            s.color(if row.disabled {
                                StyleKey::TextDim
                            } else {
                                StyleKey::Text
                            })
                        },
                    );
                }

                let (r, g, b) = if row.disabled {
                    disabled_text
                } else if highlighted {
                    (4, 20, 24)
                } else {
                    text
                };
                if let Some(measured) = row.label.take() {
                    let ty = y + row_h * 0.5 - measured.metrics.visual_center;
                    let mut block = measured.into_block_at(row_x + 8.0 + check_w + 7.0, ty);
                    if row.label_w > label_avail {
                        // Narrower than its intrinsic text (a column clamped by
                        // the viewport): truncate rather than overflow.
                        block = block.with_max_width(label_avail).with_ellipsis();
                    }
                    let block = block.with_color(r, g, b);
                    list.text(if highlighted {
                        block
                    } else {
                        block.with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.5)
                    });
                }
                if let Some(measured) = row.hint.take() {
                    let ty = y + row_h * 0.5 - measured.metrics.visual_center;
                    let x = hint_right - measured.metrics.size[0];
                    let (r, g, b) = if row.disabled {
                        (0x46, 0x4e, 0x55)
                    } else if highlighted {
                        // TextBlock cannot express alpha independently of glyph
                        // coverage here, so preserve the handoff's dark ink hue.
                        (4, 20, 24)
                    } else {
                        (0x78, 0x81, 0x8a)
                    };
                    let block = measured.into_block_at(x, ty).with_color(r, g, b);
                    list.text(if highlighted {
                        block
                    } else {
                        block.with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.5)
                    });
                }
            }

            list.pop_clip();
            painter.paint_post_content();
            target.pop_debug_scope();
        }

        // A press anywhere inside the column is the menu's, not the widget's:
        // clicking padding or a separator keeps the menu open instead of
        // counting as a click elsewhere.
        if state.mouse_clicked && rect.contains(state.mouse_x, state.mouse_y) {
            column_click = true;
        }
    }

    // ---- resolve ----
    let mut activated = None;
    if navigable {
        let deepest = state.open_levels().saturating_sub(1);
        let chosen = clicked_row.or_else(|| {
            (state.confirm && !keyed)
                .then(|| {
                    state
                        .highlights
                        .get(deepest)
                        .copied()
                        .flatten()
                        .map(|item| (deepest, item))
                })
                .flatten()
        });
        if let Some((level, item_index)) = chosen
            && let Some(items) = state.items_at_level(menu, level)
            && let Some(item) = items.get(item_index)
            && item.is_enabled()
        {
            state.click_claimed = true;
            if item.is_submenu() {
                state.open_child(menu, level, item_index);
            } else {
                activated = Some(ActivatedItem {
                    id: activation_id_for_path(menu, &state.open_path[..level], item),
                    item,
                });
                state.close();
            }
        }
    }
    if column_click {
        state.click_claimed = true;
    }
    if state.open == Some(menu_index) && state.open_levels() != columns.len() {
        // A retained click or keyboard edge can open a parent after this frame's
        // hover/layout pass. Stage the resulting complete path now so it cannot
        // freeze until an unrelated pointer event in an event-driven host.
        state.stage_open_chain(menus, layers.base_mut(), env.theme, env.style, geom);
    }
    // Retained for the next frame (and for `debug_geometry`): a chain that is
    // still open keeps what was last drawn, one that closed or switched during the
    // paint does not. `close` already dropped the state's copy, but the columns
    // were out of the state for the duration of the paint.
    state.columns = columns;
    if state.open != Some(menu_index) {
        state.columns.clear();
    }
    activated
}

/// Reconcile the current pointer with the visible chain before painting it.
/// Returns true when the open path changed and geometry must be rebuilt.
fn reconcile_pointer_path(state: &mut MenuBarState, menu: &Menu<'_>) -> bool {
    let point = (state.mouse_x, state.mouse_y);
    let mut hovered = None;
    for (level, column) in state.columns.iter().enumerate().rev() {
        if !column.rect.contains(point.0, point.1) {
            continue;
        }
        let scroll = state.scrolls.get(level).copied().unwrap_or(0.0);
        hovered = column
            .rows
            .iter()
            .find(|row| {
                !row.separator
                    && !row.disabled
                    && Rect::new(
                        column.rect.x + column.sheet_padding,
                        column.rect.y + column.sheet_padding + row.y - scroll,
                        (column.rect.width - column.sheet_padding * 2.0).max(0.0),
                        row.height,
                    )
                    .contains(point.0, point.1)
            })
            .map(|row| (level, row.item_index));
        break;
    }

    if let Some((level, item_index)) = hovered {
        let is_submenu = state
            .items_at_level(menu, level)
            .and_then(|items| items.get(item_index))
            .is_some_and(MenuItem::is_submenu);
        if is_submenu {
            if state.open_path.get(level).copied() == Some(item_index) {
                return false;
            }
            return state.open_child(menu, level, item_index);
        }
        if state.open_path.len() > level {
            state.open_path.truncate(level);
            state.highlights.truncate(level + 1);
            state.scrolls.truncate(level + 1);
            return true;
        }
        return false;
    }

    // Keep an open child while the pointer is inside any descendant column or is
    // travelling through the safe triangle from its parent toward that child.
    for level in 0..state.open_path.len() {
        let Some(child) = state.columns.get(level + 1) else {
            continue;
        };
        if child.rect.contains(point.0, point.1)
            || state
                .previous_pointer
                .is_some_and(|from| safe_corridor(from, point, child.rect))
        {
            return false;
        }
    }

    // The root sheet remains open, but moving out of the active branch closes
    // all of its descendants immediately. A stationary pointer must not undo a
    // keyboard-opened path.
    if state.pointer_moved && !state.open_path.is_empty() {
        state.open_path.clear();
        state.highlights.truncate(1);
        state.scrolls.truncate(1);
        return true;
    }
    false
}

/// Drop the promoted chain geometry, keeping the buffer's capacity. Called when
/// the chain unwinds a level: what was measured describes the level that closed.
fn forget_geometry(state: &mut MenuBarState) {
    state.geom = None;
    state.columns.clear();
}

pub(super) fn safe_corridor(from: (f32, f32), point: (f32, f32), child: Rect) -> bool {
    if child.contains(point.0, point.1) {
        return true;
    }
    let edge_x = if child.x >= from.0 {
        if point.0 <= from.0 {
            return false;
        }
        child.x
    } else {
        if point.0 >= from.0 {
            return false;
        }
        child.right()
    };
    point_in_triangle(point, from, (edge_x, child.y), (edge_x, child.bottom()))
}

fn point_in_triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    fn cross(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> f32 {
        (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
    }
    let (x, y, z) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
    (x >= 0.0 && y >= 0.0 && z >= 0.0) || (x <= 0.0 && y <= 0.0 && z <= 0.0)
}

fn sheet_padding_box(rect: Rect, widths: crate::EdgeWidths) -> Rect {
    Rect::new(
        rect.x + widths.left,
        rect.y + widths.top,
        (rect.width - widths.left - widths.right).max(0.0),
        (rect.height - widths.top - widths.bottom).max(0.0),
    )
}

fn paint_accent_row(list: &mut crate::DrawList, rect: Rect, accent: [f32; 4]) {
    list.quad(rect.x, rect.y, rect.width, rect.height, accent);
    list.quad(
        rect.x,
        rect.y,
        rect.width,
        rect.height.min(1.0),
        opaque_srgb8([0xa1, 0xd7, 0xe4]),
    );
    list.quad(
        rect.x,
        rect.y + (rect.height - 1.0).max(0.0),
        rect.width,
        rect.height.min(1.0),
        opaque_srgb8([0x5b, 0x95, 0xa2]),
    );
}
