use super::*;
use crate::diagnostics::Severity;

#[test]
fn a_wave_runs_from_end_to_end_through_its_peaks() {
    let points = wave(10.0, 18.0, 10.0, 50.0);
    let xs: Vec<f32> = points.iter().map(|p| p[0]).collect();
    assert_eq!(xs, vec![10.0, 12.0, 14.0, 16.0, 18.0]);
    let ys: Vec<f32> = points.iter().map(|p| p[1]).collect();
    assert_eq!(ys, vec![49.0, 51.0, 49.0, 51.0, 49.0]);
}

#[test]
fn a_wave_cut_off_keeps_the_phase_it_had() {
    // From 11, halfway up the first fall: the same wave as from 10, cut.
    let points = wave(11.0, 13.0, 10.0, 50.0);
    assert_eq!(points, vec![[11.0, 50.0], [12.0, 51.0], [13.0, 50.0]]);
}

#[test]
fn nothing_is_drawn_for_no_width() {
    assert!(wave(10.0, 10.0, 0.0, 0.0).is_empty());
}

#[test]
fn the_message_goes_below_the_line_or_above_it_when_there_is_no_room() {
    let viewport = Rect::new(0.0, 0.0, 400.0, 300.0);
    let line = Rect::new(50.0, 100.0, 30.0, 18.0);
    assert_eq!(place(line, 200.0, 40.0, viewport).y, 120.0);
    let low = Rect::new(50.0, 270.0, 30.0, 18.0);
    assert_eq!(place(low, 200.0, 40.0, viewport).y, 228.0);
    // Pushed back inside from the right edge.
    let right = Rect::new(350.0, 100.0, 30.0, 18.0);
    assert_eq!(place(right, 200.0, 40.0, viewport).x, 200.0);
}

#[test]
fn the_message_shows_the_severity_its_code_and_the_text() {
    let theme = wgpu_gameui::Theme::default();
    let styles = StyleResolver::new(&theme);
    let mut layers = LayerStack::new();
    let diagnostic =
        Diagnostic::new(0..3, Severity::Error, "cannot find value `plyer`").with_code("E0425");
    let rect = draw_message(
        &mut layers,
        &styles,
        &diagnostic,
        [1.0, 0.0, 0.0, 1.0],
        Rect::new(10.0, 10.0, 20.0, 18.0),
        Rect::new(0.0, 0.0, 800.0, 600.0),
    );
    let layer = layers.layers().last().expect("a tooltip layer");
    assert_eq!(layer.rect, rect);
    let texts: Vec<&str> = layer
        .list
        .texts
        .iter()
        .map(|t| t.content.as_str())
        .collect();
    assert_eq!(texts, vec!["ERROR E0425", "cannot find value `plyer`"]);
    assert!(rect.width <= MESSAGE_MAX_WIDTH);
}
