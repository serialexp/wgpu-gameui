# Pointer sense: ask gameui whether something is hovered

**Status:** Accepted
**Last updated:** 2026-10-22

## Implementation status

Tracking the gap between this design and what's on the main branch.

### Done

- (nothing yet)

### Outstanding

- [ ] `Sensor` and `DrawContext::sense` in gameui, with tests (phase 1; written and passing in `crates/core/src/sense.rs` / `sense_tests.rs`, not committed)
- [ ] `Pressable` and `Checkbox` resolve through `sense` instead of their own copy of the pattern (phase 1; done, not committed)
- [ ] Every pushed layer blocks the scene below it (and a modal keeps Tab inside it) without the host having to call `block_under_layer` (phase 2; done in gameui and agent-ui, not committed)
- [ ] The menu bar's labels register on its viewport layer while a menu is open, so it needs no opt-out and no bar-shaped blocker (phase 2; done, not committed)
- [ ] agent-ui transcript (`transcript_pane.rs`) senses its toggle strips, timestamps and buttons, with a test that a popup over a row leaves it unhovered (phase 3; done, not committed: `Sensor` passed into `TranscriptPane::draw`, every part acts on `released && hovered`, tests for a popup, a later widget on top, a press that closes a popup, and a press and release on different rows)
- [ ] agent-ui: the remaining hand-written hover checks move to `sense` (phase 3, list under "Where agent-ui tests the pointer by hand"; done, not committed. See the 2026-10-22 note under that list for what moved and what stays)
- [ ] gameui's own immediate-mode widgets move to `sense`, one at a time (phase 4, once phases 1–3 work)

## Why this exists

An application that draws something hoverable by hand (a transcript row, a chip, a bar) today
decides hover itself, with `rect.contains(input.mouse_x, input.mouse_y)`. To get it right it
also has to remember that a higher layer may have taken the pointer (`input.mouse_consumed`),
undo any scroll transform, and know that nothing drawn later on the same layer covers it. Each
of those is easy to forget. agent-ui's transcript forgot the first one: with a dropdown list,
menu or popover over the transcript, the row under it still shows its hover wash, its
timestamp hint and a hand cursor (`agent-ui/crates/desktop/src/transcript_pane.rs:567`).

gameui already has the machinery that answers "is this element the one under the pointer":
the hit-region scene, `InteractionScene` (`crates/core/src/interaction.rs`). But only five
call sites use it, and the call an app would use is too bare to be the obvious choice. This
doc is the contract for one app-facing call, `sense(id, rect) -> Response`, that answers hover,
press and click from the scene, and for moving the hand-written checks onto it.

## Goals

- One call an app makes for anything it draws by hand: give an id and a rect, get back whether
  it is hovered, pressed or clicked this frame. gameui works out layers, popups, the open
  dropdown's area, transforms, clipping and overlap within a layer.
- It works where apps draw: with a `DrawContext`, and inside callbacks that only hand over a
  `DrawList` (the rows of `List::draw_measured`).
- Hover, cursor and click for one element come from the same answer, so they can't disagree.
- A layer the host pushes blocks the scene under it without a second, separate call.
- agent-ui's transcript stops showing hover under popups, menus and dialogs.

## Non-goals

- Changing how the scene picks a winner. It stays as it is: the previous frame's geometry,
  topmost by layer and then by order drawn.
- Keyboard focus and Tab order (`FocusState`). Untouched.
- Drag-and-drop, text selection and wheel routing. They follow the pointer even when it is
  "taken" (a drag that runs past a popup's edge) and keep reading it directly.
- Moving gameui's own widgets before `sense` works in agent-ui; that is phase 4.

## How it works today

### The scene

`InteractionScene` keeps two lists of hit regions: the ones collected in the frame being drawn
and the ones from the previous, finished frame. `begin_frame` resolves the pointer against the
previous frame's regions. That is the geometry the user actually saw: halfway through drawing
a frame, gameui can't yet know whether something drawn later will cover an element. Regions are
ordered by layer, then by the order they were registered; the topmost one under the pointer
wins and is the only one hovered. `register(id, shape, transform, clip, layer, enabled, policy)`
records a region for the next frame and returns this frame's `Response` for that id: `hovered`,
`pressed`, `clicked`, `released`, `held`, `double_clicked`, `local_pos`, `scroll_delta`, plus
`resolved` (false on the first frame an id appears, when there is no previous geometry yet).

`DrawContext::interact(id, rect, enabled)` (`crates/core/src/widgets/mod.rs:484`) is the
convenient form: it takes the transform and clip from the draw list and the layer from the
context's `active_layer`, and returns an idle response when the context has no scene attached.

### Layers and the scene are two systems

`LayerStack` (`crates/core/src/layer.rs`) decides which layer gets the pointer and hands lower
layers an `InputState` with `mouse_consumed` set. The scene only knows about regions. To keep a
layer's area from reaching the scene's regions under it, someone has to register a blocker:
`UiState::block_under_layer(layers, index)` (`crates/core/src/ui_context.rs:510`). `UiContext`'s
layer helpers and `push_dropdown_layer` do that; a host that draws into a layer itself must
call it, and agent-ui does (`shell_ui.rs:1000`, `:1045`, and each dialog's `draw`). The menu
bar is the exception: its viewport layer shapes its own blocking so the bar stays live
(`docs/design/menubar.md`, "Layers and hit ordering").

Because blockers depend on the host, widgets that use the scene also check the consumed flag:
`Response::for_input(input)` (`interaction.rs:167`) blanks a response when the input is
consumed.

### Who uses it

- gameui: `Pressable` (`widgets/pressable.rs:193`), `Checkbox` (`widgets/checkbox.rs:161`), the
  menu bar (`widgets/menubar/mod.rs:271`, `menubar/paint.rs:216`, `:237`).
  `Pressable` and `Checkbox` repeat the same steps: `interact`, keep the response only if
  `resolved`, pass it through `for_input`, and otherwise fall back to an immediate rect test
  with the consumed check.
- agent-ui: only the popover backdrops (`drop_up.rs:107`, `task_popover.rs:360`,
  `usage_popovers.rs:286`).
- Most of gameui's other widgets (lists, sliders, text inputs, toolbars and so on) hit-test
  immediately against `mouse_x`/`mouse_y` with the consumed check. `InputState::is_hovered` and
  `UiContext::hit_zone_at` do the same and also don't go through the scene.

## Design

### `Sensor`: the scene, the input and the layer, held together

```rust
/// Asks the scene about elements an app draws itself.
pub struct Sensor<'a> {
    scene: Option<&'a mut InteractionScene>,
    input: &'a InputState, // screen space: never moved by a scroll offset
    layer: Option<usize>,  // the LayerStack index drawn into; None is the base
}

impl<'a> Sensor<'a> {
    pub fn new(scene: Option<&'a mut InteractionScene>, input: &'a InputState, layer: Option<usize>) -> Self;

    /// Register `rect` (in `list`'s current local space) as `id` for the next
    /// frame and answer for this one. A disabled region still takes the
    /// pointer from what lies under it, but its response stays idle.
    pub fn sense(&mut self, list: &DrawList, id: impl Into<WidgetId>, rect: Rect, enabled: bool) -> Response;
}
```

`sense` does what `Pressable` and `Checkbox` do by hand today, once:

1. Register the region with the list's current transform and clip, at the layer's scene layer
   (`index + 1`, base `0`), with the caller's `enabled`. Registering a disabled region matters:
   it keeps the pointer from whatever is under it (`interaction.rs:240–261`).
2. If the response is `resolved`, pass it through `for_input(input)`, blank it as well if
   `enabled` is false now (the scene answers from last frame's region, which may still have been
   enabled, `interaction.rs:314–318`), and return it. `for_input`
   is part of the contract, not a fallback (see "Layers block the scene on their own" for why
   blockers alone aren't enough).
3. If it isn't resolved (the first frame this id appears) and there is a scene, answer idle. An
   immediate answer here would be unsafe: the scene hands out press ownership (capture) only to
   the previous frame's winner (`interaction.rs:248–251`), so a newly shown element over an
   existing one could take a click the existing one also takes, and the rest of the gesture
   would go to the other one. The cost is that something that appeared this very frame can't be
   clicked until the next one (open question 1).
4. With no scene at all, answer with an immediate test in the same space: undo the list's
   transform on the pointer, test the rect and the clip, honour `enabled` and `mouse_consumed`.
   Today's fallback in `Pressable` and `Checkbox` tests the raw pointer against the rect without
   undoing transforms or clips (`pressable.rs:196–198`, `checkbox.rs:165–167`), so this is
   slightly better than what they do now. It still can't see overlap within a layer.

The input a `Sensor` gets is the screen-space input of its layer (from `input_for_layer` /
`input_for_base`). Several of today's hand-written checks first move the pointer by the scroll
offset (`agent-ui right_dock.rs:407–414`, `tree_rows.rs:69–85`); code moving onto `sense` must
drop that, or the offset is applied twice. Code that builds a child `DrawContext` (as
`tree_rows.rs:80–87` does) must pass the scene and `active_layer` along, or it stays on the
no-scene path.

`DrawContext::sense(id, rect) -> Response` is a thin wrapper over a `Sensor` made from the
context's own scene, input, draw list and `active_layer`, with `enabled` from the context
(`with_enabled`); `DrawContext::sense_enabled(id, rect, enabled)` also takes the element's own and uses both
(the element is enabled only if it and the context are).
Code with only a `DrawList` in hand, like the row callback of `List::draw_measured`, makes a
`Sensor` from those parts before the call and moves it into the callback (the callback is
`FnMut`, and `draw_measured` doesn't borrow the scene: `widgets/list.rs:871–884`). It must be
made from the separate parts, not by borrowing a whole `DrawContext` that is still drawing.
agent-ui's `TranscriptPane::draw` has no scene today (`transcript_pane.rs:519–528`) and gains one.

> **2026-10-22 update:** `DrawContext::sense` follows gameui's convention that a context's input
> is already in its draw list's local space (`UiContext` moves it there), so it uses the
> pointer as it gets it and works out the screen position from the list's transform. Only a
> bare `Sensor` takes screen-space input and undoes the transform itself.

### Press, release and position

`Response.clicked` is the press edge (`interaction.rs:133–135`). Elements that act on release,
like the transcript's toggles and images, act on `released && hovered`: the scene gives
`released` to the element the press began on wherever the pointer is let go
(`interaction.rs:275–276`), so `hovered` is what makes it "released over the same element".
They keep their own rules for drags and text selection
(`transcript_pane.rs:567–571`, `:743–760`). `sense` answers where the pointer is; it doesn't
decide what counts as an activation.

`local_pos` is in the draw list's local space (not relative to `Response.rect`), and is `None`
once the pointer leaves the shape or its clip, even for the element that owns the press
(`interaction.rs:104–115`, `:279–282`). Gestures that carry on outside the element, like a
terminal selection, use `sense` to start and keep reading the raw pointer until release.

`DrawContext::interact` stays for code that must not honour the consumed flag; the menu bar is
the one known case (its bar labels sit under its own viewport layer and must stay live).

### Cursor

`sense` doesn't set the cursor itself; what shape a hovered element wants differs (hand, text
beam, resize). The caller asks with the response it got:
`if response.hovered { cursor.request(CursorIcon::Pointer) }`. That keeps hover, cursor and
click on the same answer, which is the point.

### Ids

The scene keys everything by `WidgetId(u64)`, and an id must stay the same from frame to frame.
A transcript row's position in the list changes, so it can't be the id. gameui adds
`WidgetId::hashed(&impl Hash)`, a hash that is the same from frame to frame within one run, so
an app can write `WidgetId::hashed(&("transcript", scope, event.id, Part::Toggle))`. The key
includes whatever tells two copies of the same view apart (the transcript's `scope`,
`transcript_pane.rs:584`), so a subagent's transcript in a popover doesn't collide with the
main one. The scene already reports duplicates (`duplicate_ids`); `sense` asserts on them in
debug builds.

`Pressable` and `Checkbox` only use the scene when they have a `focus_id` (`pressable.rs:190–193`,
`checkbox.rs:158–161`). Without one they have no stable id, so they keep the immediate test;
they must not fall back to a shared default id, which would be one duplicate per widget.

### Layers block the scene on their own (phase 2)

Today a layer the host forgets to `block_under_layer` lets scene widgets under it win. That's the
same class of bug as the transcript's, one level down. Phase 2 removes the separate step:
`UiState::end_frame` takes the frame's `&LayerStack` and does for every layer in it what
`block_under_layer` does now. Registering blockers at the end of the frame gives the same hit
order as registering them first, because a blocker sits under everything on its own layer
whatever its registration time (`interaction.rs:335–352`).

`block_under_layer` also records the topmost modal (`top_modal`), which keeps Tab inside it
(`ui_context.rs:495–519`). `end_frame` hands that to `focus.end_frame` before the scene's
`end_frame` (`ui_context.rs:414–416`), so the layer pass runs first, at the very start of
`end_frame`.

The menu bar is the one layer user that blocks differently today. While a menu is open it pushes
a popup over the whole viewport (`menubar/state.rs:639`), so a click anywhere else closes the menu
and reaches nothing underneath. Its labels (File, View, …) are drawn in the base layer, under that
popup (`menubar/mod.rs:271`), and must stay live so moving to another label switches menus. So it
registers the viewport minus its own strip as up to four scene regions (`blocker_regions`,
`menubar/paint.rs:28–55`), and its labels use `interact` without the consumed check, because the
viewport popup marks the pointer as taken over them too.

An automatic whole-viewport blocker would kill those labels. Instead of an opt-out, the labels
move up: while a menu is open, the bar registers its label regions on the viewport layer (a
`Sensor` for that layer, with that layer's input), and while none is open, on the base layer as
now. A layer's blocker sits under everything registered on that same layer, so the labels sit
above it and everything below is blocked: the behaviour the bar wants, with no special case.
`blocker_regions` and the bar-strip exclusion go away, and the labels use plain `sense`.
Whether the viewport's own scene regions stay (as targets whose clicks close the menu) is up to
the menu bar's implementation; they no longer need to leave the strip out.

`block_under_layer` is then removed, along with its calls in agent-ui.

> **2026-10-22 update:** shipped as `InteractionScene::end_frame(&[Layer])` and
> `UiState::end_frame(&[Layer])` (the host passes `layers.layers()`), not a `&LayerStack`, so
> code with a plain scene gets the blocking too. A modal blocks the whole screen, a popup its
> rect, and a tooltip layer nothing. `UiState::end_frame` finds the top modal in the same list
> and hands it to focus. The menu bar's labels sit on the viewport layer while a menu is open
> (`MenuBarState::blocker_layer`), but keep `interact` rather than `sense`: the bar draws them
> with the base layer's input, which the viewport popup has marked as taken. agent-ui's
> popovers (`usage_popovers.rs`, `task_popover.rs`, `drop_up.rs`) push screen-sized modal
> layers, so they now keep Tab inside them as well; whether that stays is Bart's call
> (`TODO.md`).

This does not make `for_input` unnecessary. The scene answers in `begin_frame` from the
previous frame's regions, so a modal opened or a popup grown this frame has no blocker there
yet. And `LayerStack` keeps a press with the layer it began on until release, even after that
layer closes (`layer.rs:309–334`), which the scene's geometry can't express. The consumed flag
covers both.

## Where agent-ui tests the pointer by hand

From `rg mouse_x crates/desktop/src` on 2026-10-22, tests and `snapshot.rs` left out. "Move"
means the check becomes a `sense` call; "keep" means it legitimately reads the raw pointer.

| Place | What it decides | Plan |
|---|---|---|
| `transcript_pane.rs:567`, `:613–618` | row toggle strip hover and click, timestamp hint, link/image/copy/answer/task/reasoning targets | move (first; the bug) |
| `transcript_pane.rs:873–877` | which table the wheel scrolls sideways | keep (wheel routing) |
| `sidebar.rs:507` | the sidebar key's hover | move |
| `sidebar.rs:430` | pointer inside the dock (for its scroll/drag) | check during migration |
| `sidebar.rs:765`, `shell_ui.rs:1057` | menu `hovered_reason` | keep (the menu's own API) |
| `right_dock.rs:597–598`, `:881` | a dock row's hover and its stop key, a click | move |
| `right_dock.rs:410`, `forum_tab.rs:875`, `usage_dialog.rs:410` | pointer inside a scroll pane, moved by the scroll offset | goes away where its rows are sensed (the scene clips and undoes the scroll); the offset must go with it |
| `forum_tab.rs:1021`, `:872` | chip hover, board rows | move |
| `main_view.rs:575` | context bar click | move |
| `terminal_tab.rs:569`, `:690` | pointer over the terminal | move (starting a gesture); `:695`, `:731`, `:746` (which cell, during a selection or mouse report) keep the raw pointer |
| `usage_dialog.rs:421`, `:580` | table row hover, column hit | move |
| `usage_timeline_chart.rs:575` | which bar/point is under the pointer, the pinned card and its close button (`:579–592`) | sense the plot and pick the bar from `local_pos`; sense the card and its close button separately, after the plot, so they cover it |
| `providers_dialog.rs:2650` | colour swatch click | move |
| `providers_dialog.rs:1827`, `preferences_dialog.rs:754`, `usage_timeline_dialog.rs:220`, `questions_dialog.rs:516` | "outside the pane, the pointer is taken" for hand-placed rows | goes away only once every row and widget inside is sensed; widgets drawn there that still test immediately need it |
| `tree_rows.rs:72` | the same guard, for gameui `TreeNode` rows (`:95–110`) | keep until `TreeNode` itself is sensed (phase 4) |
| `drop_up.rs:109`, `task_popover.rs:362`, `usage_popovers.rs:288` | a click outside the popover closes it | keep, or answer from the backdrop's response (already registered) |
| `usage_popovers.rs:301` | the popover's close button | move |
| `disk_tree.rs:292` | where a context menu opens (screen position) | keep |
| `main.rs:833`, `:2869`, `:2876` | where the window's pointer is | keep (input plumbing) |

> **2026-10-22 update:** moved: the transcript; the right dock's cards (with the stop key
> sensed over them) and group headers; the forum's board rows and agent chips; the context bar;
> the usage dialog's scope rows and table rows; the timeline chart (its plots, then the pinned
> card and its ×, so they cover the plots); the provider colour swatches; the usage popover's ×;
> the terminal's shell list and screen (where a gesture starts); the sidebar key's tooltip.
> Two gameui additions made that possible: `UiContext::sense` (the swatches are drawn inside a
> `UiContext`) and `UiContext::icon_key` answering with its `Response` instead of a bool (the
> sidebar key already resolved through the scene, so sensing its rect a second time would have
> taken its click). `Sensor::input` lets code that takes a `Sensor` read the rest of the input.
> The scroll-offset guards went where everything inside is now sensed (the forum's boards, the
> usage scopes, the timeline). Kept: `sidebar.rs:430` (whether the pointer is anywhere in the dock,
> to fold older sessions when it leaves: an area, not an element), the menus' `hovered_reason`,
> the click-outside checks of the popovers, the "outside the pane" guards of the providers,
> preferences and questions dialogs and `tree_rows.rs` (gameui widgets inside still test the
> pointer themselves), the right dock's guard (its stop keys do too), the usage table's chart
> input (`BarChart`), the transcript's sideways-wheel and autoscroll checks, the terminal's
> cell under the pointer, and the files tab's menu position.

## Phasing

1. **gameui: `Sensor` and `sense`.** `WidgetId::hashed`, `Sensor`, `DrawContext::sense`;
   `Pressable` and `Checkbox` call it instead of repeating the steps. Tests drive several frames:
   - a sensed rect under a popup layer is not hovered, including the frame the popup opens and
     the frames after it moves or grows;
   - a press that began on a layer which then closes keeps the element below unpressed until
     release;
   - a rect under a later rect on the same layer is not hovered; a disabled rect on top still
     blocks the one under it, and a disabled context gives idle responses that still block;
   - an element that appears over another during press, hold, move and release never shares
     the gesture with it;
   - one under a scroll transform is hovered at its on-screen place, inside nested scrolls, and
     not where it is clipped away;
   - an element seen for the first time is idle with a scene, and answers immediately without one;
   - an element that turns disabled mid-gesture gets no click or release;
   - a child rect sensed after its parent wins over the parent, and a sensed element on top of
     a widget that still tests immediately (and the reverse): the immediate one is not
     protected from overlap within a layer until phase 4 moves it.
2. **gameui: layers block on their own.** `UiState::end_frame(&LayerStack)`, the menu bar's
   labels on its viewport layer while open, `blocker_regions` and `block_under_layer` removed.
   The menu bar's tests keep passing (those about `blocker_regions` itself go with it),
   including widgets that test immediately staying blocked under an open menu while the bar's
   labels still switch menus on hover, and a sensed base widget under the open menu staying
   blocked. New tests: a host-drawn modal blocks a sensed
   base element with no extra call, and Tab stays inside the top one of two stacked modals.
3. **agent-ui: move the checks.** The transcript first, with frame-loop tests (the desktop's
   test rigs): a popup over a row leaves no hover wash, no timestamp hint and no hand cursor,
   and a click that closes the popup doesn't toggle the row under it; a drag across rows and a
   release with text selected toggle nothing; a press on one image and release on another opens
   neither. Then the rest of the table above.

4. **gameui: its own widgets.** Once 1–3 work, lists, sliders, text inputs, toolbars and the
   rest move onto `sense` one at a time, each with tests for its own press and drag rules.

Phases 1 and 2 are independent; 3 needs 1, and its `block_under_layer` cleanup needs 2; 4 comes
after all three.

## Open questions (all answered)

1. **First frame of a new element.** The design answers idle when there's a scene (step 3 of
   `sense`): always correct, but a click in the one frame something appears is lost. That
   changes `Pressable` and `Checkbox`, which answer immediately today. The alternative keeps the
   immediate answer, but then it needs a rule for who owns the press when the new element
   overlaps an existing one, which the scene doesn't have.

   > **2026-10-22, Bart:** idle. Keep it simple; one lost frame (about 16 ms) won't be noticed.
2. **Should a `DrawContext` always have a scene?** Today `with_interactions` is optional, and
   without it `sense` falls back to the immediate test forever. Making the scene required would
   remove that path (a new id's first frame is still unresolved either way), at the cost of
   every host and test creating one. Codex's review suggests keeping it optional for now.

   > **2026-10-22, Bart:** keep it optional.
3. **gameui's own widgets.** Lists, sliders, text inputs, toolbars and the rest test the pointer
   immediately. Moving them onto the scene would make overlap within a layer correct everywhere,
   but it's a large change and each one has its own press/drag rules. While some widgets use
   the scene and others don't, overlap within a layer is only right between scene users.
   Options: leave them; move them one at a time as they're touched; or a dedicated pass after
   this lands. Codex's review suggests moving them one at a time once phase 1's press and
   position rules are settled.

   > **2026-10-22, Bart:** yes, after the rest works. Now phase 4.
4. **Hide the raw pointer?** Once `sense` exists, `mouse_x`/`mouse_y` could become private behind
   `pointer() -> Option<[f32; 2]>` (none when taken) plus an explicit `raw_pointer()` for drags,
   autoscroll and outside-click checks. That makes forgetting the consumed rule a compile error
   (not clipping, transforms or overlap, which only the scene handles), but it touches every
   gameui widget and every host. Codex's review suggests waiting until most widgets are sensed.

   > **2026-10-22, Bart:** no. The raw position stays public; it's still useful, and using it
   > correctly is a separate matter.
5. **How the menu bar opts out** in phase 2. It must stay a `Popup` for input routing, so not a
   new `LayerKind`; options are a flag on the layer (`push_popup` plus a `scene_blocking(false)`
   setter) or a separate `push_popup_scene_unblocked`.

   > **2026-10-22, Bart:** no opt-out. The menu bar's labels register on its viewport layer while
   > a menu is open (see "Layers block the scene on their own").

## Review

- 2026-10-22, Codex: checked the references and the design against the code. Folded in:
  disabled regions, `for_input` staying part of the contract, idle first frames, screen-space
  input for `Sensor`, release activation and `local_pos` limits, the scene-only opt-out for the
  menu bar, `top_modal` ordering, id scope and id-less widgets, and inventory fixes
  (`disk_tree.rs:292`, the timeline chart's card, `tree_rows.rs:72`). A second pass added:
  blanking a response when the element turns disabled, `released && hovered` for release
  activation, and tests for mixed and nested targets.

## References

- `crates/core/src/interaction.rs`: `InteractionScene`, `Response`, `for_input`, `block`
- `crates/core/src/widgets/mod.rs:484`: `DrawContext::interact`
- `crates/core/src/ui_context.rs:487–524`: `push_dropdown_layer`, `block_under_layer`
- `crates/core/src/layer.rs:292–340`: `input_for_layer`, how a popup takes the pointer
- `crates/core/src/widgets/pressable.rs:185–205`, `widgets/checkbox.rs:156–175`: the pattern `sense` replaces
- `docs/design/menubar.md`, "Layers and hit ordering": why the menu bar shapes its own blocking today
- agent-ui `crates/desktop/src/transcript_pane.rs:567`: the bug that started this
