//! The scenario file: what it may contain, and the checks it passes before
//! anything runs.

use std::fmt;
use std::path::Path;

use serde::Deserialize;

use super::keys::parse_keys;

/// The only `schemaVersion` this runner reads.
pub const SCHEMA_VERSION: u32 = 1;
/// How many frames a step may take when neither it nor the scenario says:
/// two seconds of animation at 60 Hz.
pub const DEFAULT_TIMEOUT_FRAMES: u32 = 120;
/// The most frames one step may be given.
pub const MAX_TIMEOUT_FRAMES: u32 = 100_000;

/// A scripted test: an app state to start from, a screen size, and steps.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scenario {
    /// Must be [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// What the scenario checks, for the report.
    pub name: String,
    /// The app state to start from. Its meaning is the app's own (agent-ui
    /// maps it onto a fixture).
    pub start: String,
    /// The screen, in logical pixels.
    #[serde(default = "default_size")]
    pub size: [f32; 2],
    /// Frames a step may take unless it says otherwise.
    #[serde(default = "default_timeout")]
    pub timeout_frames: u32,
    /// What to do, in order.
    pub steps: Vec<Step>,
}

fn default_size() -> [f32; 2] {
    [1200.0, 800.0]
}

fn default_timeout() -> u32 {
    DEFAULT_TIMEOUT_FRAMES
}

/// One step: an action, and optionally a name and its own frame budget.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    /// Shown in the report in place of the action.
    #[serde(default)]
    pub name: Option<String>,
    /// Frames this step may take, in place of the scenario's.
    #[serde(default)]
    pub timeout_frames: Option<u32>,
    /// What it does.
    #[serde(flatten)]
    pub action: Action,
}

/// Something on screen: a run of debug scopes, and text inside them.
///
/// `scope` lists scope names, outermost first. A node is under them when its
/// enclosing scopes include them in that order, not necessarily directly
/// nested. An entry matches a scope's whole name (`Button "Save"`) or the kind
/// before its label (`Button`). With `text`, the target is a text node under
/// the scopes that contains it (equals it, with `exact`); without, it is the
/// scope the last entry names.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Locator {
    /// Scope names, outermost first.
    #[serde(default)]
    pub scope: Vec<String>,
    /// Text the target shows.
    #[serde(default)]
    pub text: Option<String>,
    /// `text` must be the whole text, not part of it.
    #[serde(default)]
    pub exact: bool,
}

impl fmt::Display for Locator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.scope.is_empty() {
            write!(f, "scope {:?}", self.scope.join(" > "))?;
        }
        if let Some(text) = &self.text {
            if !self.scope.is_empty() {
                f.write_str(", ")?;
            }
            let how = if self.exact { "exactly" } else { "containing" };
            write!(f, "text {how} {text:?}")?;
        }
        Ok(())
    }
}

/// What a step does. Pointer actions aim at the centre of the target's
/// visible part.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub enum Action {
    /// Move there, press and release the primary button, a frame each.
    Click {
        /// What to click.
        target: Locator,
    },
    /// The same with the secondary button.
    RightClick {
        /// What to click.
        target: Locator,
    },
    /// Two clicks, a frame apart.
    DoubleClick {
        /// What to click.
        target: Locator,
    },
    /// Move the pointer there.
    Hover {
        /// Where to move.
        target: Locator,
    },
    /// Press on `from`, move to `to` in steps, release.
    Drag {
        /// Where the press starts.
        from: Locator,
        /// Where it is let go.
        to: Locator,
    },
    /// Turn the wheel over the target, in lines (positive is up and left).
    Scroll {
        /// What to scroll.
        target: Locator,
        /// Sideways lines.
        #[serde(default)]
        dx: f32,
        /// Lines.
        #[serde(default)]
        dy: f32,
    },
    /// Press keys, a frame each: `"tab enter"`, `"cmd-a"`. See
    /// [`parse_keys`].
    Press {
        /// Space-separated chords.
        keys: String,
    },
    /// Type text into whatever has focus, a frame per character.
    Type {
        /// What to type; a newline presses Enter.
        text: String,
    },
    /// Draw frames with no input.
    WaitFrames {
        /// How many.
        frames: u32,
    },
    /// The target's text (all the text under it, for a scope) contains
    /// `text`, or equals it with `exact`.
    ExpectText {
        /// Whose text.
        target: Locator,
        /// What it should say.
        text: String,
        /// The whole text must equal `text`.
        #[serde(default)]
        exact: bool,
    },
    /// Exactly one match, painted on screen and not clipped away. It may
    /// still be covered by something drawn over it.
    ExpectVisible {
        /// What should show.
        target: Locator,
    },
    /// No match at all.
    ExpectAbsent {
        /// What should be gone.
        target: Locator,
    },
    /// Capture the frame, when the run has a GPU.
    Screenshot {},
}

impl Action {
    /// The action's name as the file spells it.
    pub fn label(&self) -> &'static str {
        match self {
            Action::Click { .. } => "click",
            Action::RightClick { .. } => "rightClick",
            Action::DoubleClick { .. } => "doubleClick",
            Action::Hover { .. } => "hover",
            Action::Drag { .. } => "drag",
            Action::Scroll { .. } => "scroll",
            Action::Press { .. } => "press",
            Action::Type { .. } => "type",
            Action::WaitFrames { .. } => "waitFrames",
            Action::ExpectText { .. } => "expectText",
            Action::ExpectVisible { .. } => "expectVisible",
            Action::ExpectAbsent { .. } => "expectAbsent",
            Action::Screenshot {} => "screenshot",
        }
    }

    fn locators(&self) -> Vec<&Locator> {
        match self {
            Action::Click { target }
            | Action::RightClick { target }
            | Action::DoubleClick { target }
            | Action::Hover { target }
            | Action::Scroll { target, .. }
            | Action::ExpectText { target, .. }
            | Action::ExpectVisible { target }
            | Action::ExpectAbsent { target } => vec![target],
            Action::Drag { from, to } => vec![from, to],
            Action::Press { .. }
            | Action::Type { .. }
            | Action::WaitFrames { .. }
            | Action::Screenshot {} => Vec::new(),
        }
    }
}

impl Scenario {
    /// Parse and check a scenario.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let scenario: Scenario = serde_json::from_str(json).map_err(|error| error.to_string())?;
        scenario.check()?;
        Ok(scenario)
    }

    /// Read, parse and check a scenario file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let json = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        Self::from_json(&json).map_err(|error| format!("{}: {error}", path.display()))
    }

    /// The frames step `step` may take.
    pub fn budget(&self, step: &Step) -> u32 {
        step.timeout_frames.unwrap_or(self.timeout_frames)
    }

    fn check(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "schemaVersion {} is not {SCHEMA_VERSION}, the only one this runner reads",
                self.schema_version
            ));
        }
        let [width, height] = self.size;
        if !(width >= 1.0 && height >= 1.0 && width.is_finite() && height.is_finite()) {
            return Err(format!("size {width}×{height} is not a screen"));
        }
        check_budget(self.timeout_frames).map_err(|error| format!("timeoutFrames {error}"))?;
        if self.steps.is_empty() {
            return Err("there are no steps".into());
        }
        for (index, step) in self.steps.iter().enumerate() {
            let at =
                |error: String| format!("step {} ({}): {error}", index + 1, step.action.label());
            if let Some(frames) = step.timeout_frames {
                check_budget(frames).map_err(|error| at(format!("timeoutFrames {error}")))?;
            }
            for locator in step.action.locators() {
                if locator.scope.is_empty() && locator.text.is_none() {
                    return Err(at("a target needs a scope, text, or both".into()));
                }
                if locator.scope.iter().any(String::is_empty) || locator.text.as_deref() == Some("")
                {
                    return Err(at("a target's scope names and text can't be empty".into()));
                }
            }
            match &step.action {
                Action::Press { keys } => {
                    parse_keys(keys).map_err(at)?;
                }
                Action::Type { text } if text.is_empty() => {
                    return Err(at("there is nothing to type".into()));
                }
                Action::WaitFrames { frames } => {
                    check_budget(*frames).map_err(|error| at(format!("frames {error}")))?;
                }
                Action::Scroll { dx, dy, .. } if !dx.is_finite() || !dy.is_finite() => {
                    return Err(at("dx and dy must be numbers".into()));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn check_budget(frames: u32) -> Result<(), String> {
    if (1..=MAX_TIMEOUT_FRAMES).contains(&frames) {
        Ok(())
    } else {
        Err(format!(
            "{frames} is not between 1 and {MAX_TIMEOUT_FRAMES}"
        ))
    }
}
