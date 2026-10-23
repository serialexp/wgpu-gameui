//! What a run did, step by step, for a test to assert on and a person to read.

use std::fmt::Write as _;
use std::path::Path;

use serde::Serialize;

/// How a run, or one step of it, went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// It did what it said.
    Passed,
    /// It didn't; see the error.
    Failed,
    /// An earlier step failed, so it never ran.
    Skipped,
}

/// One step's outcome.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepReport {
    /// Its number, from 1.
    pub index: usize,
    /// Its name, if the file gave one.
    pub name: Option<String>,
    /// Its action, as the file spells it.
    pub action: &'static str,
    /// How it went.
    pub status: Status,
    /// Frames it drew.
    pub frames: u64,
    /// Why it failed.
    pub error: Option<String>,
    /// The file its screenshot is in, next to the report.
    pub screenshot: Option<String>,
}

/// A captured frame, kept until the report is written.
#[derive(Clone, Debug)]
pub struct Screenshot {
    /// Its file name.
    pub file: String,
    /// Width and height in pixels.
    pub size: (u32, u32),
    /// RGBA8 pixels, row by row.
    pub pixels: Vec<u8>,
}

/// A whole run's outcome.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// The report format's version: [`SCHEMA_VERSION`](super::SCHEMA_VERSION).
    pub schema_version: u32,
    /// The scenario's name.
    pub name: String,
    /// How it went.
    pub status: Status,
    /// Frames drawn in all.
    pub frames: u64,
    /// The first failure, with its step.
    pub error: Option<String>,
    /// Every step, run or skipped.
    pub steps: Vec<StepReport>,
    /// The failing frame's [`DebugReport`](crate::DebugReport) as text.
    #[serde(skip)]
    pub failure_tree: Option<String>,
    /// Screenshots taken.
    #[serde(skip)]
    pub screenshots: Vec<Screenshot>,
}

impl Report {
    /// Whether every step passed.
    pub fn passed(&self) -> bool {
        self.status == Status::Passed
    }

    /// Panic with the failure and the failing frame's tree unless every step
    /// passed: the way a test asserts on a run.
    pub fn assert_passed(&self) {
        if let Some(error) = &self.error {
            let tree = self.failure_tree.as_deref().unwrap_or("(no frame)");
            panic!(
                "scenario {:?} failed at {error}\n\nthe frame:\n{tree}",
                self.name
            );
        }
    }

    /// Write `report.json`, `report.html`, `failure.txt` (the failing frame's
    /// tree) and the screenshots into `dir`, creating it if needed.
    pub fn write(&self, dir: impl AsRef<Path>) -> std::io::Result<()> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(dir.join("report.json"), json)?;
        std::fs::write(dir.join("report.html"), self.html())?;
        if let Some(tree) = &self.failure_tree {
            std::fs::write(dir.join("failure.txt"), tree)?;
        }
        for shot in &self.screenshots {
            crate::write_png(dir.join(&shot.file), &shot.pixels, shot.size)?;
        }
        Ok(())
    }

    /// The report as a page that needs nothing but a browser.
    pub fn html(&self) -> String {
        let mut page = String::new();
        let status = |status: Status| match status {
            Status::Passed => "passed",
            Status::Failed => "failed",
            Status::Skipped => "skipped",
        };
        let _ = write!(
            page,
            "<!doctype html><meta charset=utf-8><title>{name}</title>\
             <style>body{{font:14px system-ui;margin:2em}}td,th{{padding:4px 10px;text-align:left;vertical-align:top}}\
             .passed{{color:#2a7}}.failed{{color:#c33}}.skipped{{color:#999}}img{{max-width:600px;border:1px solid #ccc}}\
             pre{{white-space:pre-wrap}}</style>\
             <h1>{name} <span class={status}>{status}</span></h1><p>{frames} frames</p>\
             <table><tr><th>#</th><th>Step</th><th>Status</th><th>Frames</th><th>Detail</th></tr>",
            name = escape(&self.name),
            status = status(self.status),
            frames = self.frames,
        );
        for step in &self.steps {
            let label = step.name.as_deref().unwrap_or(step.action);
            let _ = write!(
                page,
                "<tr><td>{}</td><td>{}</td><td class={s}>{s}</td><td>{}</td><td>",
                step.index,
                escape(label),
                step.frames,
                s = status(step.status),
            );
            if let Some(error) = &step.error {
                let _ = write!(page, "<pre>{}</pre>", escape(error));
            }
            if let Some(file) = &step.screenshot {
                let _ = write!(page, "<img src=\"{0}\" alt=\"{0}\">", escape(file));
            }
            page.push_str("</td></tr>");
        }
        page.push_str("</table>");
        if let Some(tree) = &self.failure_tree {
            let _ = write!(
                page,
                "<h2>The failing frame</h2><pre>{}</pre>",
                escape(tree)
            );
        }
        page
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}
