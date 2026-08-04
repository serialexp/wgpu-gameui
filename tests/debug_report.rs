//! End-to-end checks that [`DebugReport`] catches the positioning mistakes it
//! exists to catch — and stays quiet on a correct layout.
//!
//! These run headless with no GPU: the report reads a finished `DrawList`, so
//! the whole feature is testable on CI.

use wgpu_gameui::debug::{DebugReport, LintConfig, Problem, Severity};
use wgpu_gameui::layout::{Rect, VStack};
use wgpu_gameui::{
    Button, DrawContext, DrawList, FocusState, InputState, LayerStack, StyleResolver, TextBlock,
    Theme, UiContext,
};

const W: f32 = 800.0;
const H: f32 = 600.0;

fn screen() -> Rect {
    Rect::new(0.0, 0.0, W, H)
}

fn codes(r: &DebugReport) -> Vec<&'static str> {
    r.problems().iter().map(|p| p.code()).collect()
}

/// Build a `DrawContext` over the given list for one widget draw call.
fn ctx<'a>(
    list: &'a mut DrawList,
    focus: &'a mut FocusState,
    theme: &'a Theme,
    input: &'a InputState,
) -> DrawContext<'a> {
    DrawContext::new(list, focus, theme, input, W, H)
}

// ---------------------------------------------------------------------------
// The four mistakes agents actually make
// ---------------------------------------------------------------------------

#[test]
fn a_button_placed_off_screen_is_caught() {
    let theme = Theme::default();
    let input = InputState::default();
    let mut focus = FocusState::default();
    let mut list = DrawList::new();

    {
        let mut c = ctx(&mut list, &mut focus, &theme, &input);
        // Anchored from the wrong edge: x is past the right side of the screen.
        Button::new("Save").draw(Rect::new(860.0, 100.0, 120.0, 32.0), &mut c);
    }

    let report = DebugReport::measured(&mut list, screen());
    assert!(
        codes(&report).contains(&"off_screen"),
        "an off-screen button must be reported:\n{}",
        report.to_text()
    );
}

#[test]
fn content_overflowing_its_container_is_caught() {
    let theme = Theme::default();
    let input = InputState::default();
    let mut focus = FocusState::default();
    let mut list = DrawList::new();

    {
        let mut c = ctx(&mut list, &mut focus, &theme, &input);
        // The card reserves 150px; the button inside is 260px wide.
        c.draw_list
            .push_debug_scope_rect("card", Rect::new(20.0, 20.0, 150.0, 60.0));
        Button::new("A very wide button").draw(Rect::new(20.0, 34.0, 260.0, 32.0), &mut c);
        c.draw_list.pop_debug_scope();
    }

    let report = DebugReport::measured(&mut list, screen());
    let overflow = report
        .problems()
        .iter()
        .find(|p| p.code() == "overflows_declared")
        .unwrap_or_else(|| panic!("expected an overflow:\n{}", report.to_text()));
    match overflow {
        Problem::OverflowsDeclared { overshoot, .. } => {
            assert!(
                *overshoot > 100.0,
                "overshoot should be ~110px, got {overshoot}"
            );
        }
        _ => unreachable!(),
    }
}

#[test]
fn a_widget_collapsed_to_nothing_is_caught() {
    // The single hardest bug to see: the element leaves no geometry at all, so
    // neither a screenshot nor the draw list shows anything is missing.
    let mut list = DrawList::new();
    let available = 40.0;
    let padding = 24.0;
    let inner_width = available - padding * 2.0; // -8.0

    list.push_debug_scope_rect("row", Rect::new(10.0, 10.0, 200.0, 24.0));
    list.quad(10.0, 10.0, inner_width, 24.0, [1.0, 1.0, 1.0, 1.0]);
    list.pop_debug_scope();

    assert!(list.vertices.is_empty() && list.chrome_instances.is_empty());

    let report = DebugReport::measured(&mut list, screen());
    let c = codes(&report);
    assert!(
        c.contains(&"dropped_degenerate"),
        "the vanished primitive must be reported:\n{}",
        report.to_text()
    );
    assert!(c.contains(&"missing_paint"));
}

#[test]
fn a_column_misaligned_by_a_fraction_is_caught() {
    let mut list = DrawList::new();
    let mut focus = FocusState::new();
    let theme = Theme::default();
    let input = InputState::default();

    // Four buttons that should share a left edge; the third has a stray offset.
    // Nothing here declares a rect: the buttons declare their own allocations,
    // which is the whole point — an application supplies only the label.
    let lefts = [40.0f32, 40.0, 40.75, 40.0];
    {
        let mut c = ctx(&mut list, &mut focus, &theme, &input);
        c.draw_list.push_debug_scope("form");
        for (i, left) in lefts.into_iter().enumerate() {
            let rect = Rect::new(left, 40.0 + i as f32 * 36.0, 220.0, 28.0);
            Button::new(&format!("field{i}")).draw(rect, &mut c);
        }
        c.draw_list.pop_debug_scope();
    }

    let report = DebugReport::measured(&mut list, screen());
    let misalignment = report
        .problems()
        .iter()
        .find(|p| p.code() == "near_miss_alignment")
        .unwrap_or_else(|| panic!("expected a misalignment:\n{}", report.to_text()));

    match misalignment {
        Problem::NearMissAlignment {
            node, edge, delta, ..
        } => {
            assert_eq!(report.nodes[*node].name, "Button \"field2\"");
            assert_eq!(*edge, "left");
            assert!((delta - 0.75).abs() < 0.01, "delta {delta}");
        }
        _ => unreachable!(),
    }
}

// ---------------------------------------------------------------------------
// Quiet on correct layouts
// ---------------------------------------------------------------------------

#[test]
fn a_correct_layout_passes_assert_clean() {
    let theme = Theme::default();
    let input = InputState::default();
    let mut focus = FocusState::default();
    let mut list = DrawList::new();

    // A tidy vertical stack of three buttons inside a declared panel.
    let panel = Rect::new(40.0, 40.0, 240.0, 160.0);
    let layout = VStack::new(8.0)
        .with_padding(12.0)
        .child(32.0, 216.0)
        .child(32.0, 216.0)
        .child(32.0, 216.0);
    let result = wgpu_gameui::layout::LayoutNode::layout(&layout, panel);

    {
        let mut c = ctx(&mut list, &mut focus, &theme, &input);
        c.draw_list.push_debug_scope_rect("panel", panel);
        for i in 0..3 {
            let row = result.get(i + 1);
            c.draw_list.push_debug_scope_rect(format!("row{i}"), row);
            Button::new("Action").draw(row, &mut c);
            c.draw_list.pop_debug_scope();
        }
        c.draw_list.pop_debug_scope();
    }

    let report = DebugReport::measured(&mut list, screen());
    report.assert_clean();
    assert!(
        report.problems().is_empty(),
        "a correct layout should be completely silent:\n{}",
        report.to_text()
    );
}

#[test]
fn rows_scrolled_out_of_a_viewport_are_silent_but_a_hard_boundary_is_not() {
    // A scroll view draws rows its clip removes on every frame it is working
    // correctly, so that must be silent. The *same* geometry under an ordinary
    // clip — a window, a panel — means the content missed its container, which
    // is a hard error.
    let rows = |list: &mut DrawList| {
        for i in 0..12 {
            list.chrome_rect(
                Rect::new(20.0, 20.0 + i as f32 * 24.0, 200.0, 22.0),
                0.0,
                0.0,
                [0.3, 0.3, 0.35, 1.0],
                [0.0; 4],
            );
        }
    };

    let mut viewport = DrawList::new();
    viewport.push_clip_viewport(Rect::new(20.0, 20.0, 200.0, 100.0));
    rows(&mut viewport);
    viewport.pop_clip();

    let report = DebugReport::measured(&mut viewport, screen());
    report.assert_clean();
    assert!(
        report.problems().is_empty(),
        "a scroll view hiding its off-screen rows is not a defect:\n{}",
        report.to_text()
    );
    // The hidden rows are still in the tree, tagged with why they vanished.
    let hidden = report
        .nodes
        .iter()
        .filter(|n| n.effects.contains(&"off-viewport"))
        .count();
    assert!(hidden > 0, "{}", report.to_text());

    let mut boundary = DrawList::new();
    boundary.push_clip(Rect::new(20.0, 20.0, 200.0, 100.0));
    rows(&mut boundary);
    boundary.pop_clip();

    let report = DebugReport::measured(&mut boundary, screen());
    assert!(
        codes(&report).contains(&"fully_clipped"),
        "content lost behind a hard boundary must be reported:\n{}",
        report.to_text()
    );
    assert!(
        report.problems_at_least(Severity::Error).next().is_some(),
        "and it is an error, not a warning:\n{}",
        report.to_text()
    );
}

#[test]
fn an_empty_overlay_layer_is_not_reported_as_missing_paint() {
    // Dropdown/tooltip layers are pushed every frame and stay empty until
    // something opens; an empty one is normal, not a defect.
    let mut layers = LayerStack::new();
    layers.push_modal(Rect::new(100.0, 100.0, 200.0, 120.0));
    layers.pop_layer();

    let report = DebugReport::measured_layers(&mut layers, screen());
    assert!(
        !codes(&report).contains(&"missing_paint"),
        "{}",
        report.to_text()
    );
}

// ---------------------------------------------------------------------------
// Reporting surface
// ---------------------------------------------------------------------------

#[test]
fn windows_name_themselves_and_check_their_own_bounds() {
    let mut list = DrawList::new();
    {
        let mut ui = UiContext::new(&mut list);
        ui.push();
        ui.translate(500.0, 400.0);
        // A 400x300 window whose origin is at (500, 400) runs off an 800x600
        // screen — the sort of thing that only shows up once you look.
        ui.window_begin_named("inventory", 400.0, 300.0, false, true);
        // Local coordinates: the active transform places this at (500, 400).
        ui.list()
            .chrome_rect(Rect::new(0.0, 0.0, 400.0, 300.0), 0.0, 0.0, [1.0; 4], [0.0; 4]);
        ui.pop();
    }

    let report = DebugReport::measured(&mut list, screen());
    let window = report
        .node("inventory")
        .unwrap_or_else(|| panic!("window scope missing:\n{}", report.to_text()));
    assert!(window.named);
    assert_eq!(window.declared, Some(Rect::new(500.0, 400.0, 400.0, 300.0)));

    let strict = report.with_lints(&LintConfig::strict());
    assert!(
        codes(&strict).contains(&"partially_off_screen"),
        "{}",
        strict.to_text()
    );
}

#[test]
fn the_report_names_what_it_can_without_any_scopes() {
    let theme = Theme::default();
    let styles = StyleResolver::new(&theme);
    let mut list = DrawList::new();
    wgpu_gameui::Panel::draw_at(Rect::new(20.0, 20.0, 300.0, 120.0), &mut list, &styles);
    list.text(
        TextBlock::new("Inventory", 32.0, 32.0)
            .with_size(16.0)
            .with_max_width(200.0),
    );

    let report = DebugReport::measured(&mut list, screen());
    let text = report.to_text();

    // The label names itself from its own content.
    assert!(report.node("\"Inventory\"").is_some(), "{text}");
    // And the report says how much of the naming was guesswork.
    assert!(report.inferred_count() > 0);
    assert!(text.contains("push_debug_scope_rect"), "{text}");
}

#[test]
fn json_and_text_agree_on_the_problem_count() {
    let mut list = DrawList::new();
    list.chrome_rect(Rect::new(2000.0, 0.0, 40.0, 20.0), 0.0, 0.0, [1.0; 4], [0.0; 4]);
    let report = DebugReport::measured(&mut list, screen());

    let json = report.to_json();
    let occurrences = json.matches("\"code\":").count();
    assert_eq!(occurrences, report.problems().len());
    assert!(report.to_text().contains("off_screen"));
}

#[test]
fn write_to_dir_emits_both_renderings() {
    let dir = std::env::temp_dir().join("wgpu_gameui_debug_report_test");
    let _ = std::fs::remove_dir_all(&dir);

    let mut list = DrawList::new();
    list.push_debug_scope_rect("panel", Rect::new(0.0, 0.0, 100.0, 40.0));
    list.chrome_rect(Rect::new(0.0, 0.0, 100.0, 40.0), 0.0, 0.0, [1.0; 4], [0.0; 4]);
    list.pop_debug_scope();

    DebugReport::measured(&mut list, screen())
        .write_to_dir(&dir)
        .expect("write report");

    let txt = std::fs::read_to_string(dir.join("frame.txt")).expect("frame.txt");
    let json = std::fs::read_to_string(dir.join("frame.json")).expect("frame.json");
    assert!(txt.contains("panel"));
    assert!(json.contains("\"name\": \"panel\""));
    let _ = std::fs::remove_dir_all(&dir);
}
