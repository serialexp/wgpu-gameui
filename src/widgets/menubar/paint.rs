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

use super::model::{ActivatedItem, Menu, MenuBarId, blocker_region_id, column_blocker_id, row_id};
use super::placement;
use super::state::{MenuBarState, MenuDrawEnv, MenuLayers, activation_id};

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
        if state.up {
            keyed |= state.step_item(menu.items(), -1);
        }
        if state.down {
            keyed |= state.step_item(menu.items(), 1);
        }
        if state.cancel {
            // Unwind one level. With nothing left open the bar stays armed, so menu
            // mode survives the first Escape.
            state.open = None;
            state.highlighted_item = None;
            state.scroll = 0.0;
            forget_geometry(state);
            return None;
        }
    }

    // ---- paint ----
    // The promoted geometry is only paintable while it still describes the open
    // menu: the bar may have switched or closed the chain earlier this frame (a
    // label click or hover), leaving the geometry describing a menu nobody has
    // open.
    //
    // The columns are taken out of the state for the duration of the paint: the
    // rows are mutated (their measured text is `take`n) while the rest of the state
    // stays reachable, and putting the buffer back is what lets the next frame's
    // measurement reuse its capacity.
    let mut columns = std::mem::take(&mut state.columns);
    let geom = state
        .geom
        .take()
        .filter(|geom| geom.menu_index == menu_index);
    let Some(geom) = geom else {
        columns.clear();
        state.columns = columns;
        return None;
    };

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

    let mut hovered_row = None;
    let mut clicked_row = None;
    let mut column_click = false;

    for (level, column) in columns.iter_mut().enumerate() {
        state.scroll_into_view(column);

        // ---- register this column's hit regions ----
        let index = match slots {
            Some(slots) => slots.blocker + 1 + level,
            None => {
                // A host that drew the bar but never pushed the layers still gets
                // a visible, interactive column; it just cannot block input this
                // frame.
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
            // hover and click.
            ctx.interact(column_blocker_id(bar, column.menu_index, level), rect, true);

            ctx.draw_list.push_debug_scope_rect("Menu column", rect);

            // Rows: register for dispatch (culling the scrolled-out band and
            // everything that can never be highlighted) and collect what the
            // previous frame resolved.
            for row in column.rows.iter() {
                if row.separator || row.disabled {
                    continue;
                }
                let y = rect.y + column.sheet_padding + row.y - state.scroll;
                if y + row.height <= rect.y + column.sheet_padding
                    || y >= rect.bottom() - column.sheet_padding
                {
                    continue;
                }
                let response = ctx.interact(
                    row_id(bar, column.menu_index, menu_id, row.item_index, level),
                    Rect::new(row_x, y, row_width, row.height),
                    true,
                );
                if response.hovered {
                    hovered_row = Some(row.item_index);
                    ctx.request_cursor(crate::CursorIcon::Pointer);
                }
                if response.clicked {
                    clicked_row = Some(row.item_index);
                }
            }

            // The pointer takes the highlight back only when it actually moved,
            // so a stationary pointer over the column doesn't fight arrow keys.
            if state.pointer_moved
                && !keyed
                && let Some(item) = hovered_row
            {
                state.highlighted_item = Some(item);
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
                let y = rect.y + column.sheet_padding + row.y - state.scroll;
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
                let highlighted = state.highlighted_item == Some(row.item_index) && !row.disabled;
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
        let chosen = clicked_row.or(if state.confirm {
            state.highlighted_item
        } else {
            None
        });
        if let Some(item_index) = chosen
            && let Some(item) = menu.items().get(item_index)
            && item.is_enabled()
            && !item.is_submenu()
        {
            state.click_claimed = true;
            activated = Some(ActivatedItem {
                id: activation_id(menu, item),
                item,
            });
            // Acting on an item ends the interaction: the chain closes and the
            // bar leaves menu mode.
            state.close();
        }
    }
    if column_click {
        state.click_claimed = true;
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

/// Drop the promoted chain geometry, keeping the buffer's capacity. Called when
/// the chain unwinds a level: what was measured describes the level that closed.
fn forget_geometry(state: &mut MenuBarState) {
    state.geom = None;
    state.columns.clear();
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
