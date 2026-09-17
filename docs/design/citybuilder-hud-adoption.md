# Citybuilder HUD widget adoption — Design

Status: draft, not yet implemented
Owner: Bart
Last updated: 2026-09-16

Cross-repo note: the design lives here because this is where the design-doc
convention and the compared-against API both live, but **the implementation
lands entirely in the citybuilder repo** (`~/Projects/citybuilder`, mostly
`client/src/hud/`). Nothing in this plan requires changes to wgpu-gameui.

## Implementation status

Tracking the gap between this design and citybuilder's main branch. A phase
is done when the swap has landed, `cargo build` in `client/` is warning-clean,
and the affected panel has been eyeballed in the running client (resize the
window, scroll the sidebar, hover things).

### Done

_(nothing yet)_

### Outstanding

- [ ] **Phase 1 — layout primitives.** Replace the two private `FixedNode`
      copies (`client/src/hud/mod.rs:968`, `client/src/hud/resource_panel.rs:201`)
      with `wgpu_gameui::layout::Leaf`, and replace the hand-rolled anchor math
      in `agent_info_panel.rs:163-164` (`panel_y = screen_height - h - 10`) with
      `Anchor::BottomLeft { offset: (10.0, -10.0) }` + `Positioned`. Pure
      geometry, no rendering change expected.
- [ ] **Phase 2 — ScrollView in the command sidebar.** Replace the hand-rolled
      scroll region in `command_sidebar.rs` (content-height pass ~L318-344,
      wheel/mouse-in-rect check + clamp ~L346-358, hand-drawn scrollbar quad
      ~L496-511) with
      `ScrollView::new(build_rect).vertical_only().draw(...)`, reusing the
      existing caller-owned `self.scroll: ScrollState`. Also migrate the
      resource panel's `on_scroll` + bare `ScrollState` the same way.
      **Keep** the per-row `screen_y` visibility gating (see
      [Clip discrepancy](#clip-discrepancy-a-known-bug-to-verify)).
- [ ] **Phase 3 — verify the clip-trim bug visually.** Before/while doing
      Phase 2, run the client with a long build list and check whether
      partially visible rows bleed outside the scroll viewport (predicted by
      the render path, contradicted by a code comment — details below). Record
      the finding here; if it bleeds, note that Phase 4 is the real fix and
      draw-side culling must not be removed until then.
- [ ] **Phase 4 — atlas consolidation onto `UiRenderer` (optional, decide
      before starting).** Replace citybuilder's `NineSliceAtlas` +
      `IconAtlas` + `QuadRenderer` + `TextRenderer` HUD pipeline with gameui's
      `UiRenderer`: `load_image_rgba8` for the four UI PNGs +
      `register_nine_slice(name, sprite, border)`; tool icons via
      `load_image_file` (the `DrawList::icon(path)` string keys already carry
      asset-relative paths — no key translation needed); gameui's text pass.
      Deletes ~400 lines of atlas code and the field-by-field vertex
      conversion, and gets shader-side clipping (the per-row input gating then
      remains only as an optimization, not a correctness need).
- [ ] **Defer — badge/chip swaps until a release ships them.** The validity
      strip and material swatch chips stay plain quads for now: `badge`/`chip`
      are on this repo's main but **not** in the 0.4.0 crates.io release
      citybuilder pins (`menubar`, `toolbar`, `dock_panel`, `app_shell` are
      also main-only). Revisit when 0.5 lands.
- [ ] **Open question — Phase 4 scope.** Land Phases 1–2 first and treat
      Phase 4 as a follow-up, or do the renderer swap in the same effort? It
      is the only phase that touches the render path rather than draw calls.
- [ ] **Open question — `scroll_consumed` plumbing.** `ScrollView` consumes
      wheel events via `input.scroll_consumed` (0.4 behavior:
      `scroll_view.rs:226-239`), so `RightPanelOutput.scroll_consumed` can be
      derived from the input snapshot instead of a sidebar-computed flag.
      Decide whether to keep the output field (fewer surprises at the
      world-input layer) or drop it and read `input.scroll_consumed` directly.
- [ ] **Open question — second-consumer widgets.** If a second project needs a
      build-palette / cost-row / swatch-picker, revisit promoting them into
      gameui widgets. Until then they stay citybuilder-side composition
      (Bart: no widget split for now).

## Why this exists

Citybuilder's HUD (`client/src/hud/`, ~5,000 lines across 17 files) already
renders through wgpu-gameui 0.4: every panel draws into one shared `DrawList`
per frame using gameui `Panel`/`Button`/`Table`/`Tabs`/`ProgressBar`/
`TooltipLayer` and the `layout` module. But parts of the HUD predate or
bypass pieces the library ships, and each bypass has a standing cost:

- **Scroll:** the command sidebar hand-rolls ~100 lines of content-height
  estimation, a mouse-in-rect wheel check, offset clamping, and a manual
  scrollbar quad — all of which `ScrollView` (shipped since ≤0.4) provides,
  with a scrollbar, wheel-consumption semantics, and thumb dragging the
  hand-rolled version lacks.
- **Layout leaves:** two private `FixedNode` structs duplicate
  `layout::Leaf`; one panel duplicates `Anchor::BottomLeft` by hand. Divergent
  copies of layout math are how DPI/re-theming bugs get introduced in one
  place and fixed in another.
- **Atlases:** citybuilder maintains its own `NineSliceAtlas` and
  `IconAtlas` (plus per-frame vertex conversion that **drops gameui's clip
  fields**) to serve string-keyed nine-slice/icon draws that gameui's own
  `UiRenderer` already resolves through its built-in, growing atlas.
- **Honesty of comments:** `command_sidebar.rs:360-367` claims the clip stack
  provides the visual trim at scroll edges, but the render path discards clip
  data (`hud/mod.rs:649-662`: "Clip data is dropped; QuadRenderer doesn't
  support clipping"). Both statements cannot be true. Either the trim doesn't
  happen (partially visible rows bleed over neighboring UI) or something
  undocumented masks it — Phase 3 verifies which.

This doc is the contract for closing those gaps **without** adding new
widgets to wgpu-gameui and **without** rewriting the HUD: same immediate-mode
draw calls, same caller-owned state, same single-`DrawList` frame structure.

## Current state (verified on the tree)

Dependency: `wgpu-gameui = "0.4"` from crates.io (`client/Cargo.toml:25`).
Citybuilder's own `crates/gameui/` directory is **empty** — the migration to
the published crate already happened; only the name lingers.

**Already properly adopted** (no work needed — earlier hand-roll suspicions
corrected on inspection):

- `Table`/`TableColumn`/`TableCell` — all five management panels
  (`hud/management/*.rs`)
- `Tabs` — agent info panel (`agent_info_panel.rs:212`)
- `ProgressBar::from_u8(..).draw_labeled(..)` + `TooltipLayer` — needs bars
  (`agent_info_panel.rs:497-516`)
- `Panel::draw_at` / `Panel::draw_nine_slice`, `Button::draw_at` /
  `Button::draw_nine_slice`, `TextBlock`, `tooltip_layer`
- Anchors at 9 of 11 placement sites: `Anchor::TopRight` (right dock),
  `TopLeft` (resource panel), `TopCenter` (menu bar), `Center` (five
  management dialogs), `BottomLeft` (selected-item card, multi-agent panel,
  field actions) — via `Positioned::new(..).layout_screen(..)`

**Hand-rolled, targeted by this design:**

| Site | Today | Becomes |
|---|---|---|
| `hud/mod.rs:968-991`, `resource_panel.rs:201-224` | private `FixedNode` impls | `layout::Leaf` |
| `agent_info_panel.rs:163-164` | manual bottom-left math | `Anchor::BottomLeft` |
| `command_sidebar.rs` scroll region | manual wheel/clamp/scrollbar/culling | `ScrollView` + kept row culling |
| `resource_panel.rs` `on_scroll` | bare `ScrollState` + manual wheel | `ScrollView` |
| `hud/nine_slice_atlas.rs`, `hud/icon_atlas.rs`, vertex conversion | custom atlases + render passes | `UiRenderer` (Phase 4, optional) |
| validity strip, swatch chips, cost rows | raw `list.quad`/`list.text` | unchanged for now (badge/chip not in 0.4; see Deferred) |

## <a name="clip-discrepancy"></a>Clip discrepancy — a known bug to verify

Two citybuilder comments disagree, and the design treats the render path as
authoritative until proven otherwise:

- `command_sidebar.rs:360-367`: "The clip stack handles the *visual* trim at
  the scroll edges so partially visible rows don't bleed into other UI."
- `hud/mod.rs:649-662`: the `DrawList` → `QuadVertex` conversion is
  field-by-field and "Clip data is dropped; QuadRenderer doesn't support
  clipping."

If clip data never reaches the GPU, the trim the first comment promises does
not exist, and any row straddling a scroll edge should paint (and label)
outside the viewport. The per-row `screen_y` checks gate *input* for fully
off-screen rows and cull *drawing* for fully off-screen rows, but a row half
inside the viewport passes both checks and would straddle visibly.

Prediction: the bleed is real and merely rare (needs a row boundary to land
inside the viewport). Phase 2/3 verifies visually. Note that `ScrollView`
alone does **not** fix this while citybuilder's renderer drops clip fields —
it changes *who manages the offset*, not whether the GPU honors the clip.
The actual fix is Phase 4 (gameui's `UiRenderer` applies clip in its
shaders). Until then, keep the row culling and, if the bleed shows, extend
the `screen_y` check to clamp row heights at the viewport edges or accept
the cosmetic artifact explicitly.

Interaction note (why input gating must stay even after ScrollView): the
sidebar's buttons go through the legacy immediate path
(`Button::draw_at`/`draw_nine_slice`), whose hit test is
`rect.contains(mouse) && !mouse_consumed` — `button.rs:342`, `button.rs:458`
— with no clip awareness. (Only the retained `InteractionScene` path is
clip-aware, `interaction.rs:91-105`, and citybuilder doesn't use it.) So
off-viewport rows would still respond to clicks if the culling were removed.

## Phasing

**Phase 1 — layout primitives** (mechanical, safe, no render-path change):
swap `FixedNode` → `Leaf` and the one manual anchor. Verify by resizing the
window with the agent panel open; the panel must stay pinned 10px from the
bottom-left.

**Phase 2 — ScrollView** (the main swap, draw-calls-only):

- `ScrollState.content_size` must be set **before** `ScrollView::draw` (it
  cannot know content height until measured — documented on
  `ScrollView::draw`). The sidebar already computes `content_height` from
  category/tool/cost row counts; that loop moves above the draw call, and the
  draw closure walks the same rows once (no separate measure pass needed —
  the row heights are formulaic: 30px headers, 26/28px tool rows, per-cost
  lines).
- The draw closure receives the clip/translate already applied; the body is
  the existing category/tool/cost drawing code unchanged, minus the manual
  offset arithmetic (`scroll_area_top + virtual_y - self.scroll.offset[1]`
  collapses to `virtual_y`).
- `content_size` must account for the scrollbar reserving
  `bar_thickness` (default 13px; the old hand-rolled bar was 4px — a visible
  style change, which is fine: it's gameui's look now).
- Keep the `screen_y`-style visibility gating for input per the section
  above. Inside the closure, `ScrollBegin::inner` gives the on-screen
  viewport rect to test against (row screen y = content y − nothing: the
  closure draws in world-space pre-offset coords, so the check becomes
  `inner.contains(row_rect)` with row_rect shifted by the *current* offset —
  recompute via `state.offset` as today).

**Phase 3 — verify the trim bug** (paired with Phase 2, recorded here).

**Phase 4 — `UiRenderer` consolidation** (optional; decide via the open
question before starting). Load the four nine-slice PNGs with their existing
border insets (`panel1` 10/10/5/5, `panel2` 11/11/11/11, `button` 9/9/9/9,
`track` 4/4/4/4 — from `hud/mod.rs:166-198`) via `load_image_rgba8` +
`register_nine_slice`; load tool icons by their asset-relative paths; delete
`icon_atlas.rs`, `nine_slice_atlas.rs`, the `QuadVertex` conversion, and the
four-pass `render()` in favor of gameui's `render`/`render_layers`. Tooltips
and any future popup/menu layers then work without new plumbing.

**Deferred:** badge/chip swaps (needs a release containing them), and any
widget promotion into gameui (needs a second consumer).

## Testing / verification

- `cd client && cargo build` warning-clean after each phase.
- Manual eyeball pass per phase (citybuilder has no UI gallery test):
  1. Right dock with a long build list (unlock research categories to grow
     it): scroll to top/bottom, thumb-drag, wheel over the sidebar while the
     cursor is over the world → camera must not zoom (scroll consumed), and
     vice versa.
  2. Agent panel pinned bottom-left across resizes.
  3. Selected-item card, multi-agent panel, management dialogs unchanged.
  4. Phase 4: nine-slice panel/button corners pixel-identical at current
     sizes; tool icons render without key translation.
- Screenshots before/after each phase into the PR description.

## References

- Citybuilder: `client/Cargo.toml`, `client/src/hud/{mod,command_sidebar,
  right_panel,resource_panel,agent_info_panel,menu_bar,floor_switcher}.rs`,
  `client/src/hud/management/*.rs`, `client/src/hud/{icon_atlas,
  nine_slice_atlas}.rs`, `client/src/command/{types,mod}.rs`,
  `client/src/render/world/passes/build_overlays.rs` (world-space ghost
  preview — out of scope).
- This repo: `src/layout.rs` (`Anchor` :144, `SizeSpec` :241, `Leaf` :1280),
  `src/widgets/scroll_view.rs` (wheel consumption :226-239, `draw` contract
  :211-224), `src/widgets/button.rs` (legacy hit tests :342/:458),
  `src/interaction.rs:91-105` (clip-aware retained path),
  `src/render/ui_renderer.rs` (`load_image_*` :801-877,
  `register_nine_slice` :947, shader nine-slice :2038+).
- Prior art for the register/structure: `docs/design/menubar.md`,
  `docs/design/contextual-widget-lifecycle.md`.
