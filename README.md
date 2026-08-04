# wgpu-gameui

A custom, wgpu-based immediate-mode game UI library. Originally extracted from a
city builder game and built to replace egui in contexts where UI *aesthetics*
matter — polished chrome, MSDF text with outlines/shadows/glow, nine-slice
framing, per-subtree styling, and a Teardown-style immediate-mode verb API.

- **Render-only.** No windowing, audio, or device I/O — the app owns the event
  loop and fills a plain `InputState` struct. Renders through a single
  `UiRenderer::render` call into a wgpu `TextureView`.
- **Immediate-mode.** Build a `DrawList` per frame from widget calls; nothing
  is retained across frames except the caller-owned state structs (`UiState`,
  `ScrollState`, `FocusState`, `DragCapture`, …).
- **MSDF text.** Glyphs are rendered through a custom multi-channel signed
  distance field atlas (via `fdsm`), so text supports outlines, shadows, and
  glow at any zoom without re-rasterizing. Shaping is cosmic-text.
- **Dual API.** Draw raw widgets against a `DrawContext` for full control, or
  use the `UiContext` / `Frame` façade for auto-advancing, stateful verbs
  (`text_button`, `slider`, `text_input`, …) — the Teardown port target.
- **Inspectable.** Ask a finished frame what it actually drew: `DebugReport`
  dumps a named tree of world-space boxes plus the things that look wrong with
  them (off-screen, overflowing its container, clipped away, misaligned), as
  text or JSON, with `assert_clean()` to guard a layout in a test. See
  [Debugging & layout inspection](#debugging--layout-inspection).

Dual-licensed MIT OR Apache-2.0.

---

## Quickstart

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
wgpu-gameui = "0.1"
```

The default features bundle Noto Sans (so the UI renders identically everywhere
without system fonts) and the Phosphor icon font. Disable them to slim the
binary:

```toml
wgpu-gameui = { version = "0.1", default-features = false }
```

### Minimal render loop

The library is windowing-agnostic. You bring the event loop (e.g. `winit`) and
wgpu surface; the UI side is three steps per frame:

```rust,ignore
use wgpu_gameui::{DrawList, InputState, Theme, UiRenderer, KeyboardNav, UiState, Frame};

// --- 1. Setup (once) -------------------------------------------------
let font_system = wgpu_gameui::shared_font_system();
let mut ui_renderer = UiRenderer::new(&device, &queue, surface_format, font_system);
let theme = Theme::default();

// --- 2. Per-frame state (owned by the app, persists across frames) ---
let mut ui_state = UiState::new();
let mut input = InputState::default();

// --- 3. Frame loop ---------------------------------------------------
// Fill `input` from your window events (mouse, keyboard, wheel, text), then:
let mut list = DrawList::with_font_system(wgpu_gameui::shared_font_system());

Frame::new(&mut ui_state, &mut input, &theme, &KeyboardNav)
    .dt(0.016) // frame delta for hover/press easing
    .run(&mut list, |ui| {
        if ui.text_button("Play", Some(120.0), None) {
            // start the game
        }
        let mut name = String::from("Player");
        ui.text_input(0, &mut name, "name…", Some(200.0));
    });

// Render the DrawList:
let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
ui_renderer.render(&device, &queue, &mut encoder, &view, (width, height), scale, &list);
queue.submit(Some(encoder.finish()));

// Clear per-frame input edges after all surfaces/layers are done:
input.end_frame();
```

Run the full interactive example (window + mouse + keyboard + dropdowns +
modals + scroll views + text inputs):

```
cargo run --example hello_ui
```

---

## Widget gallery

| Widget | Façade verb (`UiContext`) | Raw widget |
|---|---|---|
| Button | `text_button(label, w, h) -> bool` | `Button::draw(rect, ctx)` |
| Checkbox | `checkbox(label, checked) -> bool` | `Checkbox::draw(checked, label, rect, ctx)` |
| Slider | `slider(id, value, min, max, w) -> f32` | `Slider::draw(value, id, capture, rect, ctx)` |
| Radio group | `radio_group(options, selected) -> usize` | `RadioGroup::draw(selected, rect, ctx)` |
| Text input | `text_input(id, buf, placeholder, w) -> bool` | `TextInput::draw(id, ctx)` |
| Password input | `password_input(id, buf, placeholder, w)` | `TextInput::password()` |
| Text area | `text_area(id, buf, placeholder, w, rows)` | `TextInput::with_multiline(true)` |
| Number input | `number_input(id, val, min, max, step, dec, w)` | `NumberInput::draw(val, id, ti, rect, ctx)` |
| Dropdown | `dropdown(id, options, selected, w)` | `Dropdown::draw(ctx)` + `DropdownState` |
| Tree | `tree_node` / `tree_leaf` / `tree_pop` | `TreeNode::draw(rect, ctx)` |
| Tabs | `tabs(labels, active) -> Option<usize>` | `Tabs::draw(rect, list, style, input)` |
| Scroll view | `scroll_begin(w, h) -> Rect` / `scroll_end()` | `ScrollView::draw(state, list, style, input, closure)` |
| Enabled/disabled subtree | `enabled_scope(enabled, \|ui\|)` / `disabled_scope(\|ui\|)` | *(scope verb — gray-tint + input-disable a block)* |
| List | *(raw widget)* | `List::draw(rect, count, state, list, style, input, closure)` |
| Table | *(raw widget)* | `Table::draw(rect, rows, scroll, list, style, input)` |
| Panel | `panel(w, h)` | `Panel::draw_at(rect, list, style)` |
| Group | `group_begin(title, w, h) -> Rect` | `Group::draw(rect, list, style) -> Rect` |
| Separator | `separator()` | `Separator::draw(rect, list, style)` |
| Progress bar | `progress_bar(value, w)` | `ProgressBar::draw(rect, list, style)` |
| Banner | `banner(severity, message, w)` | `Banner::draw(rect, list, style)` |
| Color picker | `color_picker(id, hsva, w)` | `ColorPicker::draw(hsva, id, capture, rect, ctx)` |
| Drag handle | `drag_handle(id, w, h)` | `DragHandle::draw(rect, id, capture, ctx)` |
| Image button | `image_button_key(key, w, h)` | `ImageButton::draw(rect, list, style, input)` |
| Image | `image_box(sprite, w, h)` | `Image::draw(rect, list)` |
| Icon | *(draw primitive)* | `DrawList::icon` / `Icon` widget |
| Hit zone | `hit_zone(w, h)` / `hit_zone_at(rect)` | `HitZone::test(rect, input)` |
| Toast | *(state on `UiState::toasts`)* | `ToastStack::push` / `tick` / `draw` |
| Tooltip | *(state on `UiState::tooltips`)* | `TooltipLayer::hover_zone` / `tick` / `draw` |

> **List and Table** stay raw widgets (no façade verb) because their
> closure-based row/cell APIs don't fit the simple auto-advance verb model.

A headless render of the full widget set is checked into the test suite:

```
DISPLAY=:0 cargo test --test widget_gallery -- --ignored --nocapture
# writes test_output/widget_gallery.png
#      + test_output/widget_gallery.debug.{txt,json} (the layout dump)
```

---

## Architecture

### Three layers of API

```
┌──────────────────────────────────────────────────────┐
│  Frame::run (closure-scoped begin/end_frame bracket) │  ← easiest
├──────────────────────────────────────────────────────┤
│  UiContext verbs (text_button, slider, text_input…)  │  ← façade
├──────────────────────────────────────────────────────┤
│  Raw widgets (Button::draw, Slider::draw, …)         │  ← full control
│  + DrawList primitives (quad, rounded_rect, line, …)  │
└──────────────────────────────────────────────────────┘
```

- **`DrawList`** is the pure data layer: a CPU-side command list of quads,
  rounded rects, lines, circles, icons, nine-slices, and text blocks. It owns
  the transform stack, tint stack, clip stack, and (when used) debug scopes.
  No GPU state.
- **`DrawContext`** bundles a `&mut DrawList` with `&mut FocusState`,
  `&Theme`, `&InputState`, and screen dimensions, plus optional seams for
  animation (`with_animations`) and cursor requests (`with_cursor`). Raw
  widgets take this.
- **`UiContext`** is a thin borrow over a `DrawList` (in interactive mode,
  also `&InputState` + `&mut UiState` + `&Theme`) that adds Teardown-style
  verbs: `push`/`pop`, `translate`/`rotate`/`scale`, `align`/`center`,
  `color`/`color_filter`, `place_rect`, and the auto-advancing widget verbs.
- **`Frame`** is the closure-scoped entry point: it runs `begin_frame` /
  `end_frame` around your build closure so the pair can't be forgotten or
  mis-ordered, and `UiContext` is dropped (firing push/pop balance checks)
  before `end_frame`.

### Rendering pipeline

`UiRenderer` owns the wgpu pipelines, a dynamic sprite atlas, a nine-slice
metadata table, and the MSDF glyph atlas. `render(&DrawList)` tessellates and
encodes four sub-passes in order:

```
nine-slices → colored quads → icons → MSDF text
```

`render_layers(&LayerStack)` does the same for each layer in z-order, so a
popup's quads correctly overlap a base layer's text. The renderer never samples
its own framebuffer; `blur_backdrop` takes an app-provided scene texture for
frosted-glass effects.

### Image & atlas lifecycle

Images enter the sprite atlas three ways: `load_image_file` / `load_image_bytes`
(decode PNG/JPEG), `load_image_rgba8` (already-decoded pixels — skips the decode
round-trip, for apps that hold raw buffers like rendered notification icons), and
the out-of-band `load_sprite_rgba8`. The first three are keyed and cached, so
`has_image` / `image_size` / `unload_image` see them. The atlas grows on demand
(1024 → 2048 → 4096) and `SpriteId`s are stable indices that never shift.

`unload_image` frees the slot immediately (its pixels are reclaimed and the slot
recycled); shelf *fragmentation* left by churn is reclaimed by `compact_atlas`,
which a long-running app should call periodically (gate on `atlas_size()`
approaching a threshold) to keep the texture from climbing toward its 4096² cap.

### Caller-owned state

The library is immediate-mode, but interaction state persists across frames in
caller-owned structs. Construct them once, thread `&mut` into the relevant
widgets each frame:

| State struct | Owns | Used by |
|---|---|---|
| `UiState` | focus, drag capture, dropdowns, scroll, tree, animations, text inputs, toasts, tooltips | `UiContext` verbs, `Frame::run` |
| `ScrollState` | scroll offset + content extent | `ScrollView`, `List`, `Table` |
| `ListState` | scroll + selection set + keyboard cursor | `List` |
| `FocusState` | single focus owner + Tab ring | `TextInput`, `Button`, `Checkbox`, `Slider` |
| `DragCapture` | single drag owner (arbitration) | `Slider`, `ScrollView`, `DragHandle`, `ColorPicker` |
| `DropdownState` | which dropdown is open + geometry | `Dropdown` |
| `TreeState` | expanded set + selection + nav ring | `TreeNode` |
| `AnimationState` | in-flight color/scalar transitions | animated widgets via `DrawContext::with_animations` |
| `DragTracker` | press origin + click-vs-drag latch | writes `input.is_dragging` / `drag_delta` |
| `ClickTracker` | double-click + hold detection | writes `input.mouse_double_clicked` / `mouse_held` |
| `CursorState` | per-frame cursor icon accumulator | widgets request via `DrawContext::request_cursor` |

### Layout

A separate flexbox-style layout system (`layout` module) computes `Rect`s from
a tree of `VStack` / `HStack` / `Flow` / `Positioned` nodes. It does not touch
`DrawList` — you call `layout_screen(w, h)` once, then draw widgets at the
resulting rects. Supports `Fill`/`Fixed`/`Percent`/`Fit` sizing, weighted
flex-grow, `CrossAlign`, `MainAlign` (justify-content), wrap/flow, min/max
constraints, and stable node IDs for order-independent lookup.

### Theming & styling

`Theme` is a flat struct of colors + font + spacing. Every widget resolves
style through a `StyleResolver` (precedence: `StyleOverlay` → `Theme`), so a
subtree can be restyled without cloning the theme. `UiContext::set_style_color`
/ `set_style_scalar` push scoped overrides. Custom keys via
`StyleKey::custom(name)` + `Theme::register_style`. Hover/press color
transitions via `AnimationState` (eased, with a `0.0`-duration fast path that
is byte-identical to the instant path).

### Input & focus

The app fills an `InputState` struct (mouse position/buttons, scroll delta,
keyboard edges, text input, IME preedit, and a device-agnostic `NavInput` for
keyboard/gamepad navigation). The library never reads devices. Layer-aware
input dispatch (`LayerStack::input_for_base` / `input_for_layer`) sets
`mouse_consumed` so lower layers don't fire through popups/modals. Tab focus
cycles through registered `FocusId`s, scoped to the active layer.

---

## Debugging & layout inspection

Positioning bugs are invisible in an immediate-mode library: a widget drawn
off-screen, overflowing its container, clipped away, or misaligned by a fraction
of a pixel produces no error — just a wrong picture. `DebugReport` closes that
gap by describing a *finished* frame.

```rust
use wgpu_gameui::debug::DebugReport;

let report = DebugReport::measured_layers(&mut layers, screen);
println!("{}", report.to_text());
report.assert_clean();                       // panics with the full dump
report.write_to_dir("target/ui-debug")?;     // frame.txt + frame.json
```

`to_text()` renders an indented tree of world-space boxes followed by a
`PROBLEMS` section; `to_json()` gives the same thing machine-readably (hand
written — the crate has no serde dependency).

```
screen 0.0,0.0 800.0x600.0   nodes=41  problems=1  text=measured
────────────────────────────────────────────────────────────────
Sidebar [scope] 0.0,0.0 240.0x600.0  declared=0.0,0.0 200.0x600.0
  chrome#3 [chrome] 8.0,8.0 224.0x32.0
  "Settings" [text] 16.0,14.0 52.1x20.0
────────────────────────────────────────────────────────────────
PROBLEMS (1):
!! WARN  overflows_declared      Sidebar
   painted 0.0,0.0 240.0x600.0 but declared 0.0,0.0 200.0x600.0 — overshoots by 40.0px
   painted outside the rect this scope declared — content is larger than the
   box reserved for it, so it will collide with whatever sits next to it
```

Every finding carries its coordinates and a `hint()` explaining the likely
cause, so the dump is actionable without reading this crate's source.

### Declaring what a region was *supposed* to fill

The report works with no instrumentation at all, and an un-instrumented node is
**not** a less-checked node. Unscoped draws are named on a best-effort basis (a
text block by its own content, an icon by its atlas key, otherwise `chrome#12`)
and nested by geometric containment, and every geometric check still runs on
them: off-screen, clipped away, text overflowing its box, degenerate primitives
dropped. A button that landed 400px below its window is reported just as loudly
whether it is called `SaveButton` or `chrome#12`. The name only decides how
legible the resulting line is.

What instrumentation adds is **intent**, which no amount of geometry can
recover. A draw list records what was painted, never what layout assigned — so
"this is at x=100 and should have been at x=140" is unanswerable, because the
second number existed only in the layout code that already ran. Declaring the
rect supplies it:

```rust
ui.debug_scope("Sidebar", sidebar_rect, |ui| {
    ui.text("Settings");
    // …
});
```

Two checks then become possible. A scope can be caught **overflowing** its
declared rect — the only authoritative overflow check, since a region's painted
bounds are derived from its children and so can never overflow themselves — and
caught **painting nothing** into a box it reserved, which is otherwise
indistinguishable from never having been drawn.

(One asymmetry worth knowing: near-miss alignment analysis considers *named*
siblings only, because a widget's own sub-primitives — a panel's four border
quads — sit a pixel apart by construction and would otherwise flag every panel
in the frame.)

Scopes cost nothing when unused: a scope records the *buffer lengths* at push
and pop, and because every geometry buffer is append-only that already delimits
its span, so no primitive method or widget has to know scopes exist.

### Clips: boundary vs viewport

A clip that erases an element is a layout bug when the clip is a hard boundary
(a window, a panel) and the entire point when it's a viewport — a scroll view
hides the rows either side of its window on every frame it works correctly.
Only the pusher knows which it meant, so say so:

```rust
list.push_clip_viewport(inner);   // scroll view, dropdown list, scrolled field
list.push_clip(bounds);           // hard boundary — losing content here is a bug
```

Content a viewport removed is tagged `off-viewport` in the tree (so you can see
*why* it didn't render) and skipped by the lint. `ScrollView`, `Dropdown` and
multiline `TextInput` already do this.

### Lints

`LintConfig` tunes the checks; the defaults are chosen to be **silent on a
correct UI**, so anything reported is worth reading. `assert_clean()` guards on
`Error` only — unambiguous defects like off-screen, clipped away by a boundary,
reserved-a-box-and-drew-nothing, or a size that collapsed to zero. Warnings
cover the merely suspicious (crossing a screen edge, overlapping a sibling, an
edge half a pixel off its neighbours); `assert_clean_at(Severity::Warning)`
tightens the floor.

One check has no geometric evidence at all: `quad` and friends return early on
a non-positive size, so a width computed as `available - padding * 2` that went
negative leaves *nothing* in any buffer. `DrawList::dropped_degenerate()` counts
those rejections, which is the only way a collapsed element can be seen.

### Screenshots

For pixels rather than boxes, the headless capture path is exported — no async,
no boilerplate, and it handles the two steps hand-rolled copies get wrong (a
clear-only pass first, and the 256-byte row-alignment de-pad on readback):

```rust
let Some(mut gpu) = HeadlessGpu::new() else { return };   // no adapter: skip
let rgba = gpu.capture_layers(&layers, (800, 600), wgpu::Color::BLACK);
write_png("frame.png", &rgba, (800, 600))?;
```

---

## Features

| Feature | Default | Description |
|---|---|---|
| `bundled-font` | ✅ | Embed Noto Sans (regular/bold/italic) as the default sans-serif. Drop ~1.5 MB if you supply your own fonts. |
| `phosphor-icons` | ✅ | Embed the Phosphor (MIT) icon font and expose the `PhosphorIcon` enum + `Icon` widget. Drop ~0.5 MB if unused. |
| `headless` | ✅ | `HeadlessGpu` — offscreen device + renderer for screenshot capture in tests. Pulls in `pollster`. The dependency-free half (`capture_draw_list`, `write_png`) is always available. |
| `tracy` | ❌ | Emit `tracing` spans around the render path for Tracy profiling. |

---

## Testing & benchmarks

```bash
# Unit tests (791 tests, headless, no GPU)
cargo test --lib

# Widget gallery (headless GPU render → PNG + layout dump)
DISPLAY=:0 cargo test --test widget_gallery -- --ignored --nocapture
# writes test_output/widget_gallery.png, .debug.txt, .debug.json

# Benchmarks (CPU-only groups need no GPU; render groups do)
DISPLAY=:0 cargo bench --bench ui_stress
```

Benchmark groups: `drawlist_build`, `frame_render`, `render_text_only`,
`nine_slice`, `icons`, `primitives_build`, `primitives_render`, `layout_resolve`,
`text_shape`, `interactive_widgets`, `text_input_edit`, `scroll_view`,
`list_virtual`, `table`, `ui_context_frame`, `animation`.

---

## License

MIT OR Apache-2.0, at your option.
