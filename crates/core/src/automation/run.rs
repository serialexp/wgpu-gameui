//! Playing a [`Scenario`] against an app, a frame at a time.

use std::time::{Duration, Instant};

use crate::debug::DebugReport;
use crate::layout::Rect;
use crate::{HostEvent, Key, LayerStack, NOMINAL_FRAME_DT, PointerButton};

use super::keys::{Chord, parse_keys};
use super::locate::{find_all, find_one, text_of};
use super::report::{Report, Screenshot, Status, StepReport};
use super::{Action, Locator, SCHEMA_VERSION, Scenario};

#[cfg(feature = "headless")]
use crate::HeadlessGpu;

/// How many pointer moves a drag takes from its press to its release.
const DRAG_MOVES: u32 = 8;

/// An app a scenario can drive.
pub trait ScenarioApp {
    /// Take one input event, the way the window would deliver it.
    fn event(&mut self, event: &HostEvent);

    /// Build one frame into `layers` at `size` (logical pixels), then end the
    /// input frame ([`InputState::end_frame`](crate::InputState::end_frame)),
    /// as the window's redraw does. Read the time from `clock`, never from
    /// the system: that is what makes a run the same on every machine.
    fn frame(&mut self, layers: &mut LayerStack, size: (f32, f32), clock: FrameClock);

    /// The colour the window clears to, for screenshots.
    fn backdrop(&self) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// The run's time for one frame. Every frame is one nominal frame
/// ([`NOMINAL_FRAME_DT`]) after the last.
#[derive(Clone, Copy, Debug)]
pub struct FrameClock {
    /// Frames drawn before this one.
    pub frame: u64,
    /// When this frame is.
    pub now: Instant,
    /// Seconds since the last frame.
    pub dt: f32,
}

impl FrameClock {
    /// Seconds since the run began, for APIs that take a plain time
    /// ([`ClickTracker::update`](crate::ClickTracker::update)).
    pub fn seconds(&self) -> f64 {
        self.frame as f64 * f64::from(self.dt)
    }
}

/// Play `scenario` against `app`, drawing into `layers`. With `gpu`, the run
/// can take screenshots (the `screenshot` step, and one when a step fails);
/// make `layers` with [`HeadlessGpu::layer_stack`] then, so text is measured
/// with the fonts it is drawn with.
pub fn run<A: ScenarioApp>(
    scenario: &Scenario,
    app: &mut A,
    layers: &mut LayerStack,
    #[cfg(feature = "headless")] gpu: Option<&mut HeadlessGpu>,
) -> Report {
    let [width, height] = scenario.size;
    let mut runner = Runner {
        app,
        layers,
        #[cfg(feature = "headless")]
        gpu,
        size: (width, height),
        start: Instant::now(),
        frames: 0,
        report: None,
        pointer: None,
        screenshots: Vec::new(),
    };
    // The first frame lays out what the first step aims at, and registers
    // the hit zones its press lands on.
    runner.draw();
    let mut steps = Vec::with_capacity(scenario.steps.len());
    let mut error = None;
    for (index, step) in scenario.steps.iter().enumerate() {
        let mut report = StepReport {
            index: index + 1,
            name: step.name.clone(),
            action: step.action.label(),
            status: Status::Skipped,
            frames: 0,
            error: None,
            screenshot: None,
        };
        if error.is_none() {
            let before = runner.frames;
            let outcome = runner.step(&step.action, scenario.budget(step), index + 1);
            report.frames = runner.frames - before;
            match outcome {
                Ok(shot) => {
                    report.status = Status::Passed;
                    report.screenshot = shot;
                }
                Err(message) => {
                    report.status = Status::Failed;
                    report.screenshot =
                        runner.capture(format!("step-{:02}-failure.png", index + 1));
                    error = Some(format!(
                        "step {} ({}): {message}",
                        index + 1,
                        step.name.as_deref().unwrap_or(step.action.label())
                    ));
                    report.error = Some(message);
                }
            }
        }
        steps.push(report);
    }
    let failure_tree = error
        .as_ref()
        .and(runner.report.as_ref())
        .map(DebugReport::to_text);
    Report {
        schema_version: SCHEMA_VERSION,
        name: scenario.name.clone(),
        status: if error.is_some() {
            Status::Failed
        } else {
            Status::Passed
        },
        frames: runner.frames,
        error,
        steps,
        failure_tree,
        screenshots: std::mem::take(&mut runner.screenshots),
    }
}

struct Runner<'a, A> {
    app: &'a mut A,
    layers: &'a mut LayerStack,
    #[cfg(feature = "headless")]
    gpu: Option<&'a mut HeadlessGpu>,
    size: (f32, f32),
    start: Instant,
    frames: u64,
    /// The last frame's tree.
    report: Option<DebugReport>,
    /// Where the last pointer event put the pointer.
    pointer: Option<(f32, f32)>,
    screenshots: Vec<Screenshot>,
}

impl<A: ScenarioApp> Runner<'_, A> {
    fn draw(&mut self) {
        let dt = NOMINAL_FRAME_DT;
        let clock = FrameClock {
            frame: self.frames,
            now: self.start + Duration::from_secs_f64(self.frames as f64 * f64::from(dt)),
            dt,
        };
        self.layers.clear();
        self.app.frame(self.layers, self.size, clock);
        self.frames += 1;
        let screen = Rect::new(0.0, 0.0, self.size.0, self.size.1);
        self.report = Some(DebugReport::measured_layers(self.layers, screen));
    }

    fn send(&mut self, event: HostEvent) {
        if let HostEvent::PointerMoved { x, y } = event {
            self.pointer = Some((x, y));
        }
        self.app.event(&event);
    }

    /// Send `events`, then draw a frame.
    fn frame_with(&mut self, events: impl IntoIterator<Item = HostEvent>) {
        for event in events {
            self.send(event);
        }
        self.draw();
    }

    /// Draw frames until `check` holds on the latest one, `budget` frames at
    /// most. The frame already drawn is checked first, for free.
    fn until<T>(
        &mut self,
        budget: u32,
        mut check: impl FnMut(&DebugReport) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut drawn = 0;
        loop {
            let report = self
                .report
                .as_ref()
                .expect("the run draws a frame before any step");
            let failure = match check(report) {
                Ok(value) => return Ok(value),
                Err(failure) => failure,
            };
            if drawn == budget {
                return Err(format!("after {budget} frames, {failure}"));
            }
            self.draw();
            drawn += 1;
        }
    }

    /// The centre of the one visible match for `target`.
    fn aim(&mut self, target: &Locator, budget: u32) -> Result<(f32, f32), String> {
        self.until(budget, |report| {
            let found = find_one(report, target)?;
            let shown = found
                .visible
                .ok_or_else(|| format!("{} is not on screen", found.path))?;
            Ok((shown.x + shown.width / 2.0, shown.y + shown.height / 2.0))
        })
    }

    fn click(&mut self, (x, y): (f32, f32), button: PointerButton, times: u32) {
        self.frame_with([HostEvent::PointerMoved { x, y }]);
        for _ in 0..times {
            self.frame_with([HostEvent::Button { button, down: true }]);
            self.frame_with([HostEvent::Button {
                button,
                down: false,
            }]);
        }
    }

    fn chord(&mut self, chord: Chord) {
        let held = !chord.modifiers.is_empty();
        let mut events = Vec::with_capacity(4);
        if held {
            events.push(HostEvent::Modifiers(chord.modifiers));
        }
        events.push(HostEvent::Key {
            key: chord.key,
            down: true,
            repeat: false,
            text: chord.text(),
        });
        events.push(HostEvent::Key {
            key: chord.key,
            down: false,
            repeat: false,
            text: None,
        });
        if held {
            events.push(HostEvent::Modifiers(Default::default()));
        }
        self.frame_with(events);
    }

    fn typed(&mut self, character: char) {
        let (key, text) = match character {
            '\n' => (Key::Enter, None),
            ' ' => (Key::Space, Some(" ".to_owned())),
            c => (Key::Char(c), Some(c.to_string())),
        };
        self.frame_with([
            HostEvent::Key {
                key,
                down: true,
                repeat: false,
                text,
            },
            HostEvent::Key {
                key,
                down: false,
                repeat: false,
                text: None,
            },
        ]);
    }

    /// Do one step; a screenshot step returns its file name.
    fn step(
        &mut self,
        action: &Action,
        budget: u32,
        number: usize,
    ) -> Result<Option<String>, String> {
        match action {
            Action::Click { target } => {
                let at = self.aim(target, budget)?;
                self.click(at, PointerButton::Primary, 1);
            }
            Action::RightClick { target } => {
                let at = self.aim(target, budget)?;
                self.click(at, PointerButton::Secondary, 1);
            }
            Action::DoubleClick { target } => {
                let at = self.aim(target, budget)?;
                self.click(at, PointerButton::Primary, 2);
            }
            Action::Hover { target } => {
                let (x, y) = self.aim(target, budget)?;
                self.frame_with([HostEvent::PointerMoved { x, y }]);
            }
            Action::Drag { from, to } => {
                let (x0, y0) = self.aim(from, budget)?;
                let (x1, y1) = self.aim(to, budget)?;
                self.frame_with([HostEvent::PointerMoved { x: x0, y: y0 }]);
                self.frame_with([HostEvent::Button {
                    button: PointerButton::Primary,
                    down: true,
                }]);
                for n in 1..=DRAG_MOVES {
                    let t = n as f32 / DRAG_MOVES as f32;
                    self.frame_with([HostEvent::PointerMoved {
                        x: x0 + (x1 - x0) * t,
                        y: y0 + (y1 - y0) * t,
                    }]);
                }
                self.frame_with([HostEvent::Button {
                    button: PointerButton::Primary,
                    down: false,
                }]);
            }
            Action::Scroll { target, dx, dy } => {
                let (x, y) = self.aim(target, budget)?;
                self.frame_with([HostEvent::PointerMoved { x, y }]);
                self.frame_with([HostEvent::Wheel { dx: *dx, dy: *dy }]);
            }
            Action::Press { keys } => {
                for chord in parse_keys(keys)? {
                    self.chord(chord);
                }
            }
            Action::Type { text } => {
                for character in text.chars() {
                    self.typed(character);
                }
            }
            Action::WaitFrames { frames } => {
                for _ in 0..*frames {
                    self.draw();
                }
            }
            Action::ExpectText {
                target,
                text,
                exact,
            } => {
                self.until(budget, |report| {
                    let found = find_one(report, target)?;
                    let shown = text_of(report, found.node);
                    let holds = if *exact {
                        shown == *text
                    } else {
                        shown.contains(text.as_str())
                    };
                    if holds {
                        Ok(())
                    } else {
                        let how = if *exact { "be" } else { "contain" };
                        Err(format!("{} says {shown:?}, not {how} {text:?}", found.path))
                    }
                })?;
            }
            Action::ExpectVisible { target } => {
                self.until(budget, |report| {
                    let found = find_one(report, target)?;
                    match found.visible {
                        Some(_) => Ok(()),
                        None => Err(format!("{} is not on screen", found.path)),
                    }
                })?;
            }
            Action::ExpectAbsent { target } => {
                self.until(budget, |report| {
                    let found = find_all(report, target);
                    match found.first() {
                        None => Ok(()),
                        Some(first) => Err(format!(
                            "{target} still matches {} node(s), the first {}",
                            found.len(),
                            first.path
                        )),
                    }
                })?;
            }
            Action::Screenshot {} => {
                let file = format!("step-{number:02}.png");
                return match self.capture(file) {
                    Some(file) => Ok(Some(file)),
                    None => Err("this run has no GPU to take a screenshot with".into()),
                };
            }
        }
        Ok(None)
    }

    /// Capture the last frame as `file`, when the run has a GPU.
    fn capture(&mut self, file: String) -> Option<String> {
        #[cfg(feature = "headless")]
        {
            let gpu = self.gpu.as_mut()?;
            let size = (self.size.0.round() as u32, self.size.1.round() as u32);
            let clear = gpu.clear_color(self.app.backdrop());
            let pixels = gpu.capture_layers(self.layers, size, clear);
            self.screenshots.push(Screenshot {
                file: file.clone(),
                size,
                pixels,
            });
            Some(file)
        }
        #[cfg(not(feature = "headless"))]
        {
            let _ = file;
            None
        }
    }
}
