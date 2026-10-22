use super::*;
use crate::{DrawContext, FocusState, LayerStack, Theme};

const BACK: WidgetId = WidgetId(1);
const FRONT: WidgetId = WidgetId(2);

fn area() -> Rect {
    Rect::new(10.0, 10.0, 100.0, 40.0)
}

/// The pointer at (`x`, `y`), the button up.
fn at(x: f32, y: f32) -> InputState {
    InputState {
        mouse_x: x,
        mouse_y: y,
        ..InputState::default()
    }
}

/// The press edge at (`x`, `y`).
fn press(x: f32, y: f32) -> InputState {
    InputState {
        mouse_down: true,
        mouse_clicked: true,
        ..at(x, y)
    }
}

/// The button held at (`x`, `y`).
fn hold(x: f32, y: f32) -> InputState {
    InputState {
        mouse_down: true,
        ..at(x, y)
    }
}

/// The release edge at (`x`, `y`).
fn release(x: f32, y: f32) -> InputState {
    InputState {
        mouse_released: true,
        ..at(x, y)
    }
}

/// A host's per-frame loop: the scene resolved against `input`, a draw list,
/// and the layers pushed for this frame.
#[derive(Default)]
struct Rig {
    scene: InteractionScene,
    list: DrawList,
    layers: LayerStack,
}

impl Rig {
    /// One frame: push `popup` (if any) as a popup layer, then call `draw`
    /// with a sensor for the base list. Ending the frame with the layers lets
    /// the scene block what lies under the popup.
    fn frame<R>(
        &mut self,
        input: &InputState,
        popup: Option<Rect>,
        draw: impl FnOnce(&mut Sensor<'_>, &mut DrawList) -> R,
    ) -> R {
        self.scene.begin_frame(input);
        self.list.clear();
        self.layers.clear();
        if let Some(rect) = popup {
            self.layers.push_popup(rect);
            self.layers.pop_layer();
        }
        let base = self.layers.input_for_base(input);
        let out = draw(
            &mut Sensor::new(Some(&mut self.scene), &base, None),
            &mut self.list,
        );
        self.scene.end_frame(self.layers.layers());
        out
    }

    /// Frames sensing `rect` as `id` on the base list, one per input.
    fn sense_each(&mut self, inputs: &[InputState], id: WidgetId, rect: Rect) -> Vec<Response> {
        inputs
            .iter()
            .map(|input| {
                self.frame(input, None, |sensor, list| {
                    sensor.sense(list, id, rect, true)
                })
            })
            .collect()
    }
}

#[test]
fn an_element_seen_for_the_first_time_is_idle_and_answers_from_then_on() {
    let mut rig = Rig::default();
    let [first, second] = rig.sense_each(&[at(20.0, 20.0), at(20.0, 20.0)], BACK, area())[..]
    else {
        unreachable!()
    };
    assert!(!first.resolved && !first.hovered, "{first:?}");
    assert!(second.resolved && second.hovered, "{second:?}");
    assert_eq!(second.local_pos, Some([20.0, 20.0]));
}

#[test]
fn without_a_scene_the_pointer_is_tested_directly() {
    let list = DrawList::new();
    let inside = at(20.0, 20.0);
    let outside = at(200.0, 20.0);
    let over = Sensor::new(None, &inside, None).sense(&list, BACK, area(), true);
    let away = Sensor::new(None, &outside, None).sense(&list, BACK, area(), true);
    let disabled = Sensor::new(None, &inside, None).sense(&list, BACK, area(), false);
    let taken = inside.consumed();
    let under = Sensor::new(None, &taken, None).sense(&list, BACK, area(), true);
    assert!(over.hovered && !over.resolved);
    assert!(!away.hovered && !disabled.hovered && !under.hovered);
}

#[test]
fn an_element_under_a_popup_is_not_hovered_from_the_frame_the_popup_opens() {
    let mut rig = Rig::default();
    let pointer = at(20.0, 20.0);
    let sense =
        |sensor: &mut Sensor<'_>, list: &mut DrawList| sensor.sense(list, BACK, area(), true);
    rig.frame(&pointer, None, sense);
    assert!(rig.frame(&pointer, None, sense).hovered);
    // The scene has no blocker for the popup yet; the taken pointer covers it.
    let popup = Rect::new(0.0, 0.0, 50.0, 50.0);
    assert!(!rig.frame(&pointer, Some(popup), sense).hovered);
    assert!(!rig.frame(&pointer, Some(popup), sense).hovered);
    // Moved off the pointer, the element is hovered again once the scene has
    // the popup where it now is (it answers from what was last shown); grown
    // back over it, it isn't, at once.
    let moved = Rect::new(60.0, 0.0, 50.0, 50.0);
    assert!(!rig.frame(&pointer, Some(moved), sense).hovered);
    assert!(rig.frame(&pointer, Some(moved), sense).hovered);
    assert!(
        !rig.frame(&pointer, Some(Rect::new(0.0, 0.0, 120.0, 60.0)), sense)
            .hovered
    );
}

#[test]
fn a_press_that_began_on_a_closed_popup_never_presses_what_was_under_it() {
    let mut rig = Rig::default();
    let popup = Rect::new(0.0, 0.0, 50.0, 50.0);
    let sense =
        |sensor: &mut Sensor<'_>, list: &mut DrawList| sensor.sense(list, BACK, area(), true);
    rig.frame(&at(20.0, 20.0), Some(popup), sense);
    rig.frame(&press(20.0, 20.0), Some(popup), sense);
    // The press closed the popup; the button is still held over the element.
    let held = rig.frame(&hold(20.0, 20.0), None, sense);
    let released = rig.frame(&release(20.0, 20.0), None, sense);
    assert!(!held.pressed && !held.hovered, "{held:?}");
    assert!(!released.released, "{released:?}");
    assert!(rig.frame(&at(20.0, 20.0), None, sense).hovered);
}

#[test]
fn of_two_overlapping_elements_the_later_one_takes_the_pointer() {
    let mut rig = Rig::default();
    let both = |sensor: &mut Sensor<'_>, list: &mut DrawList| {
        let back = sensor.sense(list, BACK, area(), true);
        let front = sensor.sense(list, FRONT, Rect::new(10.0, 10.0, 30.0, 30.0), true);
        (back, front)
    };
    rig.frame(&at(20.0, 20.0), None, both);
    let (back, front) = rig.frame(&press(20.0, 20.0), None, both);
    assert!(!back.hovered && !back.clicked);
    assert!(front.hovered && front.clicked);
    // Off the front one, the back one has the pointer.
    let (back, front) = rig.frame(&at(80.0, 20.0), None, both);
    assert!(back.hovered && !front.hovered);
}

#[test]
fn a_disabled_element_on_top_stays_idle_and_still_covers_what_is_under_it() {
    let mut rig = Rig::default();
    let both = |sensor: &mut Sensor<'_>, list: &mut DrawList| {
        let back = sensor.sense(list, BACK, area(), true);
        let front = sensor.sense(list, FRONT, area(), false);
        (back, front)
    };
    rig.frame(&at(20.0, 20.0), None, both);
    let (back, front) = rig.frame(&press(20.0, 20.0), None, both);
    assert!(!back.hovered && !back.clicked, "{back:?}");
    assert!(!front.hovered && !front.clicked, "{front:?}");
}

#[test]
fn an_element_that_turns_disabled_mid_gesture_gets_no_click_or_release() {
    let mut rig = Rig::default();
    let sense = |enabled: bool| {
        move |sensor: &mut Sensor<'_>, list: &mut DrawList| {
            sensor.sense(list, BACK, area(), enabled)
        }
    };
    rig.frame(&at(20.0, 20.0), None, sense(true));
    let pressed = rig.frame(&press(20.0, 20.0), None, sense(false));
    let released = rig.frame(&release(20.0, 20.0), None, sense(false));
    assert!(!pressed.clicked && !pressed.pressed, "{pressed:?}");
    assert!(!released.released, "{released:?}");
}

#[test]
fn an_element_that_appears_over_another_mid_gesture_never_shares_it() {
    let mut rig = Rig::default();
    let back_only = |sensor: &mut Sensor<'_>, list: &mut DrawList| {
        (sensor.sense(list, BACK, area(), true), Response::default())
    };
    let both = |sensor: &mut Sensor<'_>, list: &mut DrawList| {
        let back = sensor.sense(list, BACK, area(), true);
        (back, sensor.sense(list, FRONT, area(), true))
    };
    rig.frame(&at(20.0, 20.0), None, back_only);
    let (back, front) = rig.frame(&press(20.0, 20.0), None, both);
    assert!(
        back.clicked && !front.clicked,
        "the new one is idle: {front:?}"
    );
    for input in [hold(20.0, 20.0), hold(30.0, 25.0)] {
        let (back, front) = rig.frame(&input, None, both);
        assert!(back.pressed && !front.pressed, "{back:?} {front:?}");
    }
    let (back, front) = rig.frame(&release(30.0, 25.0), None, both);
    assert!(back.released && !front.released, "{back:?} {front:?}");
}

#[test]
fn an_element_under_a_scroll_is_hovered_where_it_shows_and_not_where_it_is_clipped() {
    let mut rig = Rig::default();
    // Rows scrolled up by 100 inside a pane at y 0..60, nested in a pane
    // moved right by 5.
    let row = Rect::new(10.0, 110.0, 100.0, 40.0);
    let sense = |sensor: &mut Sensor<'_>, list: &mut DrawList| {
        list.push_transform();
        list.translate(5.0, 0.0);
        list.push_clip(Rect::new(0.0, 0.0, 200.0, 60.0));
        list.push_transform();
        list.translate(0.0, -100.0);
        let response = sensor.sense(list, BACK, row, true);
        list.pop_transform();
        list.pop_clip();
        list.pop_transform();
        response
    };
    // On screen the row spans x 15..115, y 10..50, clipped at 60.
    rig.frame(&at(20.0, 20.0), None, sense);
    let shown = rig.frame(&at(20.0, 20.0), None, sense);
    assert!(shown.hovered, "{shown:?}");
    assert_eq!(shown.local_pos, Some([15.0, 120.0]));
    assert!(
        !rig.frame(&at(12.0, 20.0), None, sense).hovered,
        "left of it"
    );
    // Where the row would be if it weren't scrolled, it isn't.
    assert!(!rig.frame(&at(20.0, 120.0), None, sense).hovered);
    // A row reaching past the pane's clip isn't hovered below it.
    let tall = Rect::new(10.0, 110.0, 100.0, 100.0);
    let clipped = |sensor: &mut Sensor<'_>, list: &mut DrawList| {
        list.push_clip(Rect::new(0.0, 0.0, 200.0, 60.0));
        list.push_transform();
        list.translate(0.0, -100.0);
        let response = sensor.sense(list, FRONT, tall, true);
        list.pop_transform();
        list.pop_clip();
        response
    };
    rig.frame(&at(20.0, 70.0), None, clipped);
    assert!(!rig.frame(&at(20.0, 70.0), None, clipped).hovered);
    assert!(rig.frame(&at(20.0, 50.0), None, clipped).hovered);
}

#[test]
fn a_child_sensed_after_its_parent_takes_the_pointer_inside_it() {
    let mut rig = Rig::default();
    let child = Rect::new(80.0, 15.0, 20.0, 20.0);
    let nested = |sensor: &mut Sensor<'_>, list: &mut DrawList| {
        let parent = sensor.sense(list, BACK, area(), true);
        (parent, sensor.sense(list, FRONT, child, true))
    };
    rig.frame(&at(85.0, 20.0), None, nested);
    let (parent, inner) = rig.frame(&at(85.0, 20.0), None, nested);
    assert!(inner.hovered && !parent.hovered);
    let (parent, inner) = rig.frame(&at(20.0, 20.0), None, nested);
    assert!(parent.hovered && !inner.hovered);
}

#[test]
#[should_panic(expected = "sensed twice in one frame")]
#[cfg(debug_assertions)]
fn sensing_one_id_twice_in_a_frame_is_a_bug() {
    let mut rig = Rig::default();
    rig.frame(&at(20.0, 20.0), None, |sensor, list| {
        sensor.sense(list, BACK, area(), true);
        sensor.sense(list, BACK, area(), true);
    });
}

#[test]
fn hashed_ids_are_the_same_for_the_same_key_and_differ_between_keys() {
    let toggle = WidgetId::hashed(&("transcript", 0_u8, "event-1", 0_u8));
    assert_eq!(
        toggle,
        WidgetId::hashed(&("transcript", 0_u8, "event-1", 0_u8))
    );
    assert_ne!(
        toggle,
        WidgetId::hashed(&("transcript", 0_u8, "event-1", 1_u8))
    );
    assert_ne!(
        toggle,
        WidgetId::hashed(&("transcript", 1_u8, "event-1", 0_u8))
    );
}

/// `DrawContext::sense` takes the context's input to be in its local space,
/// as every widget drawn through a context does.
#[test]
fn a_draw_context_senses_in_its_own_local_space_on_its_layer() {
    let theme = Theme::default();
    let mut focus = FocusState::new();
    let mut scene = InteractionScene::new();
    let mut list = DrawList::new();
    let mut layers = LayerStack::new();
    let popup = Rect::new(0.0, 0.0, 300.0, 300.0);
    let index = layers.push_popup(popup);
    layers.pop_layer();
    let mut frame = |screen: &InputState, scene: &mut InteractionScene| {
        scene.begin_frame(screen);
        list.clear();
        list.push_transform();
        list.translate(0.0, 50.0);
        // The pointer moved into the list's space, as UiContext does.
        let local = InputState {
            mouse_y: screen.mouse_y - 50.0,
            ..layers.input_for_layer(index, screen)
        };
        let on_layer = {
            let mut cx = DrawContext::new(&mut list, &mut focus, &theme, &local, 300.0, 300.0)
                .with_interactions(&mut *scene)
                .with_layer(index);
            cx.sense(BACK, area())
        };
        list.pop_transform();
        // A base element in the same place, sensed later: the layer is
        // still above it.
        let base = Sensor::new(Some(&mut *scene), screen, None).sense(
            &list,
            FRONT,
            Rect::new(10.0, 60.0, 100.0, 40.0),
            true,
        );
        scene.end_frame(layers.layers());
        (on_layer, base)
    };
    // On screen the element is at y 60..100.
    frame(&at(20.0, 70.0), &mut scene);
    let (on_layer, base) = frame(&at(20.0, 70.0), &mut scene);
    assert!(on_layer.hovered, "{on_layer:?}");
    assert!(!base.hovered, "{base:?}");
    assert!(!frame(&at(20.0, 20.0), &mut scene).0.hovered);
}

#[test]
fn a_disabled_draw_context_gives_idle_answers_that_still_cover() {
    let theme = Theme::default();
    let mut focus = FocusState::new();
    let mut scene = InteractionScene::new();
    let mut list = DrawList::new();
    let mut frame = |input: &InputState, scene: &mut InteractionScene| {
        scene.begin_frame(input);
        list.clear();
        let mut cx = DrawContext::new(&mut list, &mut focus, &theme, input, 300.0, 300.0)
            .with_interactions(scene);
        let back = cx.sense(BACK, area());
        let mut cx = cx.with_enabled(false);
        let front = cx.sense(FRONT, area());
        drop(cx);
        scene.end_frame(&[]);
        (back, front)
    };
    frame(&at(20.0, 20.0), &mut scene);
    let (back, front) = frame(&press(20.0, 20.0), &mut scene);
    assert!(!back.hovered && !back.clicked, "{back:?}");
    assert!(!front.hovered && !front.clicked, "{front:?}");
}
