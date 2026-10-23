# UI scenarios: scripted tests that drive a gameui app

**Status:** In progress
**Last updated:** 2026-10-23

## Implementation status

Tracking the gap between this design and what's on the main branch.

### Done

- (nothing yet)

### Outstanding

Phase 1, gameui, headless:

- [ ] `HostEvent`, a window-system-neutral input event, and `InputState::apply` that maps one onto the input fields (written, uncommitted: `crates/core/src/host_event.rs`, outside the feature, since every host uses it)
- [ ] `HostEvent::from_winit` behind a `winit` feature, used by `hello_ui.rs`, the designer and agent-ui in place of their own mappings (written, uncommitted: `host_event_winit.rs`; `hello_ui`, the designer and agent-ui use it)
- [ ] Locators: a scope path plus text, resolved against a `DebugReport` to exactly one node, or an error that lists what matched (written, uncommitted: `automation/locate.rs`)
- [ ] The scenario file format: JSON, `schemaVersion: 1`, unknown fields rejected, checked before anything runs (written, uncommitted: `automation/scenario.rs` and `keys.rs`, behind the `automation` feature)
- [ ] The `ScenarioApp` trait and the runner: steps turned into frames, checks retried for a frame budget, a stop at the first failure (written, uncommitted: `automation/run.rs`)
- [ ] Screenshots through `HeadlessGpu`, only when the run is given one (written, uncommitted)
- [ ] The report: `report.json`, `report.html`, and the failing frame's `DebugReport` as text (written, uncommitted: `automation/report.rs`)
- [ ] A small test app in gameui that runs every action (written, uncommitted: `automation/tests.rs`, see the note under Steps)

Phase 2, agent-ui, headless:

- [ ] Route the desktop's winit events through `HostEvent`, so shortcuts and the terminal's keys go through the same path a scenario uses (`crates/desktop/src/main.rs` `window_event`) (written, uncommitted: `App::host_event` and `key_event`; the terminal's key encoding takes gameui's `Key` and `HeldModifiers`)
- [ ] `App` implements `ScenarioApp`; a scenario's `start` picks one of `snapshot.rs`'s fixtures (written, uncommitted: `crates/desktop/src/scenario_app.rs`, names from `snapshot::Preview::NAMED`, which `--render-ui` now reads too; `--scenario PATH REPORT_DIR` runs one file)
- [ ] Debug scopes on the regions scenarios need to name (sidebar, session rows, composer, right dock tabs, dialogs) (written, uncommitted: see the note under Phasing)
- [ ] One test that runs every file in `crates/desktop/scenarios/` (written, uncommitted: `scenario_app_tests.rs`, with `session-menu.json` and `composer-typing.json`)
- [ ] Move `snapshot.rs`'s hand-written `Step` lists onto scenarios (not started: most of them inject data a scenario can't, see open question 4)

Phase 3, live (stdio):

- [ ] `--automation stdio` on the desktop: JSON lines in (events and queries), JSON lines out (acknowledgements, the tree, screenshots) (deferred: after phases 1 and 2)
- [ ] Explicit test ids on hit zones, option B (deferred: noted in `TODO.md`; scope paths and text first)

## Why this exists

gameui apps have no way to be tested the way a user uses them. agent-ui's tests call
functions on the app and check its state, and its screenshots (`crates/desktop/src/snapshot.rs`)
are hand-written sequences of input edits (`Step::RightPress(1)`, `Step::Key(|i| i.key_up = true)`)
in Rust. Nothing checks that clicking the second session row opens its menu, or that typing in
the composer and pressing Cmd+Enter sends, without someone opening the window.

gpuix (`~/Projects/gpuix`) solved this with `gpx test scenario.json`: a list of steps (click a
`testId`, press keys, expect text, take a screenshot) run against the real app, with a report.
This doc is the contract for doing the same in gameui. gameui is immediate mode and compiled,
so the scenario file is data that a runner in gameui reads, and the app plugs in through a
small trait.

## Goals

- A scenario is a JSON file of steps, close to gpuix's format, that any gameui app can run.
- Phase 1 and 2 run headless, inside `cargo test`: no window, no network, the same result on
  every run.
- Steps go through the same input path as a real window. A shortcut that only works in the
  winit handler is a bug this should find, not hide.
- Locators name things the way a person reads the screen: a region (a debug scope) and the
  text in it.
- Waiting counts frames, never milliseconds (agent-ui's rule: tests count work, not time).
- A failure says which step, what was expected, what was there, and leaves the frame's tree
  and a screenshot behind.

## Non-goals

- Pixel comparison against stored images. Screenshots are for people to look at.
- Scripting: no expressions, loops or evaluation. The file is a flat list of steps.
- OS-level input. Events are fed to the app, not to the window system.
- Mocking the network. Apps start scenarios from fixtures that need none (agent-ui's
  `snapshot.rs` already builds them).
- Proving a node is not covered by something else. `expectVisible` means painted inside the
  screen and not clipped away, as in gpuix.

## What exists today

- **Input is data.** `InputState` (`crates/core/src/lib.rs`) holds the pointer, buttons with
  press and release edges, wheel, typed text, the named keys (`key_left`, `key_tab`, …),
  `keys: KeyState` for every other key (`keys.rs`), and the modifiers. `end_frame` clears the
  edges. A host fills it from winit; `hello_ui.rs`, `designer/src/main.rs` and agent-ui's
  `main.rs` each have their own copy of that mapping.
- **The frame can be read back as a tree.** `DebugReport::measured_layers`
  (`crates/core/src/debug.rs`) turns a finished `LayerStack` into nested `DebugNode`s: scope
  names, painted bounds, declared rects, clips, and the text of every text node. Widgets
  declare their rects; apps name regions with `ui.debug_scope("Toolbar", …)`.
- **Hit-testing is retained between frames.** `InteractionScene` (`interaction.rs`) keeps each
  hit zone's `WidgetId` and rect from the last frame, which is why a press needs a frame
  before it to land on something.
- **Headless pixels.** `HeadlessGpu::capture_layers` and `write_png` (`render/capture.rs`).
- **`LayerStack::new()` needs no GPU**, so a run without screenshots needs no adapter.
- **agent-ui's headless frame** is `shell_ui::build_ui(&mut app, &mut layers, w, h)`
  followed by `app.input.end_frame()` (`snapshot.rs` `capture`), from `fixture()`.
- **agent-ui handles many keys outside `InputState`.** Cmd+Enter submits, Cmd+V pastes,
  Cmd+N/O/,/U open things, and the terminal takes raw keys, all inside the
  `WindowEvent::KeyboardInput` arm of `main.rs`. A scenario that only edits `InputState`
  would miss all of them, which is why phase 2 routes winit through `HostEvent` first.

## Design

### Events

```rust
pub enum HostEvent {
    PointerMoved { x: f32, y: f32 },
    Button { button: PointerButton, down: bool },
    Wheel { dx: f32, dy: f32 },
    Key { key: Key, down: bool, repeat: bool },
    Modifiers(Modifiers),          // the held state, as winit reports it
    Text(String),                  // committed text, as winit's `event.text`
}
```

`Key` and `Modifiers` are the existing types (`keys.rs`, `menubar/model.rs`). Shift, Alt and the
shortcut key arrive as `Modifiers`, the way `WindowEvent::ModifiersChanged` does; Alt's own
press and release edges come from `Modifiers` changes. `InputState::apply(&HostEvent)` holds
the mapping all three hosts copy today. A host with no shortcuts of its own calls only that;
agent-ui's `App::event` calls it and then does its own shortcuts.

A scenario's `press: "cmd-enter"` is parsed into `Modifiers { primary: true }`, `Key Enter`
down, `Key Enter` up, then the modifiers released. `cmd` and `ctrl` both mean `primary`, so one
file works on macOS and Linux.

### The app trait

```rust
pub trait ScenarioApp {
    /// Feed one input event, the way the window would.
    fn event(&mut self, event: &HostEvent);
    /// Build one frame into `layers` at `size`. `now` is the run's clock:
    /// the start plus one nominal frame per frame drawn.
    fn frame(&mut self, layers: &mut LayerStack, size: (f32, f32), now: Instant);
}
```

The runner owns the clock. Each frame sets `frame_dt` to `NOMINAL_FRAME_DT` and advances `now`
by the same, so an animation, a tooltip delay or a toast's lifetime takes the same number of
frames on every machine. App code that reads `Instant::now()` itself is not deterministic
under a scenario; phase 2 finds those spots in agent-ui and passes `now` down instead.

How a scenario's `start` becomes an app is the app's business: the runner takes a closure
`FnMut(&str) -> Result<A, String>`, and agent-ui maps names onto `snapshot.rs`'s fixtures.

### Locators

```json
{ "scope": ["Sidebar", "Sessions"], "text": "Fix the login" }
```

- `scope`: debug-scope names, outermost first. A node matches when its named ancestors include
  these in order, not necessarily directly nested, like a CSS descendant selector.
- `text`: optional. With it, the target is a text node under the scope whose text contains it
  (`"exact": true` for equality). Without it, the target is the innermost scope named by the
  path's last entry.
- A locator must match exactly one node. None, or more than one, is an error that lists what
  did match, with their scope paths and bounds, so a test is fixed by adding a scope rather
  than by guessing an index.
- `text` alone, with no `scope`, is allowed for short scenarios. It breaks when the copy
  changes, so the agent-ui scenarios anchor on a scope.

A pointer action aims at the centre of the target's bounds.

### Steps

| Action | Fields | What it does |
| --- | --- | --- |
| `click`, `rightClick`, `doubleClick` | locator | A frame with the pointer moved there, then the press and release as frames of their own |
| `hover` | locator | Move there and draw a frame |
| `drag` | `from`, `to` locators | Press, moves in steps past the drag threshold, release |
| `scroll` | locator, `dy`, `dx` | Wheel over the target |
| `press` | `keys`, e.g. `"tab enter"`, `"cmd-a"` | Key down and up for each, a frame between |
| `type` | `text` | `Text` events into whatever has focus |
| `waitFrames` | `frames` | Draw that many frames |
| `expectText` | locator, `text`, `exact` | The target's text, or all text under a scope, contains or equals |
| `expectVisible` | locator | Exactly one match, painted inside the screen and not clipped away |
| `expectAbsent` | locator | No match |
| `screenshot` | | Capture the current frame (needs a `HeadlessGpu`) |

Every step takes an optional `name` and `timeoutFrames`; the scenario has a default (120, two
seconds of animation at 60 Hz). Locating a target and every `expect` retry once per frame
until it holds or the budget runs out. Input is sent once, never replayed when a check fails.
The run stops at the first failure and marks the rest skipped.

> **2026-10-23 update:** a target is a nested `"target": { … }` object (`from`/`to` for a drag)
> rather than fields of the step itself, because serde can't reject unknown fields on a step
> whose locator is flattened into it. Screenshot files are `step-NN.png`, and
> `step-NN-failure.png` for a failing step. The test app is a unit test
> (`crates/core/src/automation/tests.rs`) with its scenarios inline, so the crate's own
> `cargo test` runs it without a feature flag; the crate's dev-dependency on itself turns on
> `automation` and `winit`.

```json
{
  "schemaVersion": 1,
  "name": "Session menu opens on right click",
  "start": "shell",
  "size": [1200, 800],
  "steps": [
    { "action": "rightClick", "target": { "scope": ["Sidebar", "Sessions"], "text": "Refactor the parser" } },
    { "action": "expectVisible", "target": { "scope": ["Context menu"], "text": "Close Session" } },
    { "action": "press", "keys": "escape" },
    { "action": "expectAbsent", "target": { "scope": ["Context menu"] } }
  ]
}
```

### Report

The run writes `report.json` and `report.html` (steps, status, frames each took, the error)
into an output directory, with screenshots named by step number. On failure it adds the
failing frame's `DebugReport::to_text()` and, when it can, a screenshot. In `cargo test` the
runner returns the report, and the test panics with the failing step and the tree, so the
output is readable without opening a file.

### Live driving (phase 3)

`--automation stdio` reads one JSON object per line on stdin: either a step from the table
above or a query (`tree`, `screenshot`, `locate`). It answers one line per request on stdout.
The window feeds the same `HostEvent`s through the event loop proxy. Only the process that
started the app can drive it, so there is no port and no token. Retries still count frames,
but a live frame takes real time, and the app talks to its real server with real side effects.

## Phasing

1. gameui: events, locators, format, runner, report, and a test app that exercises each action.
2. agent-ui: winit through `HostEvent`, `App` as a `ScenarioApp`, scopes, scenario files, and
   `snapshot.rs`'s step lists replaced.
3. Live stdio, and explicit test ids (option B), later.

> **2026-10-23 update (phase 2):** `shell_ui::build_ui` takes the frame's `Instant`, and the
> busy sweeps read `App::uptime` (the frame's time since start) instead of `started.elapsed()`,
> so a run draws them the same every time. The scenario's frame moves the fixture's wall clock
> (`sidebar.clock`, `transcript.clock`) by the run's seconds. The app's network channel is
> kept open and emptied each frame, so a click that sends something doesn't end in a
> "Network" error toast. Scopes: gameui's `GroupList` names each row (`GroupHeader "name"`,
> `GroupItem "title"`, `GroupMore "label"`); agent-ui adds `Sessions` (the sidebar's list),
> `Transcript`, `Composer` and `RightDock "Tasks"` / `"Subagents"`. Dialogs needed nothing:
> gameui's `Sheet` is already `Sheet "title"`. Key chords join with `-` (`cmd-a`), as
> `keys.rs` says. `run` takes the app already built rather than a closure over `start`;
> agent-ui's `scenario_app::play` builds it from the scenario's `start`.

## Open questions for review

1. **JSON parsing.** gameui has no serde dependency today (`debug.rs` writes its JSON by hand).
   Options: (a) `serde` + `serde_json` behind the new `automation` feature, which gives
   `deny_unknown_fields` and good errors for free; (b) a small hand-written parser, no new
   dependency. I'd pick (a), since the feature is off for apps that don't use it.

   > **2026-10-23, Bart:** (a), serde behind the `automation` feature.
2. **One winit mapping.** `hello_ui.rs`, the designer and agent-ui each map winit events by
   hand. Options: (a) a `winit` feature in gameui with `HostEvent::from_winit`, used by all
   three; (b) each host keeps its own mapping onto `HostEvent`. (a) removes the copies, but
   makes gameui depend on winit's version.

   > **2026-10-23, Bart:** (a), one shared mapping in gameui.
3. **Where scenario files live in agent-ui.** `crates/desktop/scenarios/` next to the crate,
   or `tests/scenarios/` at the root.

   > **2026-10-23, Bart:** `crates/desktop/scenarios/`.
4. **`snapshot.rs`'s step lists.** Only two previews' steps are input alone (`session-menu`'s
   right click, `ask-cards`' Escape). The others inject data between frames, after the first
   frame has laid things out: conversations arriving for the open menu, a task's output
   opening, a terminal's screen. Options: (a) move the two input-only ones onto scenarios and
   keep `Step::App` for the rest, so two mechanisms remain; (b) add an app-defined step to the
   format (`{ "action": "call", "name": "conversationsLoaded" }`, a `ScenarioApp::call` hook
   with a default that refuses), and turn every preview into a scenario file that ends in a
   screenshot, `--render-ui` running it; (c) leave the previews as they are, since they are
   for looking at, and keep scenarios for behaviour.

## References

- gpuix's runner: `~/Projects/gpuix/packages/native/src/bin/gpx/scenario.rs`, its format in
  the README's "Declarative UI scenarios" section, and `examples/scenarios/counter.json`.
- `crates/core/src/debug.rs` (`DebugReport`), `interaction.rs` (`InteractionScene`),
  `lib.rs` (`InputState`), `keys.rs`, `render/capture.rs`.
- agent-ui: `crates/desktop/src/snapshot.rs` (`fixture`, `Step`, `capture`), `main.rs`
  (`WindowEvent::KeyboardInput`).
