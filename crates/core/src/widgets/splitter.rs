//! Splitter — the design's draggable pane divider (Gallery II "splitter").
//!
//! A vertical or horizontal 6px groove between two panes, with a centered
//! 2×26px grip that brightens on hover and lights up (accent, with a glow) while
//! dragged. Drag is captured through the caller-owned
//! [`DragCapture`](super::DragCapture), so only one splitter owns the pointer
//! at a time.

use super::drag::{DragCapture, DragId};
use crate::chrome::{Background, Edge, GradientAxis};
use crate::layout::Rect;
use crate::shadow::CornerRadii;

use super::DrawContext;

/// Orientation of the divider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitAxis {
    /// Vertical bar; dragging moves the split left/right (col-resize).
    Vertical,
    /// Horizontal bar; dragging moves the split up/down (row-resize).
    Horizontal,
}

/// Outcome of drawing a [`Splitter`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitterOutput {
    /// The pointer delta along the split axis this frame (positive = right /
    /// down), while the splitter owns the drag. `0.0` otherwise.
    pub delta: f32,
    /// Whether this splitter owns the active drag.
    pub dragging: bool,
}

/// A pane divider. Draw between two panes; the caller sizes the bar via the
/// rect handed to [`Splitter::draw`] (the constructors' `thickness` argument
/// documents the intended floor).
#[derive(Clone, Copy)]
pub struct Splitter {
    axis: SplitAxis,
}

impl Splitter {
    /// A vertical divider (drag moves the split left/right).
    pub fn vertical(_thickness: f32) -> Self {
        Self {
            axis: SplitAxis::Vertical,
        }
    }

    /// A horizontal divider (drag moves the split up/down).
    pub fn horizontal(_thickness: f32) -> Self {
        Self {
            axis: SplitAxis::Horizontal,
        }
    }

    /// Draw the splitter at `rect`; the widget only reports deltas, the caller
    /// applies them to its pane sizes.
    pub fn draw(
        &self,
        id: DragId,
        capture: &mut DragCapture,
        rect: Rect,
        ctx: &mut DrawContext,
    ) -> SplitterOutput {
        ctx.push_debug_scope_rect("Splitter", rect);
        let input = ctx.input;

        let (px, py) = (input.mouse_x, input.mouse_y);
        let hovered = rect.contains(px, py) && !input.mouse_consumed;
        let grab_axis = match self.axis {
            SplitAxis::Vertical => crate::CursorIcon::ResizeHorizontal,
            SplitAxis::Horizontal => crate::CursorIcon::ResizeVertical,
        };

        if !input.mouse_down {
            capture.release(id);
        }
        let claimed_now = hovered && input.mouse_clicked && capture.is_free();
        if claimed_now {
            capture.try_begin(id);
        }
        let dragging = capture.is_active(id);

        if hovered || dragging {
            // Capture keeps the axis-resize cursor and bright grip alive even
            // after a fast pointer leaves the narrow strip.
            ctx.request_cursor(grab_axis);
        }

        // The groove itself is invariant. Interaction feedback belongs solely
        // to the grip, so panes moving under a captured drag never make the
        // splitter surface flash between materials.
        let chrome = ctx.styles().splitter();
        let grip_color = if dragging {
            chrome.grip_dragging
        } else if hovered {
            chrome.grip_hover
        } else {
            chrome.grip_idle
        };
        let grip = match self.axis {
            SplitAxis::Vertical => Rect::new(
                rect.x + (rect.width - 2.0) * 0.5,
                rect.y + (rect.height - 26.0) * 0.5,
                2.0,
                26.0,
            ),
            SplitAxis::Horizontal => Rect::new(
                rect.x + (rect.width - 26.0) * 0.5,
                rect.y + (rect.height - 2.0) * 0.5,
                26.0,
                2.0,
            ),
        };
        {
            let list = &mut *ctx.draw_list;
            let axis = match self.axis {
                SplitAxis::Vertical => GradientAxis::Horizontal,
                SplitAxis::Horizontal => GradientAxis::Vertical,
            };
            list.paint_background_opaque(
                rect,
                Background::LinearGradient {
                    start: chrome.track_colors[0],
                    end: chrome.track_colors[1],
                    axis,
                },
            );
            match self.axis {
                SplitAxis::Vertical => {
                    list.edge_line(
                        rect,
                        Edge::Left,
                        chrome.outer_edges.thickness,
                        chrome.outer_edges.color,
                    );
                    list.edge_line(
                        rect,
                        Edge::Right,
                        chrome.outer_edges.thickness,
                        chrome.outer_edges.color,
                    );
                    let highlight_rect = Rect::new(
                        rect.x + chrome.outer_edges.thickness,
                        rect.y,
                        (rect.width - chrome.outer_edges.thickness).max(0.0),
                        rect.height,
                    );
                    list.edge_line(
                        highlight_rect,
                        Edge::Left,
                        chrome.inner_highlight.thickness,
                        chrome.inner_highlight.color,
                    );
                }
                SplitAxis::Horizontal => {
                    list.edge_line(
                        rect,
                        Edge::Top,
                        chrome.outer_edges.thickness,
                        chrome.outer_edges.color,
                    );
                    list.edge_line(
                        rect,
                        Edge::Bottom,
                        chrome.outer_edges.thickness,
                        chrome.outer_edges.color,
                    );
                    let highlight_rect = Rect::new(
                        rect.x,
                        rect.y + chrome.outer_edges.thickness,
                        rect.width,
                        (rect.height - chrome.outer_edges.thickness).max(0.0),
                    );
                    list.edge_line(
                        highlight_rect,
                        Edge::Top,
                        chrome.inner_highlight.thickness,
                        chrome.inner_highlight.color,
                    );
                }
            }

            if dragging {
                list.box_shadow_outset(grip, CornerRadii::uniform(1.0), chrome.dragging_glow);
            }
            list.paint_quad_background(
                grip,
                Background::Solid(grip_color),
                CornerRadii::uniform(1.0),
            );
            if !dragging {
                match self.axis {
                    SplitAxis::Vertical => list.edge_line(
                        grip,
                        Edge::Right,
                        chrome.grip_counter_edge.thickness,
                        chrome.grip_counter_edge.color,
                    ),
                    SplitAxis::Horizontal => list.edge_line(
                        grip,
                        Edge::Bottom,
                        chrome.grip_counter_edge.thickness,
                        chrome.grip_counter_edge.color,
                    ),
                }
            }
        }

        // Pointer delta along the axis while we own the drag. `drag_delta` is
        // the DragTracker-computed per-frame movement.
        let delta = if dragging {
            match self.axis {
                SplitAxis::Vertical => input.drag_delta[0],
                SplitAxis::Horizontal => input.drag_delta[1],
            }
        } else {
            0.0
        };

        ctx.pop_debug_scope();
        SplitterOutput { delta, dragging }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawList, FocusState, InputState, StyleOverlay, Theme};

    fn ctx<'a>(
        list: &'a mut DrawList,
        focus: &'a mut FocusState,
        theme: &'a Theme,
        input: &'a InputState,
    ) -> DrawContext<'a> {
        DrawContext::new(list, focus, theme, input, 800.0, 600.0)
    }

    #[test]
    fn drag_captures_and_reports_deltas() {
        let theme = Theme::default();
        let mut capture = DragCapture::new();
        let rect = Rect::new(100.0, 0.0, 5.0, 200.0);

        // Press inside: claim.
        let input = InputState {
            mouse_x: 102.0,
            mouse_y: 50.0,
            mouse_down: true,
            mouse_clicked: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = Splitter::vertical(5.0).draw(
            1,
            &mut capture,
            rect,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(out.dragging, "press on the bar claims the drag");

        // Move 6px right while held.
        let input = InputState {
            mouse_x: 108.0,
            mouse_y: 50.0,
            mouse_down: true,
            is_dragging: true,
            drag_delta: [6.0, 0.0],
            ..Default::default()
        };
        let mut list = DrawList::new();
        let out = Splitter::vertical(5.0).draw(
            1,
            &mut capture,
            rect,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(out.dragging);
        assert_eq!(out.delta, 6.0);

        // Release: drag ends.
        let input = InputState {
            mouse_x: 108.0,
            mouse_y: 50.0,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let out = Splitter::vertical(5.0).draw(
            1,
            &mut capture,
            rect,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(!out.dragging && out.delta == 0.0);
    }

    #[test]
    fn typed_overlay_reaches_splitter_track_grip_and_edges() {
        let theme = Theme::default();
        let mut chrome = theme.chrome.splitter;
        chrome.track_colors = [[0.11, 0.12, 0.13, 1.0], [0.21, 0.22, 0.23, 1.0]];
        chrome.outer_edges.color = [0.31, 0.32, 0.33, 1.0];
        chrome.inner_highlight.color = [0.41, 0.42, 0.43, 1.0];
        chrome.grip_idle = [0.51, 0.52, 0.53, 1.0];
        chrome.grip_counter_edge.color = [0.61, 0.62, 0.63, 1.0];
        let mut overlay = StyleOverlay::new();
        overlay.set_splitter(chrome);
        let input = InputState::default();
        let mut capture = DragCapture::new();
        let mut list = DrawList::new();
        let mut focus = FocusState::new();

        Splitter::vertical(6.0).draw(
            1,
            &mut capture,
            Rect::new(10.0, 20.0, 6.0, 100.0),
            &mut ctx(&mut list, &mut focus, &theme, &input).with_style(&overlay),
        );

        // Track background is now opaque soup (no SDF); verify gradient
        // endpoints appear in the vertex buffer.
        assert!(
            list.vertices
                .iter()
                .any(|v| v.color == chrome.track_colors[0])
        );
        assert!(
            list.vertices
                .iter()
                .any(|v| v.color == chrome.track_colors[1])
        );
        // Edge lines and grip remain as chrome instances, shifted down by 1.
        assert_eq!(
            list.chrome_instance(0).unwrap().bg,
            chrome.outer_edges.color
        );
        assert_eq!(
            list.chrome_instance(2).unwrap().bg,
            chrome.inner_highlight.color
        );
        assert_eq!(list.chrome_instance(3).unwrap().bg, chrome.grip_idle);
        assert_eq!(
            list.chrome_instance(4).unwrap().bg,
            chrome.grip_counter_edge.color
        );
    }

    #[test]
    fn dragging_uses_typed_grip_and_one_analytic_shadow_before_the_grip() {
        let theme = Theme::default();
        let mut chrome = theme.chrome.splitter;
        chrome.grip_dragging = [0.17, 0.27, 0.37, 1.0];
        chrome.dragging_glow.color = [0.47, 0.57, 0.67, 0.77];
        chrome.dragging_glow.blur = 9.0;
        let mut overlay = StyleOverlay::new();
        overlay.set_splitter(chrome);
        let input = InputState {
            mouse_x: 80.0,
            mouse_y: 80.0,
            mouse_down: true,
            is_dragging: true,
            drag_delta: [8.0, 0.0],
            ..Default::default()
        };
        let mut capture = DragCapture::new();
        capture.try_begin(7);
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let mut cursor = crate::CursorState::new();
        let mut draw_ctx = ctx(&mut list, &mut focus, &theme, &input)
            .with_style(&overlay)
            .with_cursor(&mut cursor);
        let out = Splitter::vertical(6.0).draw(
            7,
            &mut capture,
            Rect::new(10.0, 20.0, 6.0, 100.0),
            &mut draw_ctx,
        );

        assert!(out.dragging);
        assert_eq!(out.delta, 8.0);
        assert_eq!(cursor.resolve(), crate::CursorIcon::ResizeHorizontal);
        assert_eq!(list.shadow_instance_count(), 1);
        let shadow = list.shadow_instance(0).unwrap();
        assert_eq!(shadow.element_rect, [12.0, 57.0, 2.0, 26.0]);
        assert_eq!(shadow.color, chrome.dragging_glow.color);
        assert_eq!(shadow.params[0], chrome.dragging_glow.blur * 0.5);
        assert_eq!(list.chrome_instance(3).unwrap().bg, chrome.grip_dragging);
        // Track background is now soup, so paint_cmds has a Soup + Analytic.
        assert_eq!(list.paint_cmds.len(), 2);
        assert!(matches!(
            &list.paint_cmds[1],
            crate::widgets::PaintCmd::Analytic { instances }
                if instances == &(0..list.analytic_instances.len() as u32)
        ));
        let shadow_position = list
            .analytic_instances
            .iter()
            .position(|instance| instance.as_shadow() == Some(shadow))
            .unwrap();
        let grip_position = list
            .analytic_instances
            .iter()
            .position(|instance| {
                instance
                    .as_chrome()
                    .is_some_and(|quad| quad.bg == chrome.grip_dragging)
            })
            .unwrap();
        assert!(shadow_position < grip_position);
    }

    #[test]
    fn a_second_splitter_cannot_steal_an_active_drag() {
        let theme = Theme::default();
        let mut capture = DragCapture::new();
        let a = Rect::new(0.0, 0.0, 5.0, 200.0);
        let b = Rect::new(50.0, 0.0, 5.0, 200.0);

        let press = InputState {
            mouse_x: 2.0,
            mouse_y: 30.0,
            mouse_down: true,
            mouse_clicked: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let mut focus = FocusState::new();
        let out = Splitter::vertical(5.0).draw(
            1,
            &mut capture,
            a,
            &mut ctx(&mut list, &mut focus, &theme, &press),
        );
        assert!(out.dragging);

        // Splitter B sees the same held pointer but must not report dragging.
        let input = InputState {
            mouse_x: 52.0,
            mouse_y: 30.0,
            mouse_down: true,
            mouse_clicked: true,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let out = Splitter::vertical(5.0).draw(
            2,
            &mut capture,
            b,
            &mut ctx(&mut list, &mut focus, &theme, &input),
        );
        assert!(!out.dragging, "the capture is owned by splitter A");
    }
}
