//! A code editor with problems in it, to look at.
//!
//! ```text
//! cargo test -p wgpu-gameui-code --test editor_gallery -- --ignored --nocapture
//! ```
//! Needs a GPU; writes `test_output/code_editor_gallery.png`: an error, a
//! warning and a note underlined, their lines washed and dotted, the counts
//! in the status strip, and the message of the error under the pointer.

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{DrawContext, FocusState, InputState, Srgb, StyleResolver, Theme, write_png};
use wgpu_gameui_code::{CodeEditor, Diagnostic, Language, Severity};

const SOURCE: &str = "\
fn main() {
    let player = spawn(\"hero\", 4, 9);
    let speed = plyer.speed();
    let unused = 3;
    move_to(player, speed)
}
";

#[test]
#[ignore = "needs a GPU adapter"]
fn render_code_editor_gallery() {
    use wgpu_gameui::HeadlessGpu;

    let Some(mut gpu) = HeadlessGpu::new() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };
    let theme = Theme::default();
    let mut editor = CodeEditor::new(SOURCE, Language::Rust);
    let at = |needle: &str| {
        let start = SOURCE.find(needle).expect(needle);
        start..start + needle.len()
    };
    editor.set_diagnostics(vec![
        Diagnostic::new(
            at("plyer"),
            Severity::Error,
            "cannot find value `plyer` in this scope",
        )
        .with_code("E0425"),
        Diagnostic::new(at("unused"), Severity::Warning, "unused variable: `unused`"),
        Diagnostic::new(
            at("move_to"),
            Severity::Info,
            "`move_to` is defined in `world`",
        ),
    ]);
    let rect = Rect::new(20.0, 20.0, 560.0, 200.0);
    let size = (600, 260);
    let mut focus = FocusState::new();

    // Two frames: the first lays it out, the second rests the pointer on
    // the error's underline.
    let mut layers = gpu.layer_stack();
    for pointer in [None, Some((0.0, 0.0))] {
        layers = gpu.layer_stack();
        let input = match pointer {
            None => InputState {
                mouse_x: -100.0,
                mouse_y: -100.0,
                ..InputState::default()
            },
            Some(_) => {
                // Line 3 ("let speed = plyer…"), over "plyer": the gutter is
                // about 52 px, a column about 7 px, a line 18 px after 6.
                InputState {
                    mouse_x: rect.x + 52.0 + 18.0 * 7.2,
                    mouse_y: rect.y + 6.0 + 2.5 * 18.0,
                    ..InputState::default()
                }
            }
        };
        focus.begin_frame(&input);
        let mut cx = DrawContext::new(
            layers.base_mut(),
            &mut focus,
            &theme,
            &input,
            size.0 as f32,
            size.1 as f32,
        );
        editor.draw(1, rect, &mut cx);
        focus.end_frame(None);
    }
    let styles = StyleResolver::new(&theme);
    editor.draw_layers(
        &mut layers,
        &styles,
        Rect::new(0.0, 0.0, size.0 as f32, size.1 as f32),
    );
    let pixels = gpu.capture_layers(&layers, size, Srgb::new([0.09, 0.1, 0.11, 1.0]));
    write_png("test_output/code_editor_gallery.png", &pixels, size).expect("write png");
    eprintln!(
        "wrote test_output/code_editor_gallery.png ({}x{})",
        size.0, size.1
    );
}
