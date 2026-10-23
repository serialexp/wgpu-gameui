//! Finding a [`Locator`]'s target in a frame's [`DebugReport`].

use crate::debug::{DebugNode, DebugReport, NodeKind, OFF_VIEWPORT};
use crate::layout::Rect;

use super::Locator;

/// A node a locator matched.
#[derive(Clone, Debug)]
pub struct Match<'r> {
    /// The node.
    pub node: &'r DebugNode,
    /// The part of it on screen and not clipped away, if any.
    pub visible: Option<Rect>,
    /// Its enclosing scope names and its own, joined by ` > `.
    pub path: String,
}

/// How many matches an error lists.
const LISTED: usize = 8;

/// Every node `locator` matches, in tree order.
pub fn find_all<'r>(report: &'r DebugReport, locator: &Locator) -> Vec<Match<'r>> {
    report
        .nodes
        .iter()
        .filter(|node| matches(report, node, locator))
        .map(|node| found(report, node))
        .collect()
}

/// The one node `locator` matches, or an error saying what did: nothing (and
/// where the text does appear, or which scopes have that name), or several
/// (each with its path and box), so a test is fixed by naming a scope rather
/// than by guessing an index.
pub fn find_one<'r>(report: &'r DebugReport, locator: &Locator) -> Result<Match<'r>, String> {
    let mut all = find_all(report, locator);
    match all.len() {
        1 => Ok(all.remove(0)),
        0 => Err(nothing_matched(report, locator)),
        n => Err(format!(
            "{n} matches for {locator}; name a scope to tell them apart:\n{}",
            listing(&all)
        )),
    }
}

/// A node's text: its own for a text node, else every text node under it in
/// tree order, joined by spaces.
pub fn text_of(report: &DebugReport, node: &DebugNode) -> String {
    if let Some(text) = &node.text {
        return text.clone();
    }
    report
        .nodes
        .iter()
        .filter(|other| other.text.is_some() && descends_from(report, other, node.id))
        .filter_map(|other| other.text.as_deref())
        .collect::<Vec<_>>()
        .join(" ")
}

fn matches(report: &DebugReport, node: &DebugNode, locator: &Locator) -> bool {
    match &locator.text {
        Some(text) => {
            node.kind == NodeKind::Text
                && node.text.as_deref().is_some_and(|own| {
                    if locator.exact {
                        own == text
                    } else {
                        own.contains(text.as_str())
                    }
                })
                && in_order(&locator.scope, &enclosing_scopes(report, node))
        }
        None => {
            let Some((last, outer)) = locator.scope.split_last() else {
                return false;
            };
            is_scope(node) && names(last, &node.name) && {
                in_order(outer, &enclosing_scopes(report, node))
            }
        }
    }
}

fn is_scope(node: &DebugNode) -> bool {
    node.kind == NodeKind::Scope && node.named
}

/// Whether a locator's scope entry names a scope: its whole name, or the kind
/// before its quoted label (`Button` for `Button "Save"`).
fn names(entry: &str, name: &str) -> bool {
    name == entry
        || name
            .strip_prefix(entry)
            .is_some_and(|rest| rest.starts_with(" \""))
}

/// Whether every entry of `path` names one of `scopes`, in order.
fn in_order(path: &[String], scopes: &[&str]) -> bool {
    let mut scopes = scopes.iter();
    path.iter()
        .all(|entry| scopes.any(|scope| names(entry, scope)))
}

/// The names of the named scopes enclosing `node`, outermost first.
fn enclosing_scopes<'r>(report: &'r DebugReport, node: &DebugNode) -> Vec<&'r str> {
    let mut names = Vec::new();
    let mut parent = node.parent;
    while let Some(index) = parent {
        let enclosing = &report.nodes[index];
        if is_scope(enclosing) {
            names.push(enclosing.name.as_str());
        }
        parent = enclosing.parent;
    }
    names.reverse();
    names
}

fn descends_from(report: &DebugReport, node: &DebugNode, ancestor: usize) -> bool {
    let mut parent = node.parent;
    while let Some(index) = parent {
        if index == ancestor {
            return true;
        }
        parent = report.nodes[index].parent;
    }
    false
}

fn found<'r>(report: &'r DebugReport, node: &'r DebugNode) -> Match<'r> {
    let mut path = enclosing_scopes(report, node);
    path.push(&node.name);
    Match {
        node,
        visible: visible(report, node),
        path: path.join(" > "),
    }
}

/// What of `node` the frame shows: inside the screen and its clip, and not
/// scrolled out of a viewport.
fn visible(report: &DebugReport, node: &DebugNode) -> Option<Rect> {
    if node.effects.contains(&OFF_VIEWPORT) {
        return None;
    }
    let mut shown = node.bounds.intersection(report.screen)?;
    if let Some(clip) = node.clip {
        shown = shown.intersection(clip)?;
    }
    (shown.width > 0.0 && shown.height > 0.0).then_some(shown)
}

fn listing(matches: &[Match<'_>]) -> String {
    let mut lines: Vec<String> = matches
        .iter()
        .take(LISTED)
        .map(|found| {
            let b = found.node.bounds;
            let shown = if found.visible.is_some() {
                ""
            } else {
                ", not on screen"
            };
            format!(
                "  {} at ({:.0}, {:.0}) {:.0}×{:.0}{shown}",
                found.path, b.x, b.y, b.width, b.height
            )
        })
        .collect();
    if matches.len() > LISTED {
        lines.push(format!("  … and {} more", matches.len() - LISTED));
    }
    lines.join("\n")
}

fn nothing_matched(report: &DebugReport, locator: &Locator) -> String {
    let mut hint = String::new();
    if locator.text.is_some() && !locator.scope.is_empty() {
        let anywhere = Locator {
            scope: Vec::new(),
            ..locator.clone()
        };
        let elsewhere = find_all(report, &anywhere);
        if !elsewhere.is_empty() {
            hint = format!(
                "\nthe text is on screen, but not under those scopes:\n{}",
                listing(&elsewhere)
            );
        }
    } else if let Some(last) = locator.scope.last() {
        let named = Locator {
            scope: vec![last.clone()],
            text: None,
            exact: false,
        };
        let scopes = find_all(report, &named);
        if !scopes.is_empty() && locator.text.is_none() {
            hint = format!("\nscopes named {last:?} are under:\n{}", listing(&scopes));
        }
    }
    format!("nothing matches {locator}{hint}")
}
