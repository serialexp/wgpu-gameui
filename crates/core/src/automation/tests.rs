use super::*;
use crate::debug::DebugReport;
use crate::layout::Rect;
use crate::{
    HeldModifiers, HostEvent, InputState, Key, KeyboardNav, LayerStack, Theme, UiContext, UiState,
};

/// A counter and a name field: the smallest app with a button to click, text
/// to read and a field to type into.
struct Counter {
    input: InputState,
    state: UiState,
    theme: Theme,
    count: u32,
    name: String,
    /// Events the app was given, to check what the runner sends.
    events: Vec<HostEvent>,
    /// The clocks it drew frames at.
    clocks: Vec<FrameClock>,
}

impl Counter {
    fn new() -> Self {
        Self {
            input: InputState::default(),
            state: UiState::new(),
            theme: Theme::default(),
            count: 0,
            name: String::new(),
            events: Vec::new(),
            clocks: Vec::new(),
        }
    }
}

impl ScenarioApp for Counter {
    fn event(&mut self, event: &HostEvent) {
        self.events.push(event.clone());
        self.input.apply(event);
    }

    fn frame(&mut self, layers: &mut LayerStack, _size: (f32, f32), clock: FrameClock) {
        self.clocks.push(clock);
        self.state
            .begin_frame(&mut self.input, &self.theme, clock.dt, &KeyboardNav);
        let mut ui =
            UiContext::interactive(layers.base_mut(), &self.input, &mut self.state, &self.theme);
        let count = &mut self.count;
        let name = &mut self.name;
        ui.debug_scope("Counter", |ui| {
            ui.text(&format!("Count: {count}"));
            if ui.text_button("Add", Some(100.0), Some(30.0)) {
                *count += 1;
            }
        });
        ui.debug_scope("Form", |ui| {
            ui.text_input(1, name, "Name", Some(200.0));
            ui.text(&format!("Hello {name}"));
        });
        drop(ui);
        self.state.end_frame(layers.layers());
        self.input.end_frame();
    }
}

fn scenario(steps: &str) -> Scenario {
    let json = format!(
        r#"{{ "schemaVersion": 1, "name": "test", "start": "counter", "size": [400, 300],
             "timeoutFrames": 10, "steps": [{steps}] }}"#
    );
    Scenario::from_json(&json).unwrap_or_else(|error| panic!("{error}\n{json}"))
}

fn play(steps: &str) -> (Report, Counter) {
    let mut app = Counter::new();
    let mut layers = LayerStack::new();
    let report = run(&scenario(steps), &mut app, &mut layers, None);
    (report, app)
}

fn drawn(app: &mut Counter) -> DebugReport {
    let mut layers = LayerStack::new();
    let clock = FrameClock {
        frame: 0,
        now: std::time::Instant::now(),
        dt: crate::NOMINAL_FRAME_DT,
    };
    app.frame(&mut layers, (400.0, 300.0), clock);
    DebugReport::measured_layers(&mut layers, Rect::new(0.0, 0.0, 400.0, 300.0))
}

fn locator(scope: &[&str], text: Option<&str>) -> Locator {
    Locator {
        scope: scope.iter().map(|s| s.to_string()).collect(),
        text: text.map(str::to_owned),
        exact: false,
    }
}

// ---- Keys ----

#[test]
fn chords_parse_with_the_shortcut_key_for_the_platform() {
    let chords = parse_keys("cmd-a shift-tab f5 minus Enter").unwrap();
    let primary = HeldModifiers {
        logo: cfg!(target_os = "macos"),
        control: !cfg!(target_os = "macos"),
        ..HeldModifiers::default()
    };
    assert_eq!(
        chords[0],
        Chord {
            modifiers: primary,
            key: Key::Char('a')
        }
    );
    assert_eq!(parse_keys("ctrl-a").unwrap()[0], chords[0]);
    assert!(chords[1].modifiers.shift && chords[1].key == Key::Tab);
    assert_eq!(chords[2].key, Key::F(5));
    assert_eq!(chords[3].key, Key::Char('-'));
    assert_eq!(chords[4].key, Key::Enter);
    assert!(parse_keys("control-u").unwrap()[0].modifiers.control);
}

#[test]
fn a_chord_types_its_character_unless_it_is_a_shortcut() {
    let text = |keys: &str| parse_keys(keys).unwrap()[0].text();
    assert_eq!(text("a"), Some("a".into()));
    assert_eq!(text("shift-a"), Some("A".into()));
    assert_eq!(text("space"), Some(" ".into()));
    assert_eq!(text("cmd-a"), None);
    assert_eq!(text("alt-a"), None);
    assert_eq!(text("enter"), None);
}

#[test]
fn unknown_keys_and_modifiers_are_errors() {
    assert!(
        parse_keys("hyper-a")
            .unwrap_err()
            .contains("not a modifier")
    );
    assert!(parse_keys("f25").unwrap_err().contains("not a key"));
    assert!(parse_keys("é").unwrap_err().contains("not a key"));
    assert!(parse_keys("  ").is_err());
}

// ---- The file ----

#[test]
fn a_scenario_parses_with_defaults() {
    let parsed = Scenario::from_json(
        r#"{ "schemaVersion": 1, "name": "n", "start": "s",
             "steps": [{ "action": "click", "name": "Add one", "target": { "scope": ["Counter", "Button"] } },
                       { "action": "screenshot", "timeoutFrames": 5 }] }"#,
    )
    .unwrap();
    assert_eq!(parsed.size, [1200.0, 800.0]);
    assert_eq!(parsed.timeout_frames, DEFAULT_TIMEOUT_FRAMES);
    assert_eq!(parsed.steps[0].name.as_deref(), Some("Add one"));
    assert_eq!(
        parsed.steps[0].action,
        Action::Click {
            target: locator(&["Counter", "Button"], None)
        }
    );
    assert_eq!(parsed.budget(&parsed.steps[0]), DEFAULT_TIMEOUT_FRAMES);
    assert_eq!(parsed.budget(&parsed.steps[1]), 5);
}

#[test]
fn bad_files_are_rejected_before_anything_runs() {
    let bad = |steps: &str, wanted: &str| {
        let json =
            format!(r#"{{ "schemaVersion": 1, "name": "n", "start": "s", "steps": [{steps}] }}"#);
        let error = Scenario::from_json(&json).expect_err(steps);
        assert!(error.contains(wanted), "{steps}: {error}");
    };
    bad(
        r#"{ "action": "click", "target": { "text": "a" }, "button": 2 }"#,
        "button",
    );
    bad(r#"{ "action": "tap", "target": { "text": "a" } }"#, "tap");
    bad(r#"{ "action": "click", "target": { "txt": "a" } }"#, "txt");
    bad(
        r#"{ "action": "click", "target": {} }"#,
        "needs a scope, text, or both",
    );
    bad(
        r#"{ "action": "click", "target": { "scope": [""] } }"#,
        "can't be empty",
    );
    bad(
        r#"{ "action": "press", "keys": "cmd-zz" }"#,
        "step 1 (press)",
    );
    bad(r#"{ "action": "waitFrames", "frames": 0 }"#, "frames 0");
    bad(r#"{ "action": "type", "text": "" }"#, "nothing to type");
    bad(
        r#"{ "action": "screenshot", "timeoutFrames": 0 }"#,
        "timeoutFrames 0",
    );
    bad("", "no steps");
    let version =
        Scenario::from_json(r#"{ "schemaVersion": 2, "name": "n", "start": "s", "steps": [] }"#);
    assert!(version.unwrap_err().contains("schemaVersion 2"));
    let field = Scenario::from_json(
        r#"{ "schemaVersion": 1, "name": "n", "start": "s", "steps": [], "app": "x" }"#,
    );
    assert!(field.unwrap_err().contains("app"));
}

// ---- Finding things ----

#[test]
fn a_scope_path_and_text_find_one_node() {
    let report = drawn(&mut Counter::new());
    let add = find_one(&report, &locator(&["Counter"], Some("Add"))).unwrap();
    assert_eq!(add.node.text.as_deref(), Some("Add"));
    assert!(
        add.path.starts_with("Counter > Button \"Add\""),
        "{}",
        add.path
    );
    assert!(add.visible.is_some());
    // A kind names a labelled scope, and the entries need not be adjacent.
    let button = find_one(&report, &locator(&["Counter", "Button"], None)).unwrap();
    assert_eq!(button.node.name, "Button \"Add\"");
    assert_eq!(text_of(&report, button.node), "Add");
    assert!(find_one(&report, &locator(&["Button \"Add\""], None)).is_ok());
}

#[test]
fn text_must_be_under_the_scopes_in_order() {
    let report = drawn(&mut Counter::new());
    assert!(find_all(&report, &locator(&["Form"], Some("Add"))).is_empty());
    assert!(find_all(&report, &locator(&["Button", "Counter"], Some("Add"))).is_empty());
    let error = find_one(&report, &locator(&["Form"], Some("Add"))).unwrap_err();
    assert!(
        error.contains("not under those scopes") && error.contains("Counter"),
        "{error}"
    );
}

#[test]
fn exact_text_must_be_the_whole_text() {
    let report = drawn(&mut Counter::new());
    let mut count = locator(&[], Some("Count"));
    assert!(find_one(&report, &count).is_ok());
    count.exact = true;
    assert!(find_all(&report, &count).is_empty());
    count.text = Some("Count: 0".into());
    assert!(find_one(&report, &count).is_ok());
}

#[test]
fn several_matches_are_an_error_that_lists_them() {
    let report = drawn(&mut Counter::new());
    let error = find_one(&report, &locator(&[], Some("o"))).unwrap_err();
    assert!(
        error.contains("matches for text containing \"o\""),
        "{error}"
    );
    assert!(
        error.contains("Counter >") && error.contains("Form >"),
        "{error}"
    );
}

#[test]
fn a_missing_scope_says_so() {
    let report = drawn(&mut Counter::new());
    let error = find_one(&report, &locator(&["Sidebar"], None)).unwrap_err();
    assert_eq!(error, "nothing matches scope \"Sidebar\"");
}

// ---- Running ----

#[test]
fn a_scenario_clicks_types_and_reads_the_screen() {
    let (report, app) = play(
        r#"{ "action": "expectText", "target": { "scope": ["Counter"] }, "text": "Count: 0 Add", "exact": true },
           { "action": "click", "target": { "scope": ["Counter", "Button"] } },
           { "action": "expectText", "target": { "scope": ["Counter"], "text": "Count" }, "text": "Count: 1" },
           { "action": "click", "target": { "scope": ["Form", "TextInput"] } },
           { "action": "type", "text": "Ada" },
           { "action": "expectText", "target": { "scope": ["Form"], "text": "Hello" }, "text": "Hello Ada", "exact": true },
           { "action": "press", "keys": "cmd-a backspace" },
           { "action": "expectAbsent", "target": { "text": "Ada" } },
           { "action": "expectVisible", "target": { "scope": ["Form"], "text": "Name", "exact": true } }"#,
    );
    report.assert_passed();
    assert_eq!((app.count, app.name.as_str()), (1, ""));
    assert!(
        report
            .steps
            .iter()
            .all(|step| step.status == Status::Passed)
    );
}

#[test]
fn every_step_takes_the_frames_it_should() {
    let (report, _) = play(
        r#"{ "action": "expectVisible", "target": { "scope": ["Counter"] } },
           { "action": "click", "target": { "scope": ["Counter", "Button"] } },
           { "action": "doubleClick", "target": { "scope": ["Counter", "Button"] } },
           { "action": "rightClick", "target": { "scope": ["Counter", "Button"] } },
           { "action": "hover", "target": { "scope": ["Form"] } },
           { "action": "scroll", "target": { "scope": ["Form"] }, "dy": -2 },
           { "action": "drag", "from": { "scope": ["Counter"] }, "to": { "scope": ["Form"] } },
           { "action": "press", "keys": "tab shift-tab" },
           { "action": "type", "text": "hi\n" },
           { "action": "waitFrames", "frames": 4 }"#,
    );
    report.assert_passed();
    let frames: Vec<u64> = report.steps.iter().map(|step| step.frames).collect();
    assert_eq!(frames, [0, 3, 5, 3, 1, 2, 11, 2, 3, 4]);
    // The first frame, before any step, plus every step's.
    assert_eq!(report.frames, 1 + frames.iter().sum::<u64>());
}

#[test]
fn the_clock_moves_one_nominal_frame_per_frame() {
    let (_, app) = play(r#"{ "action": "waitFrames", "frames": 3 }"#);
    let start = app.clocks[0].now;
    for (n, clock) in app.clocks.iter().enumerate() {
        assert_eq!(clock.frame, n as u64);
        assert_eq!(clock.dt, crate::NOMINAL_FRAME_DT);
        let expected =
            std::time::Duration::from_secs_f64(n as f64 * f64::from(crate::NOMINAL_FRAME_DT));
        assert_eq!(clock.now - start, expected);
    }
    assert_eq!(app.clocks.len(), 4);
}

#[test]
fn a_click_is_a_move_then_a_press_and_a_release() {
    let (report, app) =
        play(r#"{ "action": "click", "target": { "scope": ["Counter", "Button"] } }"#);
    report.assert_passed();
    let HostEvent::PointerMoved { x, y } = app.events[0] else {
        panic!("{:?}", app.events)
    };
    assert!(x > 0.0 && y > 0.0);
    assert_eq!(
        app.events[1..],
        [
            HostEvent::Button {
                button: crate::PointerButton::Primary,
                down: true
            },
            HostEvent::Button {
                button: crate::PointerButton::Primary,
                down: false
            },
        ]
    );
}

#[test]
fn a_shortcut_holds_its_modifiers_around_the_key() {
    let (_, app) = play(r#"{ "action": "press", "keys": "shift-a" }"#);
    let shift = HeldModifiers {
        shift: true,
        ..HeldModifiers::default()
    };
    assert_eq!(
        app.events,
        [
            HostEvent::Modifiers(shift),
            HostEvent::Key {
                key: Key::Char('a'),
                down: true,
                repeat: false,
                text: Some("A".into())
            },
            HostEvent::Key {
                key: Key::Char('a'),
                down: false,
                repeat: false,
                text: None
            },
            HostEvent::Modifiers(HeldModifiers::default()),
        ]
    );
}

#[test]
fn a_failed_check_retries_for_its_budget_then_stops_the_run() {
    let (report, app) = play(
        r#"{ "action": "expectText", "target": { "scope": ["Counter"] }, "text": "Count: 5", "timeoutFrames": 3 },
           { "action": "click", "target": { "scope": ["Counter", "Button"] } }"#,
    );
    assert!(!report.passed());
    let failed = &report.steps[0];
    assert_eq!((failed.status, failed.frames), (Status::Failed, 3));
    let error = failed.error.as_deref().unwrap();
    assert!(error.starts_with("after 3 frames, Counter says"), "{error}");
    assert_eq!(report.steps[1].status, Status::Skipped);
    assert!(app.events.is_empty(), "a skipped step sends nothing");
    assert!(
        report
            .error
            .as_deref()
            .unwrap()
            .starts_with("step 1 (expectText)")
    );
    assert!(report.failure_tree.as_deref().unwrap().contains("Counter"));
}

#[test]
fn a_missing_target_fails_without_sending_input() {
    let (report, app) = play(r#"{ "action": "click", "target": { "scope": ["Sidebar"] } }"#);
    let error = report.steps[0].error.as_deref().unwrap();
    assert_eq!(error, "after 10 frames, nothing matches scope \"Sidebar\"");
    assert!(app.events.is_empty());
}

#[test]
fn a_screenshot_needs_a_gpu() {
    let (report, _) = play(r#"{ "action": "screenshot", "name": "The counter" }"#);
    let step = &report.steps[0];
    assert_eq!(step.name.as_deref(), Some("The counter"));
    assert_eq!(
        step.error.as_deref(),
        Some("this run has no GPU to take a screenshot with")
    );
}

#[test]
fn the_report_is_written_as_json_and_html() {
    let (report, _) = play(
        r#"{ "action": "expectText", "target": { "text": "Count" }, "text": "<9>", "timeoutFrames": 1 }"#,
    );
    let dir = std::env::temp_dir().join(format!("gameui-scenario-report-{}", std::process::id()));
    report.write(&dir).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("report.json")).unwrap()).unwrap();
    assert_eq!(json["schemaVersion"], 1);
    assert_eq!(json["status"], "failed");
    assert_eq!(json["steps"][0]["action"], "expectText");
    assert_eq!(json["steps"][0]["frames"], 1);
    let html = std::fs::read_to_string(dir.join("report.html")).unwrap();
    assert!(html.contains("&lt;9&gt;") && !html.contains("<9>"));
    assert!(
        std::fs::read_to_string(dir.join("failure.txt"))
            .unwrap()
            .contains("Counter")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(feature = "headless")]
#[test]
fn a_run_with_a_gpu_takes_its_screenshots() {
    let Some(mut gpu) = crate::HeadlessGpu::new() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut app = Counter::new();
    let mut layers = gpu.layer_stack();
    let scenario = scenario(
        r#"{ "action": "screenshot" },
           { "action": "expectText", "target": { "text": "Count" }, "text": "Count: 9", "timeoutFrames": 1 }"#,
    );
    let report = run(&scenario, &mut app, &mut layers, Some(&mut gpu));
    assert_eq!(report.steps[0].screenshot.as_deref(), Some("step-01.png"));
    assert_eq!(
        report.steps[1].screenshot.as_deref(),
        Some("step-02-failure.png")
    );
    let files: Vec<&str> = report
        .screenshots
        .iter()
        .map(|shot| shot.file.as_str())
        .collect();
    assert_eq!(files, ["step-01.png", "step-02-failure.png"]);
    let shot = &report.screenshots[0];
    assert_eq!(shot.size, (400, 300));
    assert_eq!(shot.pixels.len(), 400 * 300 * 4);
    // Something was drawn over the backdrop.
    assert!(shot.pixels.chunks(4).any(|px| px != [0, 0, 0, 255]));
}
