//! A node's own drawing: its shadow, its box, its text or its painter, at
//! its origin. The same function draws it in a kept list and in the frame
//! drawn from nothing.

use wgpu_gameui::DrawList;
use wgpu_gameui::layout::Rect;

use crate::layout::{text_block, text_place};
use crate::node::Paint;
use crate::paint::quad_style;
use crate::signal::Graph;
use crate::ui::NodeData;

pub(crate) fn paint_node(
    d: &NodeData,
    graph: &mut Graph,
    now: f64,
    list: &mut DrawList,
    deadline: &mut f64,
) {
    let [w, h] = d.size;
    let rect = Rect::new(0.0, 0.0, w, h);
    let look = d.look.value;
    if let Some(shadow) = look.shadow
        && !shadow.inset
    {
        list.box_shadow_outset(rect, wgpu_gameui::CornerRadii::uniform(look.radius), shadow);
    }
    if let Some(style) = quad_style(d) {
        list.paint_quad(rect, style);
    }
    if let Some(t) = &d.text {
        let (x, y, width) = text_place(d);
        list.text(text_block(&t.text.value, t.color.value, x, y, width));
    }
    if let Some(c) = &d.custom {
        let mut repaint = f64::INFINITY;
        let mut paint = Paint {
            list,
            size: d.size,
            now,
            repaint: &mut repaint,
        };
        c.paint.draw(graph, &mut paint);
        *deadline = deadline.min(repaint);
    }
}
