# Menubar — Design

Status: partial — phases 1 and 2 (input foundations, the one-level widget) landed; phases 2b+ outstanding
Owner: Bart
Last updated: 2026-09-15

## Implementation status

Tracking the gap between this design and the main branch. Every phase that
produces a visible widget lands checklist-complete per `CLAUDE.md` (module +
export, unit tests, a rendered-and-eyeballed `widget_gallery` row, `TODO.md`
note) — there is no "implemented but not in the gallery" state.

### Done

- [x] **Phase 1 — input-model foundations.** `InputState` gained `alt_down`
      (held) plus `alt_pressed`/`alt_released` (per-frame edges, cleared by
      `end_frame`, zeroed by `consumed()` while `alt_down` survives);
      `FocusState::claim_click()` marks a click claimed without stealing focus;
      `DropdownState::begin_frame` takes `&mut InputState`, captures its
      `cancel`/`up`/`down`/`confirm` edges only while a list is open, and zeroes
      those fields in the shared input; `draw_open_layer` reads the captured
      edges instead of raw `nav`; `end_frame(&mut FocusState)` hands a *row*
      claim to focus; `UiState::begin_frame` now runs
      nav map → dropdowns → tree → focus → interactions and `end_frame` runs
      tree → dropdown → focus → interactions, documented as a contract.
      `hello_ui` maps `AltLeft`/`AltRight`. Covered by 12 new tests.
- [x] **Bug fixed as part of Phase 1 — Escape and row clicks no longer blur
      focus.** One Escape used to close an open dropdown *and* blur the focused
      field (`FocusState` blurred on the same edge), and choosing an option
      blurred the field the option acts on (Copy/Cut). Both are now regressions
      with tests at the widget level and through `UiState`.
- [x] **Unrelated bug found while implementing Phase 1, and fixed.** A closed
      but focused dropdown opened *and* immediately selected-and-closed on a
      single confirm edge (Space/Enter): `draw_open_layer` re-read the raw
      `nav.confirm` edge that `Dropdown::draw` had just used to open it. An open
      list now captures edges at frame-top and a closed list claims nothing, so
      the opening edge cannot also select. Pinned by
      `a_closed_dropdown_does_not_claim_the_confirm_edge_it_opens_on`.

### Outstanding

- [x] **Phase 2 — the widget, one level, checklist-complete.** Landed as
      `src/widgets/menubar/{mod,model,placement,state,paint,tests}.rs`, re-exported
      from the crate root (`MenuBar`, `MenuBarState`, `Menu`/`MenuItem`/
      `MenuItemId`/`MenuBarId`, `MenuTrigger`, `Accelerator`/`Key`/`Modifiers`/
      `AccelPlatform`, `SubmenuSide`, `ActivatedItem`, `MenuLayers`,
      `MenuDrawEnv`, `place_popup`, `blocker_regions`). Frame contract:
      `begin_frame(&mut InputState)` → `push_open_layers(&mut LayerStack)` → base
      input → `MenuBar::draw(rect, &mut state, &mut ctx) -> MenuBarOutput` →
      `draw_open_layers(..) -> Option<ActivatedItem>` → `end_frame(&mut FocusState)`.
      Three new `StyleKey` scalars (`MenuRowHeight`, `MenuItemMinWidth`,
      `MenuAccelGap`) with `Theme` fields/defaults/get/set. 46 unit tests
      (geometry, placement, the activation state machine, intent claiming, pointer
      dispatch, layer/region plumbing, buffer reuse). A `widget_gallery` row draws
      the strip with File open — accelerators, a separator, a check mark, a
      submenu chevron and a disabled row — seeded through the real input path
      (Alt tap + Down) rather than a test hook.
      Three places where the implementation narrowed the design as written:
      - **The bar's own left/right walk moved into `MenuBar::draw`.** It has to
        land before the chain's geometry pass (`collect_chain`) runs, or the menu
        the chain switches *to* cannot be measured until the frame after, which
        costs a second frame with nothing painted. Rows (up/down/confirm) and
        cancel still resolve in the chain's own pass.
      - **The chain's input half runs whether or not its geometry is paintable.**
        The promoted geometry is a frame behind the state, so the frame after a
        switch has nothing to draw; if that also disabled input handling, the
        arrow key that switched a menu would swallow the next one (only every
        other keypress would move the bar).
      - **`blocker_regions` returns 1, 2, 3 or 4 rects**, not "2 or 4":
        zero-area bands are dropped, so a strip docked against the viewport's top
        edge needs one region, a floating one needs three.
      Accepted consequence, pinned by tests: a menu opened this frame, and a menu
      switched this frame, paint no column — the geometry is measured during the
      bar's own draw and painted from the next promotion (one blank frame, the
      same class of latency the dropdown accepts).
      Rendering note: the menubar row is what surfaced the renderer's multi-pass
      corruption bug (missing base-layer fills, including this row's Accent
      label). Fixed by the `UiRenderer::begin_frame` frame boundary + per-pass
      uniform slots (`src/render/uniform_arena.rs`); see the checked P0 in
      `TODO.md`.
      Not in this phase: submenus render their chevron but do not open; hover
      intent/corridor, mnemonics, accelerator dispatch, item icons and wheel
      scrolling are later phases.
- [ ] **Phase 2b — swallow the outside press in the existing `Dropdown`
      (Decision B).** The dropdown pushes only its list rect, so a dismissal
      press falls through to the widget underneath, **and** the dropdown would
      need a viewport blocker excluding its own button rect; the menu's
      `blocker_regions` helper serves both. This also changes
      `single_owner_opening_b_replaces_a`: a press on another dropdown's button
      currently opens that one, and under a blanket blocker it would be swallowed
      (Open Question 3).
- [ ] **Phase 3 — recursion.** N levels, hover intent, the corridor rule, the
      close-on-sibling rule, a viewport blocker plus one popup layer and one
      column blocker region per level, per-level Escape, `MAX_MENU_DEPTH`
      truncation with a diagnostic; extended gallery row with a three-level
      chain; re-eyeball.
- [ ] **Phase 4 — accelerator dispatch.** `InputState::keys` (`KeyState`),
      `Accelerator::matches`, `MenuTrigger::KeyTap` (F10-style arming), the host
      key-mapping recipe, and tests proving the rendered hint and the matching
      value cannot diverge.
- [ ] **Phase 5 — mnemonics.** `Menu::mnemonic`/`MenuItem::mnemonic`, the
      trigger modifier + char activation, and underlined mnemonic glyphs via a
      `TextSpan` range.
- [ ] **Phase 6 — `UiContext` verbs.** `ui.menu_bar`, `push_menu_layers`,
      `draw_menu_layers`, `MenuState` on `UiState`, plus the `active_layer`
      plumbing the façade's generated `DrawContext`s currently lack
      (`src/ui_context.rs:1468`), and the `README`/`TODO.md` API notes.
- [ ] **Decide: `TextBlock` content ownership.** `TextBlock::new` takes
      `impl Into<String>` (`src/text.rs:3502`) and the block is consumed by
      `DrawList::text` (`src/widgets/draw_list.rs:1722`), so every text primitive
      owns its content — labels *and* accelerator hints each pay one allocation
      per frame. Accept it (status quo: the dropdown pays it with
      `item.clone()`, `src/widgets/dropdown.rs:315`), or land a borrowed/`Cow`
      content path first? Note this area is being actively edited by a parallel
      agent (a cosmic-text/letter-spacing migration), so it is not a free
      decision to bundle with menubar work.
- [ ] **Decide: popup column width caching.** Measure every frame the chain is
      open (simple, consistent with `Dropdown`) vs. cache per column keyed by
      `(menu id, font/style witness)` with an explicit invalidation rule.
- [ ] **Defer — `AccelTable`.** The lib provides `KeyState` + `Accelerator::matches`;
      the *host* owns the binding table and routing. A lib-owned table can be
      added later without breaking the API (Open Question 4).
- [ ] **Defer — drag-from-bar-to-item activation.** "Press File, drag down to
      Quit, release" needs chain-aware pointer capture the interaction scene
      does not currently model. v1 activates rows on press (Open Question 8).
- [ ] **Defer — wheel scrolling of a long column.** Keyboard-driven offset is v1;
      the wheel is not wired.
- [ ] **Defer — item icons.** A leading icon column (sprite id or string key,
      mirroring `TreeAction`).
- [ ] **Defer — `place_popup` back-fill into `Dropdown`.** The dropdown cannot
      flip or shift today (`src/widgets/dropdown.rs:161`).
- [ ] **Defer — using `Menu` for context menus.** The data model should serve
      right-click menus; no API is designed for it here.
- [ ] **Defer — accessibility/semantic output.** Covered by the `TODO.md` P2
      semantic-output item; nothing here emits semantics.

## Why this exists

The crate has no menubar. `Dropdown` is the only menu-shaped widget: a
single-select combo whose options are `&[&str]`, whose state holds exactly one
open id and one geometry (`src/widgets/dropdown.rs:98`), and which cannot nest.
`Tabs` is a proportional-width strip with no popups. `TreeNode` is a retained
tree *view*, not a floating menu. Consequently there is no way to express a
classic application menu bar: Alt to arm it, recursive submenus, mnemonics, or
accelerator hints.

Most of the plumbing a menubar needs already exists, which is why this is a
widget-scale project rather than a subsystem-scale one:

- **Popup layers with input gobbling.** `LayerStack::push_popup` blocks input
  only inside the popup's rect, and higher layers win (`src/layer.rs:191`,
  `src/layer.rs:238`).
- **A deferred-popup precedent.** `DropdownState::push_open_layer` pushes at
  frame-top from last frame's geometry so the popup can block input on the
  frame it opens (`src/widgets/dropdown.rs:180`); `draw_open_layer` paints after
  the base UI (`src/widgets/dropdown.rs:196`). `UiContext` already exposes the
  pair as verbs (`src/ui_context.rs:279`, `src/ui_context.rs:288`).
- **Retained interaction geometry with topmost-first dispatch.**
  `InteractionScene` resolves this frame's pointer against the previous
  completed frame and hands widgets a `Response`
  (`src/interaction.rs:177`, `src/interaction.rs:251`).
- **A device-agnostic navigation vocabulary.** `NavInput` carries
  `up/down/left/right/confirm/cancel/next/prev` (`src/lib.rs:259`), mapped from
  arrows, Enter, Escape and Tab (`src/nav.rs:157`).
- **A nav-ring precedent.** `TreeState` registers the frame's visible rows and
  resolves directional navigation at `end_frame` (`src/widgets/tree.rs:241`,
  `src/widgets/tree.rs:258`).
- **Right-aligned text and span underlines.** `TextAlign::Right` aligns within
  `max_width` (`src/text.rs:141`), and `TextSpan`/`Underline` support a
  byte-ranged underline for mnemonics (`src/text.rs:3320`, `src/text.rs:3361`).
- **Contextual measurement.** `MeasureContext::measure_text(TextBlock)` returns
  a `MeasuredText` with advance, ink bounds, baseline and line count
  (`src/measure.rs:299`), and `MeasuredText` transfers its owned block into
  paint without cloning (`src/measure.rs:198`). `Button::measure` is the worked
  example (`src/widgets/button.rs:195`).

Three real gaps have to be closed:

1. **Alt does not exist in the input model** — no field, no host mapping
   (`src/lib.rs:255` only has shift/ctrl; `examples/hello_ui.rs:342` maps only
   those two).
2. **Keyboard intents have no consumption channel, and this is already a live
   bug.** `FocusState` captures `nav.cancel` at `begin_frame`
   (`src/widgets/focus.rs:126`) and blurs unconditionally at `end_frame`
   (`src/widgets/focus.rs:213`); `DropdownState` captures the same edge
   (`src/widgets/dropdown.rs:132`) and closes on it (`src/widgets/dropdown.rs:357`).
   `UiState::end_frame` runs focus before the dropdown
   (`src/ui_context.rs:264`), so **one Escape closes an open dropdown *and*
   blurs the focused field**. Likewise a click on a dropdown row blurs focus,
   because `FocusState`'s `click_claimed` is only ever set by
   `FocusState::request` (`src/widgets/focus.rs:162`). A menubar cannot be built
   on top of that: Escape must unwind a menu level without touching focus, and
   activating an item must not blur the control the item acts on (Copy/Paste on
   the focused text field being the obvious case).
3. **There is no generic key vocabulary** for accelerators or mnemonics.

## Goals

- One declarative menubar: a strip of labelled menus, each an arbitrarily deep
  tree of items, separators, and submenu parents.
- Alt activation with the conventional machine: Alt arms the bar, arrows
  traverse, Enter activates, Escape unwinds one level at a time.
- Recursive dropdowns that behave like real menus: hover intent opens
  submenus, travelling diagonally into a child keeps its ancestors open, and an
  open sibling is replaced rather than left dangling.
- Optional per-item shortcuts with **one source of truth**: the accelerator
  value drives both the rendered hint and the match, so a displayed "Ctrl+S"
  cannot drift from what actually fires.
- Caller-owned state threaded by `&mut`, no globals — the same contract as
  `FocusState`/`DropdownState`/`TreeState`.
- Menu trees declared as `const`/`static` borrowed data for the common case, so
  a static menu costs no per-frame construction.
- Theme-relative sizing, full style resolution through `StyleKey`, and
  font-aware measurement through `MeasureContext`.

## Non-goals (v1)

- A retained or type-erased menu element tree.
- Callbacks stored inside the data model. `MenuBar` is a widget; the caller
  matches on the activation, the way `TreeState` yields a selected id.
- OS-native menus, menu proxies, or platform accelerator registration.
- Menus as `Tab` focus traps. Rows are pointer/keyboard-navigable *within the
  menu* but are not registered in the tab ring.
- A general command/keybinding system outside menus.
- User-rebindable accelerators.
- Drag from a bar label onto a row to activate it (deferred, see Outstanding).
- Semantic/accessibility output.

## Domain model

Menus are borrowed descriptions. Every constructor is `const fn`, so a whole
tree can live in `static`/`const` slices:

```rust
/// Namespace for one menubar's widget ids within the surface's shared
/// `InteractionScene`. Required, and must be unique per bar.
pub type MenuBarId = u64;
/// Stable identity for one item within a bar.
pub type MenuItemId = u64;

/// A single entry in a menu: an action, a submenu parent, or a separator.
pub struct MenuItem<'a> {
    id: Option<MenuItemId>,        // None → derived from the item's index path
    label: &'a str,
    children: &'a [MenuItem<'a>],  // empty = leaf
    separator: bool,
    enabled: bool,
    checked: bool,
    accel: Option<Accelerator>,    // drives display *and* matching
    accel_text: Option<&'a str>,   // display-only hint for host-handled shortcuts
    mnemonic: Option<char>,
}

impl<'a> MenuItem<'a> {
    pub const fn new(label: &'a str) -> Self;
    pub const fn separator() -> Self;
    pub const fn id(mut self, id: MenuItemId) -> Self;
    pub const fn children(mut self, items: &'a [MenuItem<'a>]) -> Self;
    pub const fn enabled(mut self, on: bool) -> Self;
    pub const fn checked(mut self, on: bool) -> Self;
    pub const fn accel(mut self, accel: Accelerator) -> Self;
    pub const fn shortcut(mut self, text: &'a str) -> Self;
    pub const fn mnemonic(mut self, ch: char) -> Self;
}

/// One top-level menu: its bar label plus the item tree it drops down.
pub struct Menu<'a> {
    id: Option<MenuItemId>,
    label: &'a str,
    items: &'a [MenuItem<'a>],
    mnemonic: Option<char>,
    enabled: bool,
}

impl<'a> Menu<'a> {
    pub const fn new(label: &'a str) -> Self;
    pub const fn items(mut self, items: &'a [MenuItem<'a>]) -> Self;
    pub const fn id(mut self, id: MenuItemId) -> Self;
    pub const fn mnemonic(mut self, ch: char) -> Self;
    pub const fn enabled(mut self, on: bool) -> Self;
}
```

Caller code, entirely static:

```rust
const RECENT: &[MenuItem<'static>] = &[
    MenuItem::new("build.log").id(101),
    MenuItem::new("main.rs").id(102),
];

const FILE_ITEMS: &[MenuItem<'static>] = &[
    MenuItem::new("New").accel(Accelerator::primary(Key::Char('N'))).id(FILE_NEW),
    MenuItem::new("Open…").accel(Accelerator::primary(Key::Char('O'))).id(FILE_OPEN),
    MenuItem::separator(),
    MenuItem::new("Open Recent").children(RECENT),
    MenuItem::separator(),
    MenuItem::new("Quit").accel(Accelerator::primary(Key::Char('Q'))).id(FILE_QUIT),
];

const MENUS: &[Menu<'static>] = &[
    Menu::new("File").mnemonic('F').items(FILE_ITEMS),
    Menu::new("Edit").mnemonic('E').items(EDIT_ITEMS),
];
```

### Description vs. state

`MenuItem` is a borrowed **description**; `MenuBarState` deliberately holds no
per-item state. `checked`/`enabled` in the description are the values to render
*for the borrow the caller hands in this frame*. A static menu therefore
renders static enabled/checked values, and a host whose toggles change owns its
own model and borrows a per-frame view of it (`Vec<MenuItem>` it retains and
rebuilds only when its model changes — never per frame). What the widget does
*not* do is cache or mutate item status, which keeps the "caller-owned state"
contract intact.

### Identity and namespacing

`InteractionScene` keys its responses by `WidgetId` in a single map
(`src/interaction.rs:157`, `src/interaction.rs:264`), and the crate's surfaces
share one scene (`UiState::interactions`, `src/ui_context.rs:172`). Menu ids
therefore have to be namespaced or they will collide with other widgets and
with a second menubar — which the scene reports as a duplicate-id diagnostic
rather than failing silently.

- `MenuBar::new` takes a required `MenuBarId`.
- A row's `WidgetId` is derived from `(MenuBarId, menu identity, item index
  path, level)`: cheap FNV folding, unique by construction, stable across
  frames while the tree shape holds, and independent of label text.
- A column's blocker region and its rows get distinct derived ids.
- An item's *activation* id is separate: `.id(n)` when set, otherwise derived
  from `(menu label, item label path)`. Explicit ids are what the caller
  matches on; derived ids exist so items the caller never matches still have a
  stable identity. Duplicate explicit ids within a bar are a diagnostic.

### Accelerators

One value, two uses:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// ASCII only: `set_down(Key::Char('é'), …)` is a debug assertion. Hosts
    /// fold case before mapping, so `'A'` and `'a'` are the same key.
    Char(char),
    F(u8),                             // F1..=F24
    Left, Right, Up, Down,
    Home, End, PageUp, PageDown,
    Enter, Escape, Tab, Backspace, Delete, Insert, Space,
}

/// `primary` is Ctrl on PC and ⌘ on Mac, matching `InputState::ctrl_pressed`'s
/// existing documentation (`src/lib.rs:257`).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Modifiers {
    pub primary: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Accelerator {
    pub mods: Modifiers,
    pub key: Key,
}

impl Accelerator {
    pub const fn new(mods: Modifiers, key: Key) -> Self;
    pub const fn primary(key: Key) -> Self;
    pub const fn primary_shift(key: Key) -> Self;
    /// Exact modifier agreement against this frame's key edges and the held
    /// modifier flags. Pure; unit-testable from a synthetic `InputState`.
    pub fn matches(&self, input: &InputState) -> bool;
    /// Writes the canonical hint ("Ctrl+Shift+S" / "⌘⇧S") into `out`.
    pub fn write_display(&self, platform: AccelPlatform, out: &mut String);
}
```

Deriving the hint from the accelerator is the point: the rendered and matched
values are the same value, so they cannot diverge. `MenuItem::shortcut(&str)` is
the escape hatch for hints the *host* handles; when both are set, `accel` wins
and a debug assertion flags the mistake.

`AccelPlatform` (`Pc` / `Mac`) selects words and glyphs only; it defaults to
`Pc` and the host sets it.

Ownership of *dispatch* is the caller's: the lib renders hints and exposes
`Accelerator::matches`; the host scans its own tree when a key press is present.
A lib-owned `AccelTable` is deferred (Open Question 4) — it would have to own
binding storage with its own lifetime and invalidation rules, and the crate's
existing ethos is that the caller matches on what the widget returns.

### Activation

```rust
pub struct ActivatedItem<'a> {
    /// The item's activation id (explicit `.id()`, else derived).
    pub id: MenuItemId,
    pub item: &'a MenuItem<'a>,
}
```

Submenu parents, separators and disabled items never activate. Activation is
reported from exactly one place (see the call order) so there is no ambiguity
about which result to read.

## Widget API

The menubar is two-phase for the same reason `Dropdown` is: a popup must be
registered before the frame's base input is resolved, or it cannot block what is
underneath it. The bar strip draws in the base layer; the open chain draws into
popup layers pushed at frame-top from the previous frame's geometry.

```rust
pub struct MenuBar<'a> {
    id: MenuBarId,
    menus: &'a [Menu<'a>],
    platform: AccelPlatform,
    side: SubmenuSide,             // Auto (default) | Right | Left
}

impl<'a> MenuBar<'a> {
    pub const fn new(id: MenuBarId, menus: &'a [Menu<'a>]) -> Self;
    pub const fn platform(mut self, p: AccelPlatform) -> Self;
    pub const fn submenu_side(mut self, side: SubmenuSide) -> Self;

    /// Contextual measurement of the bar strip: one `MenuRowHeight` tall, its
    /// width the sum of the measured labels plus padding.
    pub fn measure(&self, cx: &mut MeasureContext<'_>) -> Measurement;

    /// Draw the bar strip and resolve clicks on its labels. Base layer.
    pub fn draw(&self, rect: Rect, state: &mut MenuBarState, ctx: &mut DrawContext<'_>)
        -> MenuBarOutput;
}

/// Bar-level facts only. Activation is *not* reported here — rows live in
/// popup layers and are resolved by `draw_open_layers`, which runs later in the
/// frame; two activation channels would be ambiguous.
pub struct MenuBarOutput {
    pub bar_rect: Rect,
    pub armed: bool,
    pub open_menu: Option<usize>,
    pub hovered_menu: Option<usize>,
}
```

State, caller-owned, threaded by `&mut` exactly like `DropdownState`:

```rust
pub struct MenuBarState { /* private */ }

impl MenuBarState {
    pub const fn new() -> Self;

    /// Frame-top: capture this frame's Alt edges and pointer/nav state, and
    /// **claim** the navigation intents the menu will act on so no other widget
    /// sees them. Takes `&mut InputState` for that reason.
    pub fn begin_frame(&mut self, input: &mut InputState, dt: f32);

    pub fn armed(&self) -> bool;
    pub fn open_levels(&self) -> usize;
    /// True while the bar is armed or a chain is open. Hosts gate text/raw-key
    /// routing on this (the lib cannot suppress `text_input`; see Input model).
    pub fn wants_keyboard(&self) -> bool;
    /// Close every level and disarm.
    pub fn close(&mut self);

    /// Frame-top: push one popup layer per open level. The first is the
    /// viewport blocker (`input_for_base` gobbling); the rest are the columns.
    /// Returns a `Copy` token naming the contiguous range.
    pub fn push_open_layers(&mut self, layers: &mut LayerStack) -> Option<MenuLayers>;

    /// After the base UI: draw the open chain, register each column's blocker
    /// region, and resolve activation. The single activation channel.
    pub fn draw_open_layers<'a>(
        &mut self,
        layers: &mut LayerStack,
        slots: Option<MenuLayers>,
        menus: &'a [Menu<'a>],
        env: &mut MenuDrawEnv<'a>,
    ) -> Option<ActivatedItem<'a>>;

    /// Resolve dismissal, and tell `focus` not to treat the menu's own pointer
    /// gestures as click-elsewhere.
    pub fn end_frame(&mut self, focus: &mut FocusState);

    /// Seed an open chain for tests and for the gallery (the dropdown's
    /// `open_for_test` precedent, `src/widgets/dropdown.rs:366`).
    #[doc(hidden)]
    pub fn open_for_test(&mut self, path: &[usize]);
}

/// A `Copy` token for the contiguous popup-layer range pushed this frame:
/// `first` is the viewport blocker, and the `count` open columns are
/// `first + 1 ..= first + count`. Deliberately not a borrow of `MenuBarState`:
/// the caller has to pass `&mut MenuBarState` again to draw, so a borrow would
/// not compile. The columns are contiguous because they are pushed
/// consecutively at frame-top, which is also why `LayerStack::pop_layer` runs
/// immediately after each push — as the dropdown does, to keep the stack
/// balanced (`src/widgets/dropdown.rs:184`, `src/layer.rs:270`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuLayers { pub first: usize, pub count: usize }
```

`DrawContext` bundles a **single** `draw_list` (`src/widgets/mod.rs:85`), so a
multi-layer widget cannot take one and a `&mut LayerStack` at the same time —
that is exactly why `DropdownState::draw_open_layer` takes its resources
separately (`src/widgets/dropdown.rs:196`). The menubar needs more of them, so
they are bundled in an env struct:

```rust
/// Everything each popup level's `DrawContext` needs except the draw list,
/// which differs per level. Constructing a `DrawContext` per level is internal
/// detail; `active_layer` is set to that level so `OrderKey.layer` orders the
/// level above the base layer (`src/widgets/mod.rs:199`).
pub struct MenuDrawEnv<'a> {
    pub theme: &'a Theme,
    pub style: Option<&'a StyleOverlay>,
    pub input: &'a InputState,
    pub focus: &'a mut FocusState,
    pub interactions: &'a mut InteractionScene,
    pub animations: Option<&'a mut AnimationState>,
    pub cursor: Option<&'a mut CursorState>,
    pub screen_width: f32,
    pub screen_height: f32,
}
```

An `InteractionScene` is required, not optional: menu rows are resolved through
retained interaction dispatch, and a menu without a scene has no pointer
support at all. Tests construct one explicitly.

### Per-frame call order (normative)

```rust
// frame-top, in this order
state.menu.begin_frame(&mut input, dt);   // claims nav intents it will act on
state.menu.push_open_layers(layers);      // popups block base input from here
let base = layers.input_for_base(&input); // base layer sees claimed/blocked input

let bar = MenuBar::new(MENU_BAR, MENUS).draw(bar_rect, &mut state.menu, &mut ctx_for_base);

let activated = state.menu.draw_open_layers(
    layers, slots, MENUS, &mut MenuDrawEnv { /* …, input: &input */ },
);

state.menu.end_frame(&mut state.focus);
state.interactions.end_frame();
```

Because `begin_frame` runs before the base layer is drawn, base widgets read
`nav` *after* the menu claims it — which is what makes within-frame priority
work without a new consumption flag. See "Frame-top intent claiming".

## Input model changes

### 1. Alt: held state plus edges

`shift_pressed`/`ctrl_pressed` are held snapshots, but a bare Alt *tap* is a
gesture: a press and release that both land between two rendered frames would
be invisible to held-only state. Alt therefore gets the mouse's three-field
shape (`mouse_down`/`mouse_clicked`/`mouse_released`, `src/lib.rs:146`):

```rust
/// Alt is currently held (Alt/Option). Held state, like the other modifiers.
pub alt_down: bool,
/// Alt went down this frame (press edge).
pub alt_pressed: bool,
/// Every Alt key came up this frame (release edge).
pub alt_released: bool,
```

Both edges are per-frame and cleared by `InputState::end_frame`; only
`alt_down` persists. Shift/Ctrl stay held-only until something needs their
edges. The `hello_ui` example maps `AltLeft | AltRight` next to its existing
modifier arms (`examples/hello_ui.rs:342`).

### 2. Frame-top intent claiming (replaces a `nav_consumed` flag)

The obvious design — a `nav_consumed` bool that consumers set as they act — does
not work here, and the review killed it for three concrete reasons:

- `FocusState::end_frame` never sees an `InputState`
  (`src/widgets/focus.rs:211`); it acts on edges captured at `begin_frame`.
- The order is wrong: `UiState::end_frame` resolves focus *before* the dropdown
  (`src/ui_context.rs:264`), so a flag set by the dropdown arrives too late.
- Consumers during draw hold `&InputState` (`src/widgets/mod.rs:91`), and layer
  dispatch hands out clones (`src/layer.rs:238`), so there is no `&mut` path to
  set it.

Instead, **claiming happens at frame-top, by zeroing the intents in the shared
`&mut InputState`, in priority order, before anything reads them.** This needs
no new field: `NavInput` is a struct of independent `bool`s, so claiming is
per-intent by construction, and `InputState::consumed()` already establishes
that zeroing `nav` is how a layer says "not for you" (`src/lib.rs:374`).

Normative `UiState::begin_frame` order:

```text
nav.apply(input)          // NavMap fills this frame's intents
menu.begin_frame(&mut input, dt)
dropdown.begin_frame(&mut input)   // takes &mut to claim when open
tree.begin_frame(&input)
focus.begin_frame(&input)          // sees only what nobody claimed
interactions.begin_frame(&input)
```

and `UiState::end_frame`: `menu.end_frame(&mut focus)` → `dropdown.end_frame()` →
`tree.end_frame(tree_focused)` → `focus.end_frame(None)` →
`interactions.end_frame()`.

Concretely:

- The menu claims `cancel`, `up`, `down`, `left`, `right` and `confirm` when it
  will act on them (chain open, or bar armed), and never `next`/`prev` — Tab
  keeps cycling focus.
- `DropdownState::begin_frame` starts taking `&mut InputState` and claims
  `cancel`/`up`/`down`/`confirm` while it is open. That single change is what
  fixes the existing Escape bug, because focus now reads a claimed `nav.cancel`.
- `FocusState` needs no signature change at all.

This ordering is a contract, not an implementation detail: base widgets that
read `nav` during draw (`ListState` cursors, `Dropdown`'s raw
`input.nav.up/down/confirm` reads at `src/widgets/dropdown.rs:236`) must see the
claimed edges already gone. It is also why `end_frame` order changes: the menu
must decide dismissal before focus decides blur.

Typed characters are a separate channel: `nav` claiming cannot stop
`text_input` reaching a focused field, because that is not a nav intent. The
honest boundary is that the *host* gates text and `KeyState` routing while
`MenuBarState::wants_keyboard()` is true. That is specified, tested at the API
level (the flag is true while armed or open), and documented as a host
responsibility rather than pretended away.

### 3. Click claiming for focus

`FocusState` blurs on any click it did not claim (`src/widgets/focus.rs:216`),
and `click_claimed` is only ever set by `FocusState::request`
(`src/widgets/focus.rs:162`) — which also steals focus. Menus need "this click
was ours, leave focus alone":

```rust
/// Mark this frame's click as claimed without changing which widget is
/// focused. For widgets that own a click without being focus targets.
pub fn claim_click(&mut self);
```

`MenuBarState::end_frame(focus)` calls it when the menu owned the frame's
pointer gestures. The same fix applies to `DropdownState::end_frame`, which has
the identical bug today: clicking a dropdown row blurs the focused field.

### 4. Generic key edges (Phase 4)

Accelerators and mnemonics need a key vocabulary that the named-boolean fields
and `text_input` do not provide; `key_select_all`/`key_cut`/`key_copy`/`key_paste`
are the established precedent for host-supplied shortcut edges
(`src/lib.rs:236`). A fixed-size bitset, indexed by a total `Key → usize`
mapping (`Char` by ASCII code, 2 words; named keys, 1 word), keeps it
allocation-free:

```rust
pub struct KeyState { /* three [u64; 3] bitsets + len */ }

impl KeyState {
    /// Host bridge: records the transition atomically so `down`, `pressed` and
    /// `released` cannot disagree.
    pub fn key_event(&mut self, key: Key, down: bool);
    pub fn is_down(&self, key: Key) -> bool;
    pub fn was_pressed(&self, key: Key) -> bool;
    pub fn was_released(&self, key: Key) -> bool;
    /// Rising edges this frame, in `Key` order — the host's dispatch scan.
    pub fn pressed(&self) -> impl Iterator<Item = Key> + '_;
    /// Clears edges only; `down` persists like the modifier flags.
    pub fn end_frame(&mut self);
}
```

`InputState::keys: KeyState` sits beside `text_input`. `Key::Char` is ASCII-only
and case-folded by contract, so a non-ASCII `key_event` is a debug assertion;
`MenuItem::mnemonic` is folded the same way. Key repeat is the host's business —
`pressed` is an edge, not a repeat count.

## Activation and keyboard model

### Activation trigger

`armed` is a *mode*, not a key-held state: it is entered by an activation tap or
a bar click and left by a second tap, Escape with nothing open, or a click that
closes everything. Once armed, navigation is ordinary arrow/Enter keys; the
trigger need not stay held. This matches the classic behaviour, and it is why the
table is written in terms of states rather than key-halves.

**The trigger is configurable; Alt is only the default.**

```rust
/// What arms the bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MenuTrigger {
    /// A tap of Alt arms the bar. The default.
    #[default]
    AltTap,
    /// A tap of a plain key — `F10` is the other classic menu key. Needs the
    /// generic key edges, so it arrives with Phase 4.
    KeyTap(Key),
}
```

`MenuBarState::with_trigger(MenuTrigger)` (and `set_trigger`) configures it; the
trigger lives in the *state*, not the widget, because arming is evaluated at
`begin_frame` frame-top — before the bar is drawn — so the bar can render armed
on the very frame the tap lands. A `KeyTap` trigger arrives with `KeyState`
(Phase 4); making Ctrl/Shift tappable would mean giving those modifiers the same
held-plus-edges treatment Alt now has, which nothing has needed yet.

The armed bar is **visibly** armed: the highlighted menu's label draws with the
hover/active background, and while a chain is open the label whose menu is open
stays highlighted. Arming must never be invisible — that is the whole point of
the mode.

| Event | State | Result |
|---|---|---|
| trigger tap | disarmed, nothing open | arm; highlight the first enabled menu, visibly |
| trigger tap | armed, nothing open | disarm; clear the highlight |
| trigger tap | chain open | close the whole chain; disarm |
| trigger release | armed, nothing open | stay armed (a bare tap leaves the bar armed) |
| trigger release | chain open | stay open and armed (menus are now ordinary keyboard menus) |
| `left`/`right` | armed | move the highlighted top-level menu (wraps) |
| `down`/`confirm` | armed, nothing open | open the highlighted menu; highlight its first enabled item |
| `cancel` | chain open | close the deepest level; if that empties the chain, stay armed |
| `cancel` | armed, nothing open | disarm |
| `cancel` | disarmed, nothing open | not claimed; flows to focus/other widgets |
| bar label press | any | open that menu (or close it if it is already open) |
| bar label hover | armed, nothing open | move the highlight; do not open |
| bar label hover | chain open | switch the open menu immediately |
| outside press | chain open | close the chain **and swallow the press** (never reaches what is underneath) |

A bare tap leaving the bar armed is settled (Decision C): the tap arms it and it
stays armed until something disarms it. Mnemonics (Phase 5) use the same trigger
modifier as the arming chord, so `Alt`+letter follows from `MenuTrigger::AltTap`.

### Menu navigation

Driven by `NavInput`, so keyboard and gamepad share one vocabulary:

- **up/down** move the highlight within a level, skipping separators and
  disabled items, wrapping at both ends; the column scrolls to keep the
  highlight visible.
- **right** on a submenu parent opens it and highlights its first enabled item;
  on a leaf it moves to the next top-level menu, collapsing the chain back to
  one level. That asymmetry is the conventional behaviour and worth stating
  because it looks like a bug otherwise.
- **left** closes the current level and returns the highlight to the parent row;
  at the top level it moves to the previous top-level menu.
- **confirm** activates a leaf, opens a parent, and does nothing on a disabled
  item.
- **cancel** unwinds one level, per the table.
- **next/prev** (Tab) are never claimed.

Pointer and keyboard cooperate: a directional key sets the highlight, and the
pointer takes it back as soon as it moves. `DropdownState` fakes this with
"was a nav key down this frame" (`src/widgets/dropdown.rs:303`), which lets a
stationary pointer re-take the highlight on the next frame; `MenuBarState`
instead remembers the last pointer position and only reclaims the highlight
when it actually changes.

### Hover intent, the corridor rule, and closing

Three rules, all of which need specifying because they are where hand-rolled
menus visibly fail:

1. **Hover intent.** Hovering a submenu parent opens its child after
   `StyleKey::MenuHoverDelay` seconds. The delay is skipped when the parent menu
   was just opened by keyboard, or when a submenu of the same parent is already
   open (hovering a sibling parent then switches immediately). The accumulated
   hover time comes from the `dt` passed to `begin_frame` — `AnimationState`
   keeps its `dt` private (`src/animation.rs:170`) and has no accessor, so the
   menu does not scavenge it.
2. **Corridor rule.** Keeping ancestors open while the pointer is *inside* a
   child popup is not enough: a diagonal move toward the child crosses sibling
   rows while still inside the parent column, which is exactly the failure the
   rule is meant to prevent. The child therefore stays open while the pointer is
   either (a) inside the child's rect, or (b) inside the triangle shed by the
   pointer's previous position and the child's leading edge — the classic
   "safe triangle" test. Leaving both cancels the protection and starts the
   close timer.
3. **Replacement.** Settling on any non-ancestor row of the parent starts the
   same `MenuHoverDelay` timer to close (or replace) the open child, cancelled
   if the pointer reaches the child or the corridor first. Without this the
   child either stays open forever or snaps shut during a diagonal move.

### Pointer model

Activation is on **press**, consistent with every other widget in the crate
(`Response.clicked` is the press edge, `src/interaction.rs:130`). The
consequences are stated rather than hidden:

- A row that appeared this frame cannot be clicked: `InteractionScene` resolves
  against the previous frame, so its first `Response` is `resolved: false`
  (`src/interaction.rs:264`). That is desirable — the press that opened a menu
  must not also select the row it revealed — and it is tested.
- "Press File, drag to Quit, release" therefore does not activate, because the
  press was captured by the bar label and the release arrives on a different
  region. Enabling it properly needs chain-aware capture; deferred
  (Open Question 8).
- An outside press closes the chain **and is swallowed**: it never reaches the
  widget underneath (Decision B). The blocker regions described under "Layers and
  hit ordering" are the mechanism — they are not only for hover, they are what
  makes the dismissal press vanish. The same treatment is required for the
  *existing* dropdown, which currently lets the dismissal press through
  (`src/widgets/dropdown.rs:353`); that is tracked in Outstanding.
- **Two sub-cases the blocker cannot answer on its own**, both flagged in Open
  Questions rather than guessed:
  - A press on *another popup owner's* trigger. The bar's own labels are handled
    by construction (blocker regions exclude the bar strip), so clicking another
    menu's label switches rather than dismisses. A press on a *dropdown button*
    elsewhere, while a menu chain is open, is under the blocker and would be
    swallowed — a two-click interaction.
  - Whether a swallowed outside press also blurs focus. The design says **no**:
    the press was never delivered to a widget, so it is claimed
    (`FocusState::claim_click`) rather than treated as click-elsewhere. Flipping
    that is a one-line change, but it is Bart's call.
- **The swallow must be a layer-level mechanism, not input mutation.** Zeroing
  the press in the *shared* `InputState` would also hide it from a modal or
  higher popup layered above ([`input_for_layer`](src/layer.rs:238) derives every
  layer's input from that same value). Blocker layers get this right by
  construction: they only affect what is *below* them.

## Geometry and layer strategy

### Placement

Placement is a pure function, so flip/shift behaviour is unit-testable headlessly:

```rust
/// Where a popup column goes, given the anchor it hangs off, its own measured
/// size, the viewport it must stay inside, and the preferred side.
pub fn place_popup(
    anchor: Rect, size: Size, viewport: Rect, side: SubmenuSide,
) -> (Rect, SubmenuSide);
```

- The top-level column drops below its bar label, left edges aligned. Because a
  game may dock the bar at the bottom, the top level **flips above** the label
  when it would overflow the bottom (it does not just shift up, which would
  cover the bar and fight label-hover switching).
- Submenus open to the right of the parent row with a slight overlap onto the
  parent column so the pointer can travel without crossing a gap.
- `Auto` flips to the left when the column would overflow the right edge, and
  flips per level, so a deep chain can flip back and forth as needed.
- `Right`/`Left` are honoured unconditionally and then **shifted** to stay
  inside the viewport; only `Auto` flips. The tests encode that distinction.
- The viewport is a `Rect`, not a screen `Size`, so an inset game viewport or a
  non-zero-origin surface places correctly.

`Dropdown` has no flip or shift logic at all today
(`src/widgets/dropdown.rs:161`); back-filling this is deferred.

### Layers and hit ordering

- The bar strip and its labels live in the base layer (layer 0), registered
  through `DrawContext::interact`.
- Each open column gets its own popup layer at frame-top, pushed (and
  immediately popped to keep `LayerStack` balanced) from the promoted chain.
  Columns resolve above the base layer because `DrawContext::interact` derives
  `layer = active_layer + 1` (`src/widgets/mod.rs:199`).
- The first pushed layer is a **viewport blocker popup**, covering the whole
  viewport, so `input_for_base` consumes pointer input for every legacy
  (non-scene) widget below it while a chain is open (`src/layer.rs:238`).
- `InteractionScene` knows nothing about `LayerStack` rects — it dispatches
  among *registered regions only*. So the blocker is also registered as scene
  regions, and those must **exclude the bar strip**: a register-the-whole-
  viewport region would rank above the base layer and kill the bar labels, which
  must stay live for hover-to-switch. `blocker_regions(viewport, bar_rect,
  out: &mut [Rect; 4]) -> usize` is a pure, unit-testable helper returning the
  viewport minus the bar strip as up to four rects (one when the strip is docked
  against an edge and spans the full width, two when it spans the width
  mid-viewport, three when it floats below the top edge, four when it floats
  mid-viewport). Zero-area bands are dropped rather than returned. The regions
  are registered at the blocker layer, so they beat base-layer widgets everywhere
  except inside the strip.
- Each open column additionally registers a **column blocker region** covering
  its full rect (distinct namespaced id, `PointerPolicy::Target`, at that
  column's layer). Without it, a scene-registered base widget underneath a
  column's padding, border or separator would still win hover and click
  dispatch. The dropdown never hit this because it does not use the scene at all.
- `blocker_regions` needs the bar strip's rect. `MenuBar::draw` records it in
  `MenuBarState`, and `draw_open_layers` — which runs later in the same frame —
  reads it; the promoted rect from the previous frame is the fallback for a
  frame in which the host did not draw the bar.
- Layers in effect: base/bar rows `0`, blocker regions `first + 1`, column `L`
  rows and blocker `first + 2 + L` — so columns beat the blocker, and the blocker
  beats everything in the base layer outside the strip.
- The bar's labels resolve through the scene, so the consumed base input from the
  blocker popup does not disable them (scene responses ignore
  `InputState::mouse_consumed`).
- Menu rows are registered with `DrawContext::interact`. It always registers
  `HitShape::Rect` (`src/widgets/mod.rs:203`) — there is no rounded hit shape
  through this path — which is fine for menu rows.
- Rows are **not** registered with `FocusState`: menus are not Tab traps
  (non-goal), so the façade's `active_layer` focus-scoping question does not
  arise for them.

One-frame consequences, accepted and tested: a level that closed this frame
still has its previous regions in the scene for one more frame, and its popup
layer still gobbled input for the frame it closed on. This is the same class of
latency the dropdown already accepts.

The chain's own *paint* is one frame behind its state, because the layer rects
that block the base layer can only come from the previous frame's measurement.
So two frames paint no column: the frame a menu is opened on (its geometry is
measured after the bar draws), and the frame the bar switches menus on. Input is
**not** delayed with it — the chain takes its keyboard whenever it is open, even
with nothing paintable, which is what keeps arrow-key switching from swallowing
every other press. The switch case is one frame rather than two because the
bar's left/right walk resolves before the geometry pass in the same frame.

### Sizing

- Row height comes from `StyleKey::MenuRowHeight`, defaulted from `font_size`
  plus padding so re-theming and DPI scaling work. The crate currently
  hard-codes row heights in widgets (`Dropdown`'s `ITEM_HEIGHT`,
  `src/widgets/dropdown.rs:61`).
- Labels are measured through `MeasureContext::measure_text`
  (`src/measure.rs:299`) — the contextual path, not the older default-font
  `DrawList::measure_text` — with `Button::measure` as the template
  (`src/widgets/button.rs:195`).
- A column's width is the max over its visible items of
  `label_w + accel_gap + accel_w` plus horizontal padding, so hints align in one
  right-hand column. That needs one measuring pass before the column's rect is
  known, collected into a flat `Vec<RowMetrics>` retained in `MenuBarState`
  (the `MeasureBuffer`/`LayoutResult` scratch idiom,
  `src/measure.rs:442`).
- The measured `MeasuredText` is **retained for the frame and consumed into
  paint** via `into_block_at` (`src/measure.rs:198`), so the width pass and the
  paint pass are one measurement and one block, not a measure-then-rebuild
  (`MeasuredText` is deliberately not `Clone`).
- Hints draw with `TextBlock::with_align(TextAlign::Right)` against the column
  width; alignment is relative to `max_width` (`src/text.rs:141`), so the block
  must carry the real column width.
- Labels ellipsize with `with_ellipsis()` + `with_max_width(...)`, the only
  available route (`ellipsize_to_width` is private, `src/text.rs:3247`), and the
  same one the dropdown uses (`src/widgets/dropdown.rs:319`).
- Mnemonics are underlined with a `TextSpan` over the mnemonic character's byte
  range (`src/text.rs:3361`), so no prefix measurement is needed.

### Scrolling

A column taller than the viewport is **clamped and scrolled** — the keyboard
highlight auto-scrolls it into view by clamping a per-level `scroll_offset`
against the measured content height, clipped to the column rect. Without this,
items past the fold would be unreachable while navigation promised to reach
them. Wheel scrolling is deferred; the offset is a plain `f32` per level in
`MenuBarState`, not a `ScrollView`.

### Theme and style keys

**Decision A (settled): reuse the existing palette, and extend only the
scalars.** No new colour keys at all. Popup background/border reuse
`Panel`/`PanelBorder`, row text reuses `Text`, hover and the armed/highlighted
label reuse `ButtonHover`, the open menu's bar label reuses `Accent`, disabled
and hint text reuse `TextDim`, separators reuse `PanelBorder` + `BorderWidth`.
This is exactly what `Dropdown` does today (`src/widgets/dropdown.rs:274`).

Four new *scalar* keys, each needing a real `StyleKey` variant + `Theme` field +
`get`/`set` arms + default. Extend the scalar list further if implementation
needs it — new scalars are cheap and theme-relative; new colours are not wanted:

| Key | Meaning | Default |
|---|---|---|
| `MenuRowHeight` | height of a bar or item row | `font_size` + 2 × `padding` |
| `MenuItemMinWidth` | column width floor | `font_size` × 10 |
| `MenuAccelGap` | label → hint gap | `spacing` |
| `MenuHoverDelay` | seconds before hover intent opens a child | ~0.3 |

Core variants (not `StyleKey::custom`) because a first-class widget should be
themable and discoverable, and `Custom` keys are excluded from
`StyleKey::is_color` (`src/style.rs:165`).

`Theme` has no shadow/elevation field, so popup elevation is expressed only by
fill and border — as the dropdown already does.

## Performance model

- **Steady state avoids allocation on the state, geometry and dispatch paths.**
  The open chain, row-metrics scratch, layer slots, per-level scroll offsets and
  hover timers are caller-owned and capacity-retained in `MenuBarState`. No
  per-frame `Vec`, no menu description rebuilt.
- **Text owns its content, and that is a pre-existing crate-wide cost.**
  `TextBlock::new` takes `impl Into<String>` and `DrawList::text` consumes the
  block, so each label and each hint owns one `String` per frame
  (`src/text.rs:3502`, `src/widgets/draw_list.rs:1722`). A reused scratch
  `String` cannot be moved into a queued block, so there is no way to format a
  hint per frame without that allocation; the honest statement is that the
  menubar pays the same per-primitive cost as every text widget, once per
  visible label and once per visible hint. Whether to change that crate-wide is
  Open Question 1, and the menubar's allocation tests exclude it explicitly
  until answered.
- **One measuring pass per visible item per open column per frame.** This is
  *more* than the dropdown does: `Dropdown` derives its popup width from the
  button rect and never measures its rows (`src/widgets/dropdown.rs:161`).
  A menu must measure to size columns, so the budget below is a real boundary.
- **A pushed layer is not cheap.** `LayerStack::make_list` creates a fresh
  `DrawList` per push (`src/layer.rs:169`), whose transform and tint stacks
  allocate immediately. With the blocker plus N columns, an open chain costs
  N+1 `DrawList`s per frame. TODO.md's P1 keyed layers (`TODO.md:97`) is the
  systemic fix; the menubar records the cost rather than blocking on it, and the
  benchmark below makes it visible.
- **Dispatch is a short linear scan** over the caller's own accelerated items,
  and it must be **skipped entirely when no key was pressed this frame**
  (`KeyState::pressed()` empty).
- **Benchmarks at the adversarial boundary**, not a small fixture: maximum open
  depth with wide columns, a large closed tree with and without a key press, and
  a no-key steady-state frame. The earlier plan's "40-item two-level menu" was
  not adversarial for an API that allows eight levels and unbounded breadth.

## Testing plan

Headless, against `DrawList`/`Response`/state, no GPU — the shape
`src/widgets/dropdown.rs:600` and `src/widgets/tree.rs:702` already use.

**Pure geometry**
- `place_popup`: opens below and right by default; top level flips above a
  bottom-docked bar; `Auto` flips left at the right edge and per level; `Right`/
  `Left` are honoured and shifted into the viewport; a non-zero-origin viewport
  is respected; a column taller than the viewport is clamped and scrollable.
- `blocker_regions`: the viewport minus the bar strip yields 2 rects when the
  strip spans the full width and 4 when it floats; the regions never overlap the
  strip; they cover every point of the viewport outside it.
- Bar layout: labels are intrinsic width, adjacent in declaration order; the
  strip is one `MenuRowHeight` tall.
- Column width = max label + gap + max hint + padding, with hints right-aligned
  in a single column across rows.

**State machine**
- Alt press arms; a second press disarms; Alt press with a chain open closes and
  disarms; a bare Alt tap (press + release between frames) leaves the bar armed.
- Arrows traverse the bar and wrap; down/confirm opens; up/down walk rows
  skipping separators and disabled items, wrapping at both ends; the column
  auto-scrolls to the highlight.
- Right on a parent opens it; right on a leaf moves to the next top-level menu
  and collapses the chain; left closes a level and returns the highlight to the
  parent row.
- Escape unwinds exactly one level; from "armed, nothing open" it disarms; from
  "disarmed, nothing open" it is not claimed.
- Hover intent: the child opens only after `MenuHoverDelay`; a sibling switch is
  immediate when a submenu is already open; the corridor keeps the child open
  during a diagonal move; settling on a non-ancestor row starts the close timer.
- Bar label press opens/toggles; hovering a label while armed moves the
  highlight without opening; hovering while open switches.
- Disabled and separator items never activate and are never highlighted.
- Activating a leaf closes the chain and returns that `ActivatedItem`.
- Depth beyond `MAX_MENU_DEPTH` truncates with a diagnostic and renders the
  too-deep parent as non-openable — no panic. Cyclic static menu data is
  covered, not just deep acyclic data.

**Input integration**
- Escape that unwinds a menu does **not** blur a focused widget, and neither
  does Escape that closes a dropdown (the existing bug).
- Clicking a menu row does not blur a focused widget (`claim_click`); clicking a
  dropdown row does not either.
- Alt alone does not blur, does not tab, and does not move the focus ring.
- Menus never claim `next`/`prev`, so Tab still cycles focus.
- A press outside the chain closes it *and* reaches the base layer
  (click-through, matching the dropdown); a press on a row that appeared this
  frame is not delivered.
- A scene-registered base widget under a column's **padding/border/separator**
  loses pointer dispatch to the column blocker — tested with an
  interaction-backed base widget, not a legacy rect hit-test.
- `MenuBarState::wants_keyboard()` is true while armed or open, so a host can
  gate text routing.

**Accelerators and mnemonics**
- `write_display` for `Pc` and `Mac`, with and without shift/alt.
- `matches` against a synthetic `InputState`: exact modifier agreement (an extra
  held modifier does not match), a held-but-not-pressed key does not match.
- The rendered hint's accelerator is the one that matches (property test).
- Items with both `accel` and `shortcut` trip the debug assertion.
- Non-ASCII `Key::Char` trips the debug assertion; mnemonics are case-folded.
- The mnemonic underline covers the right byte range.

**Gallery and allocation**
- A `widget_gallery` row: the bar, a three-level open chain, a separator, a
  disabled item, a checked item, a clipped deep column and right-aligned hints —
  rendered and *looked at* (the repo's checklist is explicit that this has
  caught real bugs).
- A steady-state frame with three levels open performs no heap allocation on the
  state/geometry path, with the per-primitive `TextBlock` content cost excluded
  and documented while Open Question 1 is open.

## Phasing

- **Phase 1 — input-model foundations.** Alt held-state + edges,
  `FocusState::claim_click`, the `DropdownState` claim fix, and the normative
  claiming order — with regressions for the existing Escape and click-elsewhere
  bugs. Independently useful and testable with no widget.
- **Phase 2 — the widget, one level, checklist-complete.** Module + export,
  data model, activation state machine, `place_popup`, scrolling, hints, tests,
  a `widget_gallery` row, the render-and-eyeball pass, and the `TODO.md` note.
- **Phase 3 — recursion.** N levels, hover intent, corridor, replacement,
  per-level blocker regions and popup layers, per-level Escape, depth bound;
  gallery row extended and re-eyeballed.
- **Phase 4 — accelerator dispatch.** `KeyState`, the host mapping recipe,
  `Accelerator::matches`; dispatch is host-routed. `AccelTable` deferred.
- **Phase 5 — mnemonics.**
- **Phase 6 — `UiContext` verbs** and the `README`/`TODO.md` API notes.

## Open questions for review

Settled by Bart on 2026-09-15: A (style keys — reuse the palette, extend only
scalars), B (outside press — swallow, for the dropdown and the menu), C (bare
trigger tap leaves the bar armed, and the trigger is configurable). See
"Decisions taken" in the review history. What remains:

1. **A swallowed outside press and a second popup owner (from Decision B).**
   Blocker regions can exclude the bar strip, so clicking another *menu label*
   switches rather than dismisses. But a press on a **dropdown button** while a
   menu chain is open sits under the blocker and would be swallowed, making it a
   two-click interaction — and symmetrically, swallowing in the *dropdown* turns
   `single_owner_opening_b_replaces_a` (clicking another dropdown's button opens
   it) into "the first closes, then a second click opens the other". Options:
   (a) accept the two-click behaviour as the literal reading of "swallow";
   (b) track popup-owner trigger rects (the dropdown owner already sees every
   button rect each frame) and never swallow a press that lands on one;
   (c) mark such regions with a new `PointerPolicy` variant and let the
   dispatcher decide.
2. **Does a swallowed outside press also blur focus?** Design says no (the press
   was claimed, not delivered). Flipping it is one line.
3. **`TextBlock` content ownership.** Every text primitive allocates its content
   string, so the menubar pays one allocation per visible label and per visible
   hint per frame. Accept the status quo (dropdown does the same), or land a
   borrowed/`Cow` content path or a content-interned `DrawList` first? This is
   actively being edited by a parallel agent, so it is not free to bundle.
4. **Accelerator dispatch ownership.** Lib provides `matches` and the host owns
   routing (specified), or a lib-owned `AccelTable` polled once per frame?
5. **Column width caching.** Measure every open frame, or cache per column keyed
   by `(menu id, font/style witness)` with an invalidation rule?
6. **Item id ergonomics.** Explicit `.id()` for items the caller matches on,
   derived identity for everything else — acceptable, or should every item be
   required to carry an id?
7. **Drag-from-bar-to-item.** Defer (specified), or invest in chain-aware
   pointer capture so "press File, drag to Quit, release" works?
8. **`place_popup` back-fill into `Dropdown`.** Worth a follow-up, since the
   dropdown cannot flip or shift at all today? It is now a shared helper anyway
   (`blocker_regions` joins it for Phase 2b).
9. **Context menus.** Should the data model serve right-click menus as a named
   goal and phase here, or a separate follow-up?

## Review history

Two independent reviews (Codex and Claude, read-only, over the first draft)
verified the doc's citations and converged on the same blockers. The draft was
revised as follows; the rejected alternatives are recorded because they were
serious candidates.

### Decisions taken (2026-09-15, after review)

- **A — style keys: reuse the palette, extend only the scalars.** No new colour
  keys; the four scalars above, more if implementation needs them. Option B (a
  dedicated nine-colour menu palette) is dropped.
- **B — outside press is swallowed, for the existing dropdown as well as the new
  menu.** That differs from the crate's current dropdown behaviour, so it becomes
  its own tracked work item (Phase 2b) rather than a silent change, and it made
  the blocker design load-bearing rather than cosmetic. Note the mechanism
  constraint this surfaced: the swallow must be layer-level (blocker layers),
  because zeroing the press in the shared `InputState` would also hide it from a
  modal layered above.
- **C — a bare trigger tap leaves the bar armed, and it must be visibly
  highlighted.** The trigger itself became configurable (`MenuTrigger`, Alt by
  default, `F10`-style `KeyTap` with Phase 4), since "alt by default" was the
  phrasing rather than "alt only".

**Adopted changes**

1. **Replaced `nav_consumed` with frame-top intent claiming.** The draft proposed
   a `nav_consumed` flag set by consumers as they acted. Both reviews showed it
   cannot work: `FocusState::end_frame` never sees an `InputState`, the
   `UiState::end_frame` order resolves focus before the dropdown, and consumers
   hold `&InputState`/clones with no `&mut` path. Claiming now happens at
   frame-top by zeroing individual `NavInput` fields in the shared
   `&mut InputState`, in a normative priority order — no new field, per-intent
   precision for free, and it also fixes the within-frame priority inversion
   where base widgets read `nav` during draw. `DropdownState::begin_frame` taking
   `&mut InputState` is what actually fixes the pre-existing Escape bug.
2. **Reworked the multi-layer draw API.** `&mut DrawContext` plus
   `&mut LayerStack` cannot coexist (a `DrawContext` owns one draw list), and a
   `MenuLayers` that borrowed `MenuBarState` could not be passed back into a
   `&mut self` method. Now: explicit resources in a `MenuDrawEnv`, a `Copy`
   `MenuLayers` range token, and an explicit note that each push is immediately
   popped to keep the stack balanced.
3. **Added column blocker regions.** `InteractionScene` dispatches among
   registered regions only, so a scene-backed base widget under a column's
   padding/border would have won pointer dispatch. Each column now registers a
   full-rect blocker at its own layer, and a viewport blocker covers the rest —
   as a popup layer for legacy input and as scene regions for scene-backed
   widgets, with those regions excluding the bar strip so the bar's
   hover-to-switch stays live. The test for this must use an interaction-backed
   base widget, not a legacy hit-test.
4. **Namespaced ids and a single activation channel.** `MenuBarId` is required;
   row ids derive from `(bar id, menu identity, index path, level)`;
   activation returns `ActivatedItem { id, item }`; duplicate explicit ids and
   duplicate accelerators are diagnostics. `MenuBarOutput` no longer claims to
   report activation, because rows are resolved after the bar draws.
5. **Alt got real edges.** Held-only Alt would miss a tap that lands entirely
   between two frames, so Alt now has `alt_down`/`alt_pressed`/`alt_released`
   (the mouse's three-field shape). The draft's contradiction — "Alt release
   stays armed" in the table vs. "Alt release disarms" in `end_frame` — is gone,
   and `armed` is now a mode rather than a key-held condition, which also closes
   the missing "plain arrows while armed" row.
6. **Fixed the corridor rule.** Keeping ancestors open only while the pointer is
   *inside* the child does not protect the diagonal transit, which crosses
   sibling rows inside the parent column. Added the safe-triangle corridor plus
   an explicit "settling on a non-ancestor row starts the close timer" rule.
7. **Moved scrolling into v1.** The draft promised auto-scroll to the
   highlight while deferring scrolling, which left items past the fold
   unreachable. Keyboard-driven clamp-and-offset is now part of Phase 2/3; the
   wheel stays deferred.
8. **Corrected the measurement story.** The draft claimed the menu's measuring
   pass "matches what `Dropdown` already does" — it does not; the dropdown never
   measures its rows. The pass is now named as genuinely more expensive, with a
   budget and adversarial benchmarks, and the measured `MeasuredText` is
   retained and consumed into paint rather than measured and rebuilt.
9. **Corrected the allocation claims.** A reused scratch `String` cannot be
   moved into queued `TextBlock`s, so hints do allocate; that is now stated
   plainly instead of described as reusable scratch. Pushed layers are also
   more than "a handful of allocations" per frame.
10. **Fixed depth-overflow behaviour.** `debug_assert` plus "no panic" was
    self-contradictory, and `DebugReport` is a post-hoc lint over a `DrawList`,
    not a runtime diagnostics channel (`src/debug.rs:147`). Now: truncate, render
    the too-deep parent as non-openable, log, expose a counter, and test cyclic
    data.
11. **Added the focus claim path.** `MenuState`'s own `click_claimed` did not
    stop `FocusState` blurring, and menus must not blur the control they act on
    (Copy/Paste). Added `FocusState::claim_click` and applied the same fix to
    `DropdownState`. Also specified `wants_keyboard()` for hosts gating text
    routing, since `nav` claiming cannot stop `text_input`.
12. **Corrected minor claims and citations**: `DrawContext::interact` always
    registers a *rect* hit shape (no rounded shapes); `TextAlign::Right` is
    `src/text.rs:141`; `DropdownState`'s `click_claimed` is
    `src/widgets/dropdown.rs:115`; the dropdown's hover-vs-keyboard behaviour is
    not the pattern the menubar will use.
13. **Presentation**: style keys now offer reuse (A) vs. a dedicated palette (B)
    instead of asserting a convention the repo does not actually enforce;
    `AccelTable` moved from a phase to a deferred decision; mnemonics no longer
    need prefix measurement (a `TextSpan` byte range does it); the phasing was
    reshaped so each landed phase is `CLAUDE.md`-checklist-complete rather than
    deferring the mandatory gallery row to phase 7.

**Considered and rejected or deferred**

- A full-viewport *catcher* region so an outside press is swallowed rather than
  passed through. It is the conventional desktop behaviour, but the crate's
  existing dropdown is click-through, so changing it here would be a silent
  behaviour change; recorded as Open Question 3.
- A lib-owned `AccelTable`. Rejected for v1: it needs binding storage with its
  own lifetime and invalidation rules, the caller must rescan the tree to return
  the `&MenuItem` anyway, and the crate's ethos is that the caller matches on
  what the widget returns.
- A general per-intent consumption *mask* carried on `InputState` (Codex's
  alternative). Rejected in favour of zeroing the existing `NavInput` fields,
  which is already per-intent and needs no new public surface.
- Making the whole second half of the frame order a documented contract rather
  than leaving `UiState` call order incidental — adopted, but flagged here
  because it changes behaviour for `ListState`/`TreeState` while a menu is open.

## References

- `src/widgets/dropdown.rs` — deferred popup, single-owner state, raw-input
  precedent and its limits.
- `src/layer.rs` — `LayerStack::push_popup`, `input_for_base`/`input_for_layer`.
- `src/interaction.rs` — `InteractionScene`, `HitRegion`, `Response`,
  `HitShape::RoundedRect`.
- `src/widgets/focus.rs`, `src/widgets/tree.rs`, `src/widgets/list.rs` — nav
  rings, edge capture, click claiming.
- `src/nav.rs` — `NavInput`, `NavMap`, `KeyboardNav`.
- `src/style.rs`, `src/theme.rs` — `StyleKey`, `StyleResolver`, `Theme`.
- `src/measure.rs`, `src/widgets/button.rs` — contextual measurement and the
  prepared-text transfer path.
- `src/text.rs` — `TextBlock`, `TextAlign`, `with_ellipsis`, `TextSpan`/`Underline`.
- `src/ui_context.rs` — `UiState` frame hooks, dropdown verbs, `UiContext::measure`.
- `TODO.md` — popup/menu priorities (`:32`), keyed layers (`:97`), dropdown
  keyboard nav (`:267`), popup layer (`:282`).
- `docs/design/contextual-widget-lifecycle.md` — the measurement/arrangement
  contract the menubar follows.
- `CLAUDE.md` — the widget checklist each phase must satisfy.
