//! Asking the [`InteractionScene`] about elements an application draws itself.
//!
//! A hand-drawn element (a row's toggle strip, a chip, a bar) doesn't need to
//! test the pointer itself: [`Sensor::sense`] (or
//! [`DrawContext::sense`](crate::DrawContext::sense) where there's a context)
//! registers its rect and answers whether it is hovered, pressed or released
//! this frame. The scene works out layers, popups, transforms, clipping and
//! what else on the same layer covers it; the layer stack's verdict arrives as
//! [`InputState::mouse_consumed`], which the answer honours too.
//!
//! See `docs/design/pointer-sense.md`.

use crate::{
    Affine2, DrawList, HitShape, InputState, InteractionScene, PointerPolicy, Response, WidgetId,
    layout::Rect,
};

/// The scene, the input and the layer a run of elements is drawn on, held
/// together so code with only a [`DrawList`] in hand (a list's row callback)
/// can sense what it draws.
pub struct Sensor<'a> {
    scene: Option<&'a mut InteractionScene>,
    input: &'a InputState,
    layer: Option<usize>,
}

impl<'a> Sensor<'a> {
    /// A sensor for elements drawn into [`LayerStack`](crate::LayerStack)
    /// layer `layer` (`None` for the base list). `input` is that layer's
    /// input in screen space (from `input_for_layer` / `input_for_base`), not
    /// moved by any scroll offset: the sensor undoes the draw list's
    /// transform itself. Without a `scene` it tests the pointer directly,
    /// which can't see overlap within a layer.
    pub fn new(
        scene: Option<&'a mut InteractionScene>,
        input: &'a InputState,
        layer: Option<usize>,
    ) -> Self {
        Self {
            scene,
            input,
            layer,
        }
    }

    /// The input the sensor answers from, in screen space: for what an
    /// element does with the pointer beyond being over it (a drag's state,
    /// the wheel).
    pub fn input(&self) -> &'a InputState {
        self.input
    }

    /// Register `rect`, in `list`'s current local space, as `id` for the next
    /// frame and answer for this one. A disabled element still keeps the
    /// pointer from what lies under it, but its answer stays idle; so is the
    /// answer the first frame an id is seen, before the scene has its
    /// geometry. `clicked` is the press edge; an element that acts on release
    /// wants `released && hovered`.
    pub fn sense(
        &mut self,
        list: &DrawList,
        id: impl Into<WidgetId>,
        rect: Rect,
        enabled: bool,
    ) -> Response {
        let transform = list.current_transform();
        let world = [self.input.mouse_x, self.input.mouse_y];
        let local = transform
            .try_inverse()
            .map(|inverse| inverse.transform_point(world));
        resolve(
            self.scene.as_deref_mut(),
            self.input,
            Pointer { local, world },
            Placement {
                transform,
                clip: list.current_clip(),
                layer: scene_layer(self.layer),
            },
            Some(id.into()),
            rect,
            enabled,
        )
    }
}

/// The scene's layer for [`LayerStack`](crate::LayerStack) layer `layer`:
/// the base list is `0`, layer `i` is `i + 1`.
pub(crate) fn scene_layer(layer: Option<usize>) -> u32 {
    layer.map_or(0, |index| index as u32 + 1)
}

/// Where the pointer is, in the element's local space (`None` when the
/// transform can't be undone) and on screen.
pub(crate) struct Pointer {
    pub(crate) local: Option<[f32; 2]>,
    pub(crate) world: [f32; 2],
}

/// Where an element is drawn: its draw list's transform and clip, and its
/// scene layer.
pub(crate) struct Placement {
    pub(crate) transform: Affine2,
    pub(crate) clip: Option<Rect>,
    pub(crate) layer: u32,
}

/// The answer for `id`'s `rect`: from the scene when there is one and the
/// element has an id, otherwise from the pointer directly.
pub(crate) fn resolve(
    scene: Option<&mut InteractionScene>,
    input: &InputState,
    pointer: Pointer,
    placement: Placement,
    id: Option<WidgetId>,
    rect: Rect,
    enabled: bool,
) -> Response {
    let (Some(scene), Some(id)) = (scene, id) else {
        return immediate(input, pointer, placement.clip, id, rect, enabled);
    };
    debug_assert!(
        !scene.is_registered(id),
        "{id:?} was sensed twice in one frame; ids must be unique"
    );
    let response = scene.register(
        id,
        HitShape::Rect(rect),
        placement.transform,
        placement.clip,
        placement.layer,
        enabled,
        PointerPolicy::Target,
    );
    // The scene answers from last frame's region, which may still have been
    // enabled; and an id it hasn't seen yet has no answer.
    if !response.resolved || !enabled {
        return Response {
            resolved: response.resolved,
            ..Response::idle(id, rect)
        };
    }
    response.for_input(input)
}

/// The answer without a scene: the pointer tested against `rect` and the
/// clip, honouring `enabled` and a pointer a higher layer took.
fn immediate(
    input: &InputState,
    pointer: Pointer,
    clip: Option<Rect>,
    id: Option<WidgetId>,
    rect: Rect,
    enabled: bool,
) -> Response {
    let local = pointer.local.filter(|&[x, y]| {
        enabled
            && !input.mouse_consumed
            && rect.contains(x, y)
            && clip.is_none_or(|clip| clip.contains(pointer.world[0], pointer.world[1]))
    });
    let over = local.is_some();
    Response {
        id,
        rect,
        resolved: false,
        hovered: over,
        pressed: over && input.mouse_down,
        clicked: over && input.mouse_clicked,
        released: over && input.mouse_released,
        held: over && input.mouse_held,
        double_clicked: over && input.mouse_double_clicked,
        local_pos: local,
        scroll_delta: if over && !input.scroll_consumed {
            input.scroll_delta
        } else {
            0.0
        },
    }
}

#[cfg(test)]
#[path = "sense_tests.rs"]
mod tests;
