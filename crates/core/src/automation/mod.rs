//! Scenarios: scripted tests that drive an app the way a person would.
//!
//! A [`Scenario`] is a JSON file of steps (click this, type that, expect this
//! text to show) that [`run`] plays against any app implementing
//! [`ScenarioApp`]. Input goes in as [`HostEvent`](crate::HostEvent)s, the same
//! events a window delivers, and what is on screen is read back from each
//! frame's [`DebugReport`](crate::DebugReport): a [`Locator`] names a debug
//! scope and the text inside it. Nothing waits on a clock. The runner draws
//! frames, each one nominal frame later than the last, so a run takes the same
//! frames on every machine.
//!
//! ```ignore
//! let scenario = Scenario::load("scenarios/counter.json")?;
//! let mut app = MyApp::start(&scenario.start)?;
//! let mut layers = LayerStack::new();
//! let report = run(&scenario, &mut app, &mut layers, None);
//! report.write("target/scenarios/counter")?;
//! report.assert_passed();
//! ```
//!
//! See `docs/design/ui-scenarios.md` for the design.

mod keys;
mod locate;
mod report;
mod run;
mod scenario;

pub use keys::{Chord, parse_keys};
pub use locate::{Match, find_all, find_one, text_of};
pub use report::{Report, Screenshot, Status, StepReport};
pub use run::{FrameClock, ScenarioApp, run};
pub use scenario::{
    Action, DEFAULT_TIMEOUT_FRAMES, Locator, MAX_TIMEOUT_FRAMES, SCHEMA_VERSION, Scenario, Step,
};

#[cfg(test)]
mod tests;
