//! Toolbar widget — a strip of tool buttons with separators and a grip handle.
//!
//! The toolbar docks to an edge of the viewport (left/right/top/bottom). Tool
//! clicks and hover are resolved inline; its dock chooser and overflow sheet
//! follow the crate's deferred popup-layer protocol so they paint above the base
//! UI and block it correctly. The grip handle's drag is arbitrated through a
//! caller-owned [`DragCapture`]; tooltips are reported via the output for the
//! caller's own [`TooltipLayer`](crate::TooltipLayer).
//!
//! # State ownership
//!
//! [`ToolbarState`] is caller-owned and persists across frames — the same
//! contract as [`MenuBarState`](crate::MenuBarState). It is *not* a field of
//! `UiState` because `active_tool` and `active_toggles` are application state,
//! not UI plumbing. For popup support, each frame calls
//! [`ToolbarState::begin_frame`], [`ToolbarState::push_open_layer`], base
//! [`Toolbar::draw_with_id`], [`ToolbarState::draw_open_layer`], then
//! [`ToolbarState::end_frame`], mirroring [`DropdownState`](crate::DropdownState).
//!
//! # Example
//!
//! ```ignore
//! let items: &[ToolbarItem] = &[
//!     ToolbarItem::tool(1, Icon::new(PhosphorIcon::Cursor), "Select", "Q"),
//!     ToolbarItem::tool(2, Icon::new(PhosphorIcon::ArrowsOutCardinal), "Move", "W"),
//!     ToolbarItem::separator(),
//!     ToolbarItem::tool(3, Icon::new(PhosphorIcon::Cube), "Box brush", "B"),
//! ];
//! let out = Toolbar::new(items).draw(rect, &mut state, &mut capture, GRIP_ID, &mut ctx);
//! if let Some(id) = out.clicked { state.active_tool = Some(id); }
//! ```

use crate::chrome::{Background, Edge, QuadStyle, StructuralLine, SurfacePainter};
use crate::color::{hex, rgba8};
use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{StyleKey, StyleResolver};
use crate::widgets::drag::{DragCapture, DragId};
use crate::widgets::icon::Icon;
use crate::widgets::material::{self, Material, Tone};
use crate::{InputState, LayerStack};

use super::{DrawContext, DrawList};

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// Which edge of the viewport the toolbar docks to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToolbarEdge {
    /// Left edge (vertical strip, tools stacked top-to-bottom).
    #[default]
    Left,
    /// Right edge (vertical strip).
    Right,
    /// Top edge (horizontal strip, tools laid out left-to-right).
    Top,
    /// Bottom edge (horizontal strip).
    Bottom,
}

impl ToolbarEdge {
    /// Whether this edge produces a vertical toolbar.
    pub fn is_vertical(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

/// One tool button's description, borrowed per-frame.
pub struct ToolDef<'a> {
    /// Stable identity used for active-tool matching and reported in output.
    pub id: u64,
    /// The icon to draw inside the button.
    pub icon: Icon,
    /// Display name (shown in tooltip).
    pub label: &'a str,
    /// Shortcut key hint (shown in tooltip, e.g. `"B"` or `"Shift+G"`).
    pub shortcut: &'a str,
    /// Whether this tool can be selected.
    pub enabled: bool,
}

/// Stable identity for a toolbar within one UI surface.
pub type ToolbarId = u64;

/// An interaction the toolbar resolved this frame. Application state remains
/// caller-owned: apply this event to the selected tool or toggle value yourself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarEvent {
    /// A modal tool was activated.
    ToolActivated(u64),
    /// A toggle tool was activated.
    ToggleActivated(u64),
    /// The dock sheet selected a new edge.
    DockChanged(ToolbarEdge),
}

/// A toolbar item: a modal tool, independently-held toggle, or group separator.
pub enum ToolbarItem<'a> {
    /// A modal tool button, highlighted by [`ToolbarState::active_tool`].
    Tool(ToolDef<'a>),
    /// An independently-held tool button, highlighted by [`ToolDef::enabled`] and
    /// the caller-supplied active id in [`ToolbarState::active_tool`].
    Toggle(ToolDef<'a>),
    /// A thin separator line between tool button groups.
    Separator,
}

impl<'a> ToolbarItem<'a> {
    /// Shorthand for a modal tool item.
    pub fn tool(id: u64, icon: Icon, label: &'a str, shortcut: &'a str) -> Self {
        Self::Tool(ToolDef {
            id,
            icon,
            label,
            shortcut,
            enabled: true,
        })
    }

    /// Shorthand for an independently-held toggle item.
    pub fn toggle(id: u64, icon: Icon, label: &'a str, shortcut: &'a str) -> Self {
        Self::Toggle(ToolDef {
            id,
            icon,
            label,
            shortcut,
            enabled: true,
        })
    }

    /// Shorthand for a separator.
    pub fn separator() -> Self {
        Self::Separator
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct PopupGeometry {
    anchor: Rect,
    viewport: Rect,
    kind: PopupKind,
    /// First item that did not fit in the inline rail. Meaningful only for
    /// [`PopupKind::Overflow`]; dock sheets set this to zero.
    overflow_start: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PopupKind {
    Dock,
    Overflow,
}

/// Caller-owned toolbar state, persisted across frames.
pub struct ToolbarState {
    /// Which edge the toolbar currently docks to.
    pub edge: ToolbarEdge,
    /// Which modal tool id is currently active.
    pub active_tool: Option<u64>,
    /// Independently-held toggle ids. The application owns and updates this
    /// collection after receiving [`ToolbarEvent::ToggleActivated`].
    pub active_toggles: Vec<u64>,
    /// The bounding rect the toolbar can dock within — set by
    /// [`AppShell`](crate::AppShell) (or the caller) before drawing. Grip
    /// drag-to-dock checks the mouse against this area, not the full screen.
    /// Defaults to a zero rect; when zero the drag falls back to screen bounds.
    pub dock_area: Rect,
    popup: Option<PopupKind>,
    geom: Option<PopupGeometry>,
    next_geom: Option<PopupGeometry>,
    escape: bool,
    mouse_clicked: bool,
    click_claimed: bool,
}

impl ToolbarState {
    /// Create a new toolbar state docked to `edge` with no active tools.
    pub fn new(edge: ToolbarEdge) -> Self {
        Self {
            edge,
            active_tool: None,
            active_toggles: Vec::new(),
            dock_area: Rect::default(),
            popup: None,
            geom: None,
            next_geom: None,
            escape: false,
            mouse_clicked: false,
            click_claimed: false,
        }
    }

    /// Promote last frame's popup geometry and claim Escape for an open sheet.
    /// Call before deriving base-layer input from [`LayerStack`].
    pub fn begin_frame(&mut self, input: &mut InputState) {
        self.geom = self.next_geom.take();
        self.escape = self.popup.is_some() && input.nav.cancel;
        if self.popup.is_some() {
            input.nav.cancel = false;
        }
        self.mouse_clicked = input.mouse_clicked;
        self.click_claimed = false;
    }

    /// Push the popup sheet from last frame's geometry before drawing base UI.
    pub fn push_open_layer(&mut self, layers: &mut LayerStack) -> Option<usize> {
        let geometry = self.geom?;
        if self.popup != Some(geometry.kind) {
            return None;
        }
        let index = layers.push_popup(popup_rect(geometry));
        layers.pop_layer();
        Some(index)
    }

    /// Close any open toolbar sheet.
    pub fn close_popup(&mut self) {
        self.popup = None;
        self.geom = None;
        self.next_geom = None;
    }

    /// Apply dismissal after [`draw_open_layer`](Self::draw_open_layer).
    pub fn end_frame(&mut self) {
        if self.escape || (self.mouse_clicked && !self.click_claimed) {
            self.close_popup();
        }
    }

    fn toggle_is_active(&self, id: u64) -> bool {
        self.active_toggles.contains(&id)
    }
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

/// Toolbar widget — a strip of tool buttons with separators and a grip handle.
pub struct Toolbar<'a> {
    items: &'a [ToolbarItem<'a>],
}

impl<'a> Toolbar<'a> {
    /// Create a toolbar from a slice of tool items.
    pub fn new(items: &'a [ToolbarItem<'a>]) -> Self {
        Self { items }
    }

    /// Compute the preferred size of the toolbar along the main axis.
    ///
    /// For a vertical toolbar this is the height; for horizontal, the width.
    /// Cross-axis size is `button_size + 2 * padding`.
    pub fn preferred_extent(&self, button_size: f32, padding: f32) -> f32 {
        self.preferred_extent_for_edge(button_size, padding, ToolbarEdge::Left)
    }

    /// Compute the preferred main-axis size for a particular dock edge.
    ///
    /// The 2px plinth travel contributes to a vertical toolbar's item height,
    /// but remains on the cross axis when the toolbar is horizontal.
    pub fn preferred_extent_for_edge(
        &self,
        button_size: f32,
        padding: f32,
        edge: ToolbarEdge,
    ) -> f32 {
        let travel = 2.0;
        let vertical = edge.is_vertical();
        let sep_thick = SEPARATOR_THICKNESS + SEPARATOR_GAP * 2.0;
        let mut extent = grip_main_extent(vertical);
        let mut children = 1;
        for item in self.items {
            extent += match item {
                ToolbarItem::Tool(_) | ToolbarItem::Toggle(_) => {
                    tool_main_extent(button_size, travel, vertical)
                }
                ToolbarItem::Separator => sep_thick,
            };
            children += 1;
        }
        extent + ITEM_GAP * (children - 1) as f32 + padding * 2.0
    }

    /// Compute the preferred cross-axis size for a vertical toolbar.
    pub fn preferred_cross(&self, button_size: f32, padding: f32) -> f32 {
        self.preferred_cross_for_edge(button_size, padding, ToolbarEdge::Left)
    }

    /// Compute the preferred cross-axis size for a particular dock edge.
    ///
    /// The dock-edge border participates in the rail's outer size rather than
    /// consuming one side of its centered content area. Horizontal rails also
    /// include the key plinth's travel on their cross axis.
    pub fn preferred_cross_for_edge(
        &self,
        button_size: f32,
        padding: f32,
        edge: ToolbarEdge,
    ) -> f32 {
        button_size
            + padding * 2.0
            + RAIL_EDGE_THICKNESS
            + if edge.is_vertical() { 0.0 } else { 2.0 }
    }

    /// Draw the toolbar using id `0`. Prefer [`draw_with_id`](Self::draw_with_id)
    /// when a surface has more than one toolbar or uses its popup sheets.
    pub fn draw(
        &self,
        rect: Rect,
        state: &mut ToolbarState,
        drag_capture: &mut DragCapture,
        grip_drag_id: DragId,
        ctx: &mut DrawContext,
    ) -> ToolbarOutput {
        self.draw_with_id(0, rect, state, drag_capture, grip_drag_id, ctx)
    }

    /// Draw the toolbar into `rect`, returning interaction intents and staging
    /// popup geometry for the next frame.
    pub fn draw_with_id(
        &self,
        id: ToolbarId,
        rect: Rect,
        state: &mut ToolbarState,
        drag_capture: &mut DragCapture,
        grip_drag_id: DragId,
        ctx: &mut DrawContext,
    ) -> ToolbarOutput {
        let s = ctx.styles();
        let toolbar_chrome = s.toolbar();
        let button_size = s.scalar(StyleKey::ToolbarButtonSize);
        let padding = s.scalar(StyleKey::ToolbarPadding);
        let travel = s.scalar(StyleKey::Travel);
        let vertical = state.edge.is_vertical();

        ctx.push_debug_scope_rect("Toolbar", rect);
        self.draw_rail(rect, state.edge, toolbar_chrome, ctx.draw_list);

        // The grip's layout slot follows the compact mark and its orientation-
        // specific design margins. Vertical rails have 2px before / 4px after;
        // horizontal rails have no leading margin and 2px after the 3px mark.
        let mut cursor = if vertical {
            rect.y + padding
        } else {
            rect.x + padding
        };
        // CSS `align-items: center` keeps keys centered across the rail even when
        // the caller gives the toolbar more than its preferred cross extent.
        // The theme padding determines the preferred extent, not a fixed inset.
        let cross_start = centered_cross_start(rect, state.edge, button_size, travel);

        let mut output = ToolbarOutput {
            id,
            clicked: None,
            event: None,
            hovered: None,
            grip_dragging: false,
            grip_delta: [0.0, 0.0],
            overflowed: 0,
            rect,
        };

        // --- Grip handle ---
        // The design's mark is 14×3 with 2px before and 4px after it. Keep the
        // full cross-axis rail as its hit target without inflating that layout slot.
        let grip_rect = if vertical {
            Rect::new(rect.x, cursor, rect.width, grip_main_extent(true))
        } else {
            Rect::new(cursor, rect.y, grip_main_extent(false), rect.height)
        };
        self.draw_grip(
            grip_rect,
            vertical,
            drag_capture,
            grip_drag_id,
            &mut output,
            ctx,
        );

        // Once the grip gesture has crossed the DragTracker's movement
        // threshold, snap the toolbar to whichever edge of the dock area the
        // pointer is closest to — classic drag-to-dock. Capture starts on the
        // press edge, so checking `is_dragging` here is what prevents a tap (or
        // sub-threshold pointer jitter) from unexpectedly changing edges.
        // `dock_area` is the space the toolbar can validly occupy (set by
        // AppShell to the rect remaining after menu bar, status bar, and
        // sidebars). Falls back to the full screen when unset.
        if output.grip_dragging && ctx.input.is_dragging {
            let area = if state.dock_area.width > 0.0 && state.dock_area.height > 0.0 {
                state.dock_area
            } else {
                Rect::new(
                    0.0,
                    0.0,
                    ctx.screen_width.max(1.0),
                    ctx.screen_height.max(1.0),
                )
            };
            let mx = ctx.input.mouse_x;
            let my = ctx.input.mouse_y;

            let distances = [
                (mx - area.x, ToolbarEdge::Left),
                (area.right() - mx, ToolbarEdge::Right),
                (my - area.y, ToolbarEdge::Top),
                (area.bottom() - my, ToolbarEdge::Bottom),
            ];
            let new_edge = distances
                .iter()
                .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
                .unwrap()
                .1;

            if new_edge != state.edge {
                state.edge = new_edge;
                output.event = Some(ToolbarEvent::DockChanged(new_edge));
                state.close_popup();
            }
        }

        cursor += grip_main_extent(vertical) + ITEM_GAP;

        // Reserve the trailing overflow control only when the complete strip does
        // not fit. An always-reserved key changes the handoff's composition even
        // for roomy rails and can manufacture overflow that did not exist before.
        let main_extent = if vertical { rect.height } else { rect.width };
        let needs_overflow = self.preferred_extent_for_edge(button_size, padding, state.edge)
            > main_extent + f32::EPSILON;
        let overflow_rect = trailing_button_rect(rect, state.edge, padding, button_size, travel);
        let main_limit = if needs_overflow {
            if vertical {
                overflow_rect.y - padding
            } else {
                overflow_rect.x - padding
            }
        } else if vertical {
            rect.bottom() - padding
        } else {
            rect.right() - padding
        };
        let mut overflow_started = false;
        let mut overflow_start = self.items.len();
        for (item_index, item) in self.items.iter().enumerate() {
            let item_extent = match item {
                ToolbarItem::Tool(_) | ToolbarItem::Toggle(_) => {
                    tool_main_extent(button_size, travel, vertical)
                }
                ToolbarItem::Separator => SEPARATOR_THICKNESS + SEPARATOR_GAP * 2.0,
            };
            if cursor + item_extent > main_limit {
                overflow_started = true;
                overflow_start = overflow_start.min(item_index);
            }
            if overflow_started {
                if !matches!(item, ToolbarItem::Separator) {
                    output.overflowed += 1;
                }
                continue;
            }
            match item {
                ToolbarItem::Tool(tool) | ToolbarItem::Toggle(tool) => {
                    let tool_rect = if vertical {
                        Rect::new(
                            cross_start,
                            cursor,
                            button_size,
                            tool_extent(button_size, travel),
                        )
                    } else {
                        Rect::new(
                            cursor,
                            cross_start,
                            button_size,
                            tool_extent(button_size, travel),
                        )
                    };
                    let kind = if matches!(item, ToolbarItem::Toggle(_)) {
                        ToolKind::Toggle
                    } else {
                        ToolKind::Modal
                    };
                    self.draw_tool_button(tool, kind, tool_rect, state, &mut output, ctx);
                    cursor += tool_main_extent(button_size, travel, vertical) + ITEM_GAP;
                }
                ToolbarItem::Separator => {
                    cursor += SEPARATOR_GAP;
                    let inset = padding + 2.0;
                    let sep_rect = if vertical {
                        Rect::new(
                            rect.x + inset,
                            cursor,
                            (rect.width - inset * 2.0).max(0.0),
                            SEPARATOR_THICKNESS,
                        )
                    } else {
                        Rect::new(
                            cursor,
                            rect.y + inset,
                            SEPARATOR_THICKNESS,
                            (rect.height - inset * 2.0).max(0.0),
                        )
                    };
                    let separator = toolbar_chrome.separator;
                    let rule = separator[0];
                    let counter = separator[1];
                    if vertical {
                        ctx.draw_list
                            .edge_line(sep_rect, Edge::Top, rule.thickness, rule.color);
                        ctx.draw_list.edge_line(
                            Rect::new(
                                sep_rect.x,
                                sep_rect.y + rule.thickness,
                                sep_rect.width,
                                counter.thickness,
                            ),
                            Edge::Top,
                            counter.thickness,
                            counter.color,
                        );
                    } else {
                        ctx.draw_list
                            .edge_line(sep_rect, Edge::Left, rule.thickness, rule.color);
                        ctx.draw_list.edge_line(
                            Rect::new(
                                sep_rect.x + rule.thickness,
                                sep_rect.y,
                                counter.thickness,
                                sep_rect.height,
                            ),
                            Edge::Left,
                            counter.thickness,
                            counter.color,
                        );
                    }
                    cursor += SEPARATOR_THICKNESS + SEPARATOR_GAP + ITEM_GAP;
                }
            }
        }

        if output.overflowed > 0 {
            self.draw_overflow_button(overflow_rect, state, &mut output, ctx);
        }

        // Right-click anywhere on the strip opens the dock-position chooser,
        // anchored to the grip. The design reserves left-click on the grip for
        // dragging only; the `⋯` overflow key is a separate left-click target.
        if !ctx.input.mouse_consumed
            && ctx.input.mouse_right_clicked
            && rect.contains(ctx.input.mouse_x, ctx.input.mouse_y)
        {
            state.popup = match state.popup {
                Some(PopupKind::Dock) => None,
                _ => Some(PopupKind::Dock),
            };
            state.click_claimed = true;
        }

        let viewport = Rect::new(
            0.0,
            0.0,
            ctx.screen_width.max(0.0),
            ctx.screen_height.max(0.0),
        );
        if let Some(kind) = state.popup {
            let anchor = if kind == PopupKind::Dock {
                grip_rect
            } else {
                overflow_rect
            };
            state.next_geom = Some(PopupGeometry {
                anchor,
                viewport,
                kind,
                overflow_start,
            });
        }
        ctx.pop_debug_scope();
        output
    }

    // --- Internal helpers ---

    fn draw_grip(
        &self,
        rect: Rect,
        vertical: bool,
        capture: &mut DragCapture,
        drag_id: DragId,
        output: &mut ToolbarOutput,
        ctx: &mut DrawContext,
    ) {
        let input = ctx.input;

        // Release-first protocol.
        if !input.mouse_down {
            capture.release(drag_id);
        }

        let hovered = !input.mouse_consumed && rect.contains(input.mouse_x, input.mouse_y);

        if hovered && input.mouse_clicked && capture.is_free() {
            capture.try_begin(drag_id);
        }

        let dragging = capture.is_active(drag_id);
        output.grip_dragging = dragging;
        if dragging {
            output.grip_delta = input.drag_delta;
        }

        // Compact repeating ridge mark, matching the docked rail design. The
        // resting ridges are a translucent white material, not theme text; the
        // dark counter-edge makes the three-pixel mark read against either end
        // of the rail gradient.
        let chrome = ctx.styles().toolbar();
        let ridge_color = chrome.grip_colors[if dragging {
            2
        } else if hovered {
            1
        } else {
            0
        }];
        let cx = rect.x + rect.width * 0.5;
        let cy = rect.y + rect.height * 0.5;
        if vertical {
            let x = cx - 7.0;
            let y = rect.y + GRIP_VERTICAL_MARGIN_START;
            for offset in [0.0_f32, 3.0, 6.0, 9.0, 12.0] {
                ctx.draw_list
                    .quad(x + offset, y, 1.0, GRIP_THICKNESS, ridge_color);
            }
            ctx.draw_list.edge_line(
                Rect::new(
                    x,
                    y,
                    14.0,
                    GRIP_THICKNESS + chrome.grip_counter_edge.thickness,
                ),
                Edge::Bottom,
                chrome.grip_counter_edge.thickness,
                chrome.grip_counter_edge.color,
            );
        } else {
            let x = rect.x;
            let y = cy - 7.0;
            for offset in [0.0_f32, 3.0, 6.0, 9.0, 12.0] {
                ctx.draw_list
                    .quad(x, y + offset, GRIP_THICKNESS, 1.0, ridge_color);
            }
            ctx.draw_list.edge_line(
                Rect::new(
                    x,
                    y,
                    GRIP_THICKNESS + chrome.grip_counter_edge.thickness,
                    14.0,
                ),
                Edge::Right,
                chrome.grip_counter_edge.thickness,
                chrome.grip_counter_edge.color,
            );
        }

        // Cursor.
        if hovered || dragging {
            let icon = if dragging {
                crate::CursorIcon::Grabbing
            } else {
                crate::CursorIcon::Grab
            };
            ctx.request_cursor(icon);
        }
    }

    fn draw_tool_button(
        &self,
        tool: &ToolDef<'_>,
        kind: ToolKind,
        rect: Rect,
        state: &ToolbarState,
        output: &mut ToolbarOutput,
        ctx: &mut DrawContext,
    ) {
        let input = ctx.input;
        let s = ctx.styles();

        let is_active = match kind {
            ToolKind::Modal => state.active_tool == Some(tool.id),
            ToolKind::Toggle => state.toggle_is_active(tool.id),
        };
        let hovered =
            tool.enabled && !input.mouse_consumed && rect.contains(input.mouse_x, input.mouse_y);
        let pressed = hovered && input.mouse_down;
        let clicked = hovered && input.mouse_clicked;

        let face = draw_tool_face(
            ctx.draw_list,
            &s,
            rect,
            is_active,
            hovered,
            pressed,
            tool.enabled,
        );

        // The handoff specifies a 12px glyph. Phosphor's MSDF tiles include
        // intrinsic font padding, so a 14px destination produces that visible
        // footprint while preserving each glyph's aspect ratio.
        const ICON_BOX: f32 = 14.0;
        let icon_size = ICON_BOX.min(face.width).min(face.height);
        let icon_rect = Rect::new(
            face.x + (face.width - icon_size) * 0.5,
            face.y + (face.height - icon_size) * 0.5,
            icon_size,
            icon_size,
        );
        let icon_tint = if is_active {
            hex(0xeafaff)
        } else if !tool.enabled {
            rgba8([0xb6, 0xbe, 0xc5], 0.45)
        } else if pressed {
            hex(0xb7bfc6)
        } else if hovered {
            hex(0xeef2f6)
        } else {
            hex(0xb6bec5)
        };
        tool.icon.tint(icon_tint).draw(icon_rect, ctx.draw_list);

        // Report hover and click.
        if hovered {
            output.hovered = Some(tool.id);
            ctx.request_cursor(crate::CursorIcon::Pointer);
        }
        if clicked && tool.enabled {
            output.clicked = Some(tool.id);
            output.event = Some(match kind {
                ToolKind::Modal => ToolbarEvent::ToolActivated(tool.id),
                ToolKind::Toggle => ToolbarEvent::ToggleActivated(tool.id),
            });
        }
    }

    fn draw_rail(
        &self,
        rect: Rect,
        edge: ToolbarEdge,
        chrome: crate::ToolbarChrome,
        list: &mut DrawList,
    ) {
        let outward = match edge {
            ToolbarEdge::Left => [1.0, 0.0],
            ToolbarEdge::Right => [-1.0, 0.0],
            ToolbarEdge::Top => [0.0, 1.0],
            ToolbarEdge::Bottom => [0.0, -1.0],
        };
        let shadow = BoxShadow {
            offset: [
                outward[0] * chrome.rail_shadow.offset[1],
                outward[1] * chrome.rail_shadow.offset[1],
            ],
            ..chrome.rail_shadow
        };
        let dock_edge = match edge {
            ToolbarEdge::Left => Edge::Right,
            ToolbarEdge::Right => Edge::Left,
            ToolbarEdge::Top => Edge::Bottom,
            ToolbarEdge::Bottom => Edge::Top,
        };
        let style = QuadStyle {
            background: Background::LinearGradient {
                start: chrome.rail_colors[0],
                end: chrome.rail_colors[1],
                axis: if edge.is_vertical() {
                    crate::GradientAxis::Horizontal
                } else {
                    crate::GradientAxis::Vertical
                },
            },
            ..Default::default()
        };
        let lines = [
            StructuralLine {
                edge: dock_edge,
                offset: 0.0,
                style: chrome.dock_edge,
            },
            StructuralLine {
                edge: dock_edge,
                offset: chrome.dock_edge.thickness,
                style: chrome.dock_highlight,
            },
        ];
        let mut painter = SurfacePainter::new(
            list,
            rect,
            rect,
            CornerRadii::default(),
            style,
            std::slice::from_ref(&shadow),
            &lines,
        );
        painter.paint_pre_content_opaque();
        painter.paint_post_content();
    }

    fn draw_overflow_button(
        &self,
        rect: Rect,
        state: &mut ToolbarState,
        output: &mut ToolbarOutput,
        ctx: &mut DrawContext,
    ) {
        let hovered =
            !ctx.input.mouse_consumed && rect.contains(ctx.input.mouse_x, ctx.input.mouse_y);
        let material = Material::new(Tone::Ghost)
            .hovered(hovered)
            .pressed(hovered && ctx.input.mouse_down);
        let face = material::draw_with_radius(
            ctx.draw_list,
            &ctx.styles(),
            rect,
            ctx.styles().scalar(StyleKey::BorderRadius),
            &material,
        );
        let color = if hovered {
            ctx.styles().color(StyleKey::Text)
        } else {
            ctx.styles().color(StyleKey::TextDim)
        };
        let y =
            ctx.draw_list
                .vcentered_text_y(face.y, face.height, 12.0, ctx.theme.font.as_ref(), "…");
        ctx.draw_list.text(
            ctx.styles()
                .text_block("…", face.x + (face.width - 8.0) * 0.5, y)
                .with_size(12.0)
                .with_color_f32(color)
                .with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0),
        );
        if hovered {
            ctx.request_cursor(crate::CursorIcon::Pointer);
        }
        if hovered && ctx.input.mouse_clicked {
            state.popup = match state.popup {
                Some(PopupKind::Overflow) => None,
                _ => Some(PopupKind::Overflow),
            };
            state.click_claimed = true;
            output.event = None;
        }
    }
}

#[derive(Clone, Copy)]
enum ToolKind {
    Modal,
    Toggle,
}

/// Draw one toolbar face from the toolbar-specific values in the design source.
/// Unlike the reusable material helper, an idle toolbar key has a translucent
/// neutral face rather than becoming fully transparent.
#[allow(clippy::too_many_arguments)]
fn draw_tool_face(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    active: bool,
    hovered: bool,
    pressed: bool,
    enabled: bool,
) -> Rect {
    if !active {
        return draw_neutral_tool(list, s, rect, hovered, pressed, enabled);
    }
    let travel = s.scalar(StyleKey::Travel);
    let face = Rect::new(
        rect.x,
        rect.y + travel,
        rect.width,
        (rect.height - travel).max(0.0),
    );
    paint_tool_surface(
        list,
        face,
        s.toolbar().tool_latched,
        &s.toolbar().tool_insets[3],
        enabled,
    );
    face
}

fn draw_neutral_tool(
    list: &mut DrawList,
    s: &StyleResolver,
    rect: Rect,
    hovered: bool,
    pressed: bool,
    enabled: bool,
) -> Rect {
    let chrome = s.toolbar();
    let dropped = enabled && pressed;
    let state = if dropped {
        2
    } else if hovered {
        1
    } else {
        0
    };
    let style = [chrome.tool_idle, chrome.tool_hover, chrome.tool_pressed][state];
    let travel = s.scalar(StyleKey::Travel);
    let face = Rect::new(
        rect.x,
        rect.y + if dropped { travel } else { 0.0 },
        rect.width,
        (rect.height - travel).max(0.0),
    );
    paint_tool_surface(list, face, style, &chrome.tool_insets[state], enabled);
    face
}

fn paint_tool_surface(
    list: &mut DrawList,
    rect: Rect,
    mut style: QuadStyle,
    authored_shadows: &[BoxShadow],
    enabled: bool,
) {
    let mut shadows = [BoxShadow::default(); 2];
    shadows.copy_from_slice(authored_shadows);
    if !enabled {
        const DISABLED_ALPHA: f32 = 0.45;
        let dim = |mut color: [f32; 4]| {
            color[3] *= DISABLED_ALPHA;
            color
        };
        style.background = match style.background {
            Background::Solid(color) => Background::Solid(dim(color)),
            Background::LinearGradient { start, end, axis } => Background::LinearGradient {
                start: dim(start),
                end: dim(end),
                axis,
            },
        };
        style.border_color = dim(style.border_color);
        for shadow in &mut shadows {
            shadow.color = dim(shadow.color);
        }
    }
    let padding = rect.inset(style.border_widths.left.max(style.border_widths.top));
    let mut painter = SurfacePainter::new(
        list,
        rect,
        padding,
        style.corner_radii,
        style,
        &shadows,
        &[],
    );
    painter.paint_pre_content();
    painter.paint_post_content();
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// Outcome of drawing a [`Toolbar`].
#[derive(Debug, Clone, Copy)]
pub struct ToolbarOutput {
    /// Stable identity of the toolbar that produced this output.
    pub id: ToolbarId,
    /// Backward-compatible id of the tool activated this frame, if any.
    pub clicked: Option<u64>,
    /// Typed toolbar intent. Apply it to caller-owned application state.
    pub event: Option<ToolbarEvent>,
    /// Tool that is hovered this frame (for external [`TooltipLayer`](crate::TooltipLayer)).
    pub hovered: Option<u64>,
    /// Whether the grip handle is being dragged.
    pub grip_dragging: bool,
    /// Drag delta from the grip handle this frame.
    pub grip_delta: [f32; 2],
    /// Number of non-separator items available from the overflow sheet.
    pub overflowed: usize,
    /// The rect consumed by the toolbar.
    pub rect: Rect,
}

fn centered_cross_start(rect: Rect, edge: ToolbarEdge, size: f32, travel: f32) -> f32 {
    let vertical = edge.is_vertical();
    let (cross_origin, cross_extent) = if vertical {
        (rect.x, rect.width)
    } else {
        (rect.y, rect.height)
    };
    let content_extent = if vertical {
        size
    } else {
        tool_extent(size, travel)
    };
    let interior_origin = if matches!(edge, ToolbarEdge::Right | ToolbarEdge::Bottom) {
        cross_origin + RAIL_EDGE_THICKNESS
    } else {
        cross_origin
    };
    let interior_extent = (cross_extent - RAIL_EDGE_THICKNESS).max(0.0);
    interior_origin + (interior_extent - content_extent) * 0.5
}

fn trailing_button_rect(
    rect: Rect,
    edge: ToolbarEdge,
    padding: f32,
    size: f32,
    travel: f32,
) -> Rect {
    if edge.is_vertical() {
        Rect::new(
            centered_cross_start(rect, edge, size, travel),
            (rect.bottom() - padding - size).max(rect.y),
            size.min((rect.width - RAIL_EDGE_THICKNESS).max(0.0)),
            size,
        )
    } else {
        Rect::new(
            (rect.right() - padding - size).max(rect.x),
            centered_cross_start(rect, edge, size, travel),
            size,
            size.min((rect.height - RAIL_EDGE_THICKNESS).max(0.0)),
        )
    }
}

fn popup_rect(geometry: PopupGeometry) -> Rect {
    let size = match geometry.kind {
        PopupKind::Dock => [124.0, 108.0],
        PopupKind::Overflow => [160.0, 240.0],
    };
    let mut x = geometry.anchor.right() + 7.0;
    let mut y = geometry.anchor.y;
    if x + size[0] > geometry.viewport.right() {
        x = geometry.anchor.x - size[0] - 7.0;
    }
    if x < geometry.viewport.x {
        x = geometry.viewport.x;
    }
    if y + size[1] > geometry.viewport.bottom() {
        y = geometry.viewport.bottom() - size[1];
    }
    Rect::new(x, y.max(geometry.viewport.y), size[0], size[1])
}

impl ToolbarState {
    /// Draw the open dock or overflow sheet into its popup layer. `items` must be
    /// the same borrowed item sequence passed to the toolbar's base draw.
    pub fn draw_open_layer(
        &mut self,
        layers: &mut LayerStack,
        popup: Option<usize>,
        items: &[ToolbarItem<'_>],
        style: &StyleResolver,
        input: &InputState,
    ) -> Option<ToolbarEvent> {
        let geometry = self.geom?;
        if self.popup != Some(geometry.kind) {
            return None;
        }
        let rect = popup_rect(geometry);
        let index = match popup {
            Some(index) => index,
            None => {
                let index = layers.push_popup(rect);
                layers.pop_layer();
                index
            }
        };
        let layer_input = layers.input_for_layer(index, input);
        let mut event = None;
        {
            let list = &mut layers.layers_mut()[index].list;
            list.push_debug_scope_rect(
                "Toolbar popup",
                Rect::new(
                    rect.x - 12.0,
                    rect.y - 12.0,
                    rect.width + 24.0,
                    rect.height + 24.0,
                ),
            );
            let chrome = style.toolbar().popup;
            let border = chrome.surface.border_widths;
            let padding = Rect::new(
                rect.x + border.left,
                rect.y + border.top,
                (rect.width - border.left - border.right).max(0.0),
                (rect.height - border.top - border.bottom).max(0.0),
            );
            let mut painter = SurfacePainter::new(
                list,
                rect,
                padding,
                chrome.surface.corner_radii,
                chrome.surface,
                &chrome.shadows,
                &chrome.lines,
            );
            painter.paint_pre_content();
            painter.paint_post_content();
            let title = match geometry.kind {
                PopupKind::Dock => "Dock toolbar",
                PopupKind::Overflow => "More tools",
            };
            let title_y =
                list.vcentered_text_y(rect.y + 3.0, 18.0, 10.0, style.theme().font.as_ref(), title);
            list.text(
                style
                    .text_block(title, rect.x + 8.0, title_y)
                    .with_size(10.0)
                    .with_color(220, 225, 230)
                    .with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0),
            );
            match geometry.kind {
                PopupKind::Dock => {
                    for (i, edge) in [
                        ToolbarEdge::Top,
                        ToolbarEdge::Right,
                        ToolbarEdge::Bottom,
                        ToolbarEdge::Left,
                    ]
                    .iter()
                    .copied()
                    .enumerate()
                    {
                        let row = Rect::new(
                            rect.x + 4.0,
                            rect.y + 24.0 + i as f32 * 20.0,
                            rect.width - 8.0,
                            18.0,
                        );
                        let hovered = !layer_input.mouse_consumed
                            && row.contains(layer_input.mouse_x, layer_input.mouse_y);
                        if hovered {
                            list.quad(
                                row.x,
                                row.y,
                                row.width,
                                row.height,
                                style.color(StyleKey::Accent),
                            );
                        }
                        let label = match edge {
                            ToolbarEdge::Top => "Top",
                            ToolbarEdge::Right => "Right",
                            ToolbarEdge::Bottom => "Bottom",
                            ToolbarEdge::Left => "Left",
                        };
                        let mark = if self.edge == edge { "•" } else { " " };
                        let y = list.vcentered_text_y(
                            row.y,
                            row.height,
                            11.0,
                            style.theme().font.as_ref(),
                            label,
                        );
                        list.text(
                            style
                                .text_block(mark, row.x + 4.0, y)
                                .with_size(11.0)
                                .with_color(140, 210, 215)
                                .with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0),
                        );
                        list.text(
                            style
                                .text_block(label, row.x + 18.0, y)
                                .with_size(11.0)
                                .with_color(225, 230, 235)
                                .with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0),
                        );
                        if hovered && layer_input.mouse_clicked {
                            event = Some(ToolbarEvent::DockChanged(edge));
                            self.click_claimed = true;
                            self.close_popup();
                        }
                    }
                }
                PopupKind::Overflow => {
                    let mut row_index = 0;
                    for item in items.iter().skip(geometry.overflow_start) {
                        let tool = match item {
                            ToolbarItem::Tool(tool) | ToolbarItem::Toggle(tool) => tool,
                            ToolbarItem::Separator => continue,
                        };
                        let row = Rect::new(
                            rect.x + 4.0,
                            rect.y + 24.0 + row_index as f32 * 20.0,
                            rect.width - 8.0,
                            18.0,
                        );
                        row_index += 1;
                        if row.bottom() > rect.bottom() - 4.0 {
                            break;
                        }
                        let hovered = tool.enabled
                            && !layer_input.mouse_consumed
                            && row.contains(layer_input.mouse_x, layer_input.mouse_y);
                        if hovered {
                            list.quad(
                                row.x,
                                row.y,
                                row.width,
                                row.height,
                                style.color(StyleKey::ButtonHover),
                            );
                        }
                        let y = list.vcentered_text_y(
                            row.y,
                            row.height,
                            11.0,
                            style.theme().font.as_ref(),
                            tool.label,
                        );
                        list.text(
                            style
                                .text_block(tool.label, row.x + 7.0, y)
                                .with_size(11.0)
                                .with_color(225, 230, 235)
                                .with_shadow(0, 0, 0, 128, 0.0, -1.0, 0.0),
                        );
                        if hovered && layer_input.mouse_clicked {
                            event = Some(match item {
                                ToolbarItem::Tool(_) => ToolbarEvent::ToolActivated(tool.id),
                                ToolbarItem::Toggle(_) => ToolbarEvent::ToggleActivated(tool.id),
                                ToolbarItem::Separator => unreachable!(),
                            });
                            self.click_claimed = true;
                            self.close_popup();
                        }
                    }
                }
            }
            list.pop_debug_scope();
        }
        if layer_input.mouse_clicked && rect.contains(layer_input.mouse_x, layer_input.mouse_y) {
            self.click_claimed = true;
        }
        event
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Main-axis dimensions copied from the toolbar mockup's grip and flex gap.
const GRIP_THICKNESS: f32 = 3.0;
const GRIP_VERTICAL_MARGIN_START: f32 = 2.0;
const GRIP_VERTICAL_MARGIN_END: f32 = 4.0;
const GRIP_HORIZONTAL_MARGIN_END: f32 = 2.0;
const ITEM_GAP: f32 = 2.0;

/// Dock-edge rule painted inside the toolbar's outer rectangle.
const RAIL_EDGE_THICKNESS: f32 = 1.0;

/// Thickness of a separator line.
const SEPARATOR_THICKNESS: f32 = 1.0;

/// Gap on each side of a separator.
const SEPARATOR_GAP: f32 = 3.0;

fn grip_main_extent(vertical: bool) -> f32 {
    if vertical {
        GRIP_VERTICAL_MARGIN_START + GRIP_THICKNESS + GRIP_VERTICAL_MARGIN_END
    } else {
        GRIP_THICKNESS + GRIP_HORIZONTAL_MARGIN_END
    }
}

fn tool_extent(button_size: f32, travel: f32) -> f32 {
    button_size + travel
}

fn tool_main_extent(button_size: f32, travel: f32, vertical: bool) -> f32 {
    if vertical {
        tool_extent(button_size, travel)
    } else {
        button_size
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;
    use crate::color::rgb8;
    use crate::layout::Rect;
    use crate::style::StyleOverlay;
    use crate::widgets::DrawList;
    use crate::widgets::drag::DragCapture;
    use crate::widgets::focus::FocusState;

    /// Top colour of the latched tool face gradient in `theme`.
    fn latched_top(theme: &Theme) -> [f32; 4] {
        match theme.chrome.toolbar.tool_latched.background {
            Background::LinearGradient { start, .. } => start,
            other => panic!("latched face should be a gradient, got {other:?}"),
        }
    }

    #[test]
    fn latched_tool_face_uses_the_forge_latch_tokens() {
        use crate::color::oklch;
        let toolbar = Theme::default().chrome.toolbar;
        assert_eq!(
            toolbar.tool_latched.background,
            Background::LinearGradient {
                start: oklch(0.62, 0.1, 200.0, 1.0),
                end: oklch(0.7, 0.11, 200.0, 1.0),
                axis: crate::chrome::GradientAxis::Vertical,
            }
        );
        let [shade, hi] = toolbar.tool_insets[3];
        assert_eq!(shade.color, oklch(0.32, 0.07, 200.0, 1.0));
        assert_eq!((shade.offset, shade.blur), ([0.0, 2.0], 4.0));
        assert_eq!(hi.color, oklch(0.55, 0.09, 200.0, 1.0));
        assert_eq!((hi.offset, hi.blur), ([0.0, 1.0], 0.0));
    }

    fn ctx<'a>(
        list: &'a mut DrawList,
        focus: &'a mut FocusState,
        theme: &'a Theme,
        input: &'a crate::InputState,
    ) -> DrawContext<'a> {
        DrawContext::new(list, focus, theme, input, 800.0, 600.0)
    }

    fn sample_items() -> Vec<ToolbarItem<'static>> {
        vec![
            ToolbarItem::tool(1, Icon::new(crate::render::PhosphorIcon::Plus), "Add", "A"),
            ToolbarItem::tool(
                2,
                Icon::new(crate::render::PhosphorIcon::Minus),
                "Remove",
                "D",
            ),
            ToolbarItem::separator(),
            ToolbarItem::tool(
                3,
                Icon::new(crate::render::PhosphorIcon::Gear),
                "Settings",
                "S",
            ),
        ]
    }

    #[test]
    fn clicking_a_tool_reports_its_id() {
        let items = sample_items();
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        let mut capture = DragCapture::new();
        let rect = Rect::new(0.0, 0.0, 28.0, 200.0);

        // The first tool button follows the 9px grip slot and 2px flex gap.
        let tool_y = 2.0 + grip_main_extent(true) + ITEM_GAP + 13.0;
        let tool_x = 14.0;

        let mut input = crate::InputState::default();
        input.mouse_x = tool_x;
        input.mouse_y = tool_y;
        input.mouse_clicked = true;
        input.mouse_down = true;

        let mut cx = ctx(&mut list, &mut focus, &theme, &input);
        let out = Toolbar::new(&items).draw(rect, &mut state, &mut capture, 99, &mut cx);
        assert_eq!(out.clicked, Some(1), "should report first tool's id");
    }

    #[test]
    fn active_tool_uses_the_designs_held_face_in_dropped_position() {
        let items = sample_items();
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        state.active_tool = Some(2);
        let mut capture = DragCapture::new();
        let rect = Rect::new(0.0, 0.0, 28.0, 200.0);
        let input = crate::InputState::default();

        let mut cx = ctx(&mut list, &mut focus, &theme, &input);
        Toolbar::new(&items).draw(rect, &mut state, &mut capture, 99, &mut cx);

        let held_top = latched_top(&theme);
        let held = list
            .chrome_instances()
            .find(|instance| instance.bg == held_top)
            .expect("selected tool should use the toolbar-specific held gradient");
        let second_tool_y = theme.toolbar_padding
            + grip_main_extent(true)
            + ITEM_GAP
            + tool_extent(theme.toolbar_button_size, theme.travel)
            + ITEM_GAP;
        assert_eq!(held.rect[1], second_tool_y + theme.travel);
        assert_eq!(held.rect[3], theme.toolbar_button_size);
    }

    #[test]
    fn grip_uses_design_material_and_black_counter_edge() {
        let items = sample_items();
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        let mut capture = DragCapture::new();
        let input = crate::InputState::default();

        Toolbar::new(&items).draw(
            Rect::new(0.0, 0.0, 28.0, 200.0),
            &mut state,
            &mut capture,
            99,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );

        let ridge_instances = list
            .chrome_instances()
            .filter(|instance| instance.bg == [1.0, 1.0, 1.0, 0.28])
            .count();
        assert_eq!(
            ridge_instances, 5,
            "five grip ridges use the 28% white material"
        );
        assert!(
            list.chrome_instances()
                .any(|instance| instance.bg == [0.0, 0.0, 0.0, 0.5]),
            "grip has the design's solid-black counter-edge"
        );
    }

    #[test]
    fn idle_tool_keeps_the_designs_neutral_face() {
        let items = sample_items();
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        let mut capture = DragCapture::new();
        let input = crate::InputState::default();

        Toolbar::new(&items).draw(
            Rect::new(0.0, 0.0, 28.0, 200.0),
            &mut state,
            &mut capture,
            99,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );

        let top = rgb8([0x41, 0x44, 0x48]);
        let bottom = rgb8([0x2a, 0x2e, 0x33]);
        let border = rgb8([0x10, 0x12, 0x15]);
        assert!(
            list.chrome_instances()
                .any(|instance| instance.bg == top && instance.bg2 == bottom),
            "resting tools use the typed handoff face gradient"
        );
        assert!(
            list.chrome_instances()
                .any(|instance| instance.border == border && instance.widths == [1.0; 4]),
            "resting tools use the typed handoff border"
        );
    }

    #[test]
    fn disabled_tool_does_not_report_click() {
        let items = vec![ToolbarItem::Tool(ToolDef {
            id: 1,
            icon: Icon::new(crate::render::PhosphorIcon::Plus),
            label: "Add",
            shortcut: "A",
            enabled: false,
        })];
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        let mut capture = DragCapture::new();
        let rect = Rect::new(0.0, 0.0, 28.0, 200.0);

        let tool_y = 2.0 + grip_main_extent(true) + ITEM_GAP + 13.0;
        let mut input = crate::InputState::default();
        input.mouse_x = 14.0;
        input.mouse_y = tool_y;
        input.mouse_clicked = true;
        input.mouse_down = true;

        let mut cx = ctx(&mut list, &mut focus, &theme, &input);
        let out = Toolbar::new(&items).draw(rect, &mut state, &mut capture, 99, &mut cx);
        assert_eq!(out.clicked, None, "disabled tool should not report click");
    }

    #[test]
    fn consumed_input_suppresses_hover() {
        let items = sample_items();
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        let mut capture = DragCapture::new();
        let rect = Rect::new(0.0, 0.0, 28.0, 200.0);

        let tool_y = 2.0 + grip_main_extent(true) + ITEM_GAP + 13.0;
        let mut input = crate::InputState::default();
        input.mouse_x = 14.0;
        input.mouse_y = tool_y;
        input.mouse_consumed = true;

        let mut cx = ctx(&mut list, &mut focus, &theme, &input);
        let out = Toolbar::new(&items).draw(rect, &mut state, &mut capture, 99, &mut cx);
        assert_eq!(out.hovered, None, "consumed input should suppress hover");
    }

    #[test]
    fn horizontal_toolbar_lays_out_left_to_right() {
        let items = sample_items();
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Top);
        let mut capture = DragCapture::new();
        let rect = Rect::new(0.0, 0.0, 300.0, 28.0);

        // Click the midpoint of the first tool face after the compact grip slot.
        let tool_x = 2.0 + grip_main_extent(false) + ITEM_GAP + 13.0;
        let tool_y = 14.0;

        let mut input = crate::InputState::default();
        input.mouse_x = tool_x;
        input.mouse_y = tool_y;
        input.mouse_clicked = true;
        input.mouse_down = true;

        let mut cx = ctx(&mut list, &mut focus, &theme, &input);
        let out = Toolbar::new(&items).draw(rect, &mut state, &mut capture, 99, &mut cx);
        assert_eq!(
            out.clicked,
            Some(1),
            "horizontal layout should find first tool"
        );
    }

    #[test]
    fn grip_drag_reports_delta() {
        let items = sample_items();
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        let mut capture = DragCapture::new();
        let rect = Rect::new(0.0, 0.0, 28.0, 200.0);

        // Click in the grip zone.
        let mut input = crate::InputState::default();
        input.mouse_x = 14.0;
        input.mouse_y = 2.0 + grip_main_extent(true) * 0.5;
        input.mouse_clicked = true;
        input.mouse_down = true;
        input.drag_delta = [5.0, 3.0];

        let mut cx = ctx(&mut list, &mut focus, &theme, &input);
        let out = Toolbar::new(&items).draw(rect, &mut state, &mut capture, 99, &mut cx);
        assert!(out.grip_dragging, "grip should be dragging");
        assert_eq!(out.grip_delta, [5.0, 3.0]);
    }

    #[test]
    fn grip_only_snaps_after_drag_threshold_is_crossed() {
        let items = sample_items();
        let toolbar = Toolbar::new(&items);
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        state.dock_area = Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut capture = DragCapture::new();
        let mut tracker = crate::DragTracker::new();
        let rect = Rect::new(0.0, 0.0, 28.0, 200.0);
        let grip_y = theme.toolbar_padding + grip_main_extent(true) * 0.5;

        // Pressing near the top edge captures the grip, but is not a drag yet.
        // Without the threshold gate, nearest-edge docking would jump to Top.
        let mut input = crate::InputState {
            mouse_x: 14.0,
            mouse_y: grip_y,
            mouse_clicked: true,
            mouse_down: true,
            ..Default::default()
        };
        tracker.update(&mut input);
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let out = toolbar.draw(
            rect,
            &mut state,
            &mut capture,
            99,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(out.grip_dragging, "the pressed grip should own capture");
        assert!(!input.is_dragging);
        assert_eq!(state.edge, ToolbarEdge::Left);
        assert_eq!(out.event, None);

        // Sub-threshold jitter must likewise leave the dock edge alone.
        input.mouse_clicked = false;
        input.mouse_x = 16.0;
        tracker.update(&mut input);
        let mut list = DrawList::new();
        let out = toolbar.draw(
            rect,
            &mut state,
            &mut capture,
            99,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(!input.is_dragging);
        assert_eq!(state.edge, ToolbarEdge::Left);
        assert_eq!(out.event, None);

        // Crossing DragTracker's default 4px threshold enables nearest-edge
        // docking for the remainder of the held gesture.
        input.mouse_x = 20.0;
        tracker.update(&mut input);
        let mut list = DrawList::new();
        let out = toolbar.draw(
            rect,
            &mut state,
            &mut capture,
            99,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(input.is_dragging);
        assert_eq!(state.edge, ToolbarEdge::Top);
        assert_eq!(out.event, Some(ToolbarEvent::DockChanged(ToolbarEdge::Top)));
    }

    #[test]
    fn preferred_extent_accounts_for_all_items() {
        let items = sample_items();
        let toolbar = Toolbar::new(&items);
        let btn = 24.0;
        let pad = 2.0;
        // 9px vertical grip + four 2px flex gaps + three 26px plinth boxes + separator.
        let expected = grip_main_extent(true)
            + 4.0 * ITEM_GAP
            + 3.0 * tool_extent(btn, 2.0)
            + (SEPARATOR_THICKNESS + SEPARATOR_GAP * 2.0)
            + pad * 2.0;
        let actual = toolbar.preferred_extent(btn, pad);
        assert!(
            (actual - expected).abs() < 0.01,
            "expected {expected}, got {actual}"
        );

        let horizontal_expected = grip_main_extent(false)
            + 4.0 * ITEM_GAP
            + 3.0 * btn
            + (SEPARATOR_THICKNESS + SEPARATOR_GAP * 2.0)
            + pad * 2.0;
        let horizontal = toolbar.preferred_extent_for_edge(btn, pad, ToolbarEdge::Top);
        assert!(
            (horizontal - horizontal_expected).abs() < 0.01,
            "expected horizontal {horizontal_expected}, got {horizontal}"
        );
    }

    #[test]
    fn horizontal_tool_face_is_centered_in_a_tall_rail() {
        let items = sample_items();
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Top);
        let mut capture = DragCapture::new();
        let input = crate::InputState::default();
        let rail = Rect::new(0.0, 0.0, 200.0, 36.0);

        Toolbar::new(&items).draw(
            rail,
            &mut state,
            &mut capture,
            99,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );

        let idle_top = rgb8([0x41, 0x44, 0x48]);
        let face = list
            .chrome_instances()
            .find(|instance| instance.bg == idle_top)
            .expect("idle horizontal tool face");
        assert_eq!(face.rect[2], theme.toolbar_button_size);
        assert_eq!(face.rect[3], theme.toolbar_button_size);
        assert_eq!(
            face.rect[1],
            (rail.height - RAIL_EDGE_THICKNESS - (theme.toolbar_button_size + theme.travel)) * 0.5
        );
        assert_eq!(list.icons_msdf[0].local.width, 14.0);
        assert_eq!(list.icons_msdf[0].local.height, 14.0);
        assert_eq!(list.icons_msdf[0].local.x, face.rect[0] + 5.0);
        assert_eq!(list.icons_msdf[0].local.y, face.rect[1] + 5.0);
    }

    #[test]
    fn vertical_tool_face_is_centered_in_a_wide_rail() {
        let items = sample_items();
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        state.active_tool = Some(1);
        let mut capture = DragCapture::new();
        let input = crate::InputState::default();
        let rail = Rect::new(0.0, 0.0, 36.0, 200.0);

        Toolbar::new(&items).draw(
            rail,
            &mut state,
            &mut capture,
            99,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );

        let held_top = latched_top(&theme);
        let held = list
            .chrome_instances()
            .find(|instance| instance.bg == held_top)
            .expect("selected vertical tool face");
        assert_eq!(
            held.rect[0],
            (rail.width - RAIL_EDGE_THICKNESS - theme.toolbar_button_size) * 0.5
        );
        assert_eq!(held.rect[2], theme.toolbar_button_size);
    }

    #[test]
    fn overflow_button_is_centered_across_an_oversized_rail() {
        let vertical = trailing_button_rect(
            Rect::new(10.0, 20.0, 36.0, 100.0),
            ToolbarEdge::Left,
            3.0,
            24.0,
            2.0,
        );
        assert_eq!(vertical.x, 15.5);
        assert_eq!(vertical.width, 24.0);

        let horizontal = trailing_button_rect(
            Rect::new(10.0, 20.0, 100.0, 36.0),
            ToolbarEdge::Top,
            3.0,
            24.0,
            2.0,
        );
        assert_eq!(horizontal.y, 24.5);
        assert_eq!(horizontal.height, 24.0);
    }

    #[test]
    fn fitting_rail_does_not_draw_or_reserve_an_overflow_key() {
        let items = sample_items();
        let toolbar = Toolbar::new(&items);
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Top);
        let mut capture = DragCapture::new();
        let input = crate::InputState::default();
        let width = toolbar.preferred_extent_for_edge(
            theme.toolbar_button_size,
            theme.toolbar_padding,
            ToolbarEdge::Top,
        );

        let output = toolbar.draw(
            Rect::new(0.0, 0.0, width, 30.0),
            &mut state,
            &mut capture,
            99,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );

        assert_eq!(output.overflowed, 0);
        assert!(
            list.texts.iter().all(|text| text.content != "…"),
            "a fitting rail must match the handoff and end after its final tool"
        );
    }

    #[test]
    fn small_rail_reserves_overflow_button_instead_of_silently_losing_tools() {
        let items = sample_items();
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        let mut capture = DragCapture::new();
        let input = crate::InputState::default();
        let mut cx = ctx(&mut list, &mut focus, &theme, &input);
        let output = Toolbar::new(&items).draw(
            Rect::new(0.0, 0.0, 28.0, 70.0),
            &mut state,
            &mut capture,
            99,
            &mut cx,
        );
        assert_eq!(output.overflowed, 2);
        assert!(
            list.chrome_instance_count() != 0,
            "rail gradient should use composable quad geometry"
        );
    }

    #[test]
    fn toggle_reports_typed_event_without_mutating_caller_state() {
        let items = [ToolbarItem::toggle(
            9,
            Icon::new(crate::render::PhosphorIcon::Plus),
            "Snap",
            "G",
        )];
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let theme = Theme::default();
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        let mut capture = DragCapture::new();
        let mut input = crate::InputState::default();
        input.mouse_x = 14.0;
        input.mouse_y = 30.0;
        input.mouse_clicked = true;
        let mut cx = ctx(&mut list, &mut focus, &theme, &input);
        let output = Toolbar::new(&items).draw(
            Rect::new(0.0, 0.0, 28.0, 100.0),
            &mut state,
            &mut capture,
            99,
            &mut cx,
        );
        assert_eq!(output.event, Some(ToolbarEvent::ToggleActivated(9)));
        assert!(
            state.active_toggles.is_empty(),
            "caller applies toggle state"
        );
    }

    #[test]
    fn rail_shadow_uses_authored_elevation_in_each_dock_direction() {
        let theme = Theme::default();
        for (edge, expected) in [
            (ToolbarEdge::Left, [2.0, 0.0]),
            (ToolbarEdge::Right, [-2.0, 0.0]),
            (ToolbarEdge::Top, [0.0, 2.0]),
            (ToolbarEdge::Bottom, [0.0, -2.0]),
        ] {
            let mut list = DrawList::new();
            Toolbar::new(&[]).draw_rail(
                Rect::new(10.0, 20.0, 30.0, 40.0),
                edge,
                theme.chrome.toolbar,
                &mut list,
            );
            let shadow = &list.shadow_instance(0).unwrap();
            let source = shadow.shadow_rect;
            assert_eq!(
                [
                    source[0] - shadow.element_rect[0],
                    source[1] - shadow.element_rect[1]
                ],
                expected
            );
            assert_eq!(
                shadow.params[0],
                theme.chrome.toolbar.rail_shadow.blur * 0.5
            );
        }
    }

    #[test]
    fn typed_overlay_reaches_toolbar_rail_tools_and_popup() {
        let theme = Theme::default();
        let mut chrome = theme.chrome.toolbar;
        chrome.rail_colors = [[0.11, 0.12, 0.13, 1.0], [0.21, 0.22, 0.23, 1.0]];
        chrome.tool_idle.background = Background::Solid([0.31, 0.32, 0.33, 1.0]);
        chrome.popup.surface.background = Background::Solid([0.41, 0.42, 0.43, 1.0]);
        let mut overlay = StyleOverlay::new();
        overlay.set_toolbar(chrome);
        let items = sample_items();
        let input = crate::InputState::default();
        let mut list = DrawList::new();
        let mut focus = FocusState::default();
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        let mut capture = DragCapture::new();
        Toolbar::new(&items).draw(
            Rect::new(0.0, 0.0, 28.0, 200.0),
            &mut state,
            &mut capture,
            99,
            &mut ctx(&mut list, &mut focus, &theme, &input).with_style(&overlay),
        );
        // Rail background is now opaque soup (no SDF).
        assert!(
            list.vertices
                .iter()
                .any(|v| v.color == chrome.rail_colors[0])
        );
        assert!(
            list.chrome_instances()
                .any(|quad| quad.bg == [0.31, 0.32, 0.33, 1.0])
        );

        state.popup = Some(PopupKind::Dock);
        state.geom = Some(PopupGeometry {
            anchor: Rect::new(0.0, 0.0, 10.0, 10.0),
            viewport: Rect::new(0.0, 0.0, 300.0, 300.0),
            kind: PopupKind::Dock,
            overflow_start: 0,
        });
        let mut layers = LayerStack::new();
        state.draw_open_layer(
            &mut layers,
            None,
            &items,
            &StyleResolver::with_overlay(&theme, &overlay),
            &input,
        );
        assert!(
            layers.layers()[0]
                .list
                .chrome_instances()
                .any(|quad| quad.bg == [0.41, 0.42, 0.43, 1.0])
        );
        assert_eq!(layers.layers()[0].list.shadow_instance_count(), 2);
    }

    #[test]
    fn dock_popup_selects_edge_through_popup_layer() {
        let items = sample_items();
        let theme = Theme::default();
        let styles = StyleResolver::new(&theme);
        let mut state = ToolbarState::new(ToolbarEdge::Left);
        let mut capture = DragCapture::new();
        let mut focus = FocusState::default();
        // Right-click on the strip opens the dock popup (the design reserves
        // left-click on the grip for dragging only).
        let mut first_input = crate::InputState::default();
        first_input.mouse_x = 14.0;
        first_input.mouse_y = 10.0;
        first_input.mouse_right_clicked = true;
        let mut base = DrawList::new();
        Toolbar::new(&items).draw_with_id(
            7,
            Rect::new(0.0, 0.0, 28.0, 180.0),
            &mut state,
            &mut capture,
            99,
            &mut ctx(&mut base, &mut focus, &theme, &first_input),
        );
        let mut next_input = crate::InputState::default();
        state.begin_frame(&mut next_input);
        let mut layers = LayerStack::new();
        let popup = state
            .push_open_layer(&mut layers)
            .expect("dock popup layer");
        let popup_bounds = layers.layers()[popup].rect;
        next_input.mouse_x = popup_bounds.x + 20.0;
        next_input.mouse_y = popup_bounds.y + 33.0;
        next_input.mouse_clicked = true;
        let event = state.draw_open_layer(&mut layers, Some(popup), &items, &styles, &next_input);
        assert_eq!(event, Some(ToolbarEvent::DockChanged(ToolbarEdge::Top)));
        state.end_frame();
    }
}
