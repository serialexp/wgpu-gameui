//! Throwaway visual proof for the syntax-highlight byte-mapping fix: render a
//! multiline Lua snippet through `UiContext::text_area_syntax` and capture the
//! frame to a PNG. Not tracked; used manually with:
//! ```
//! DISPLAY=:0 cargo run --example lua_highlight_probe --features syntax-lua
//! ```

use wgpu_gameui::{
    HeadlessGpu, InputState, SyntaxHighlighting, SyntaxTheme, Theme, UiContext, UiState,
    capture_draw_list, write_png,
};

fn main() {
    let Some(mut gpu) = HeadlessGpu::new() else {
        eprintln!("no GPU adapter");
        return;
    };
    let mut list = gpu.draw_list();
    let syntax = SyntaxHighlighting::lua(SyntaxTheme::default()).expect("lua config");
    let theme = Theme::default();
    let mut state = UiState::default();
    let input = InputState::default();

    let mut buffer = String::from(
        "-- a comment\nlocal function greet(name)\n  if name == nil then return end\n  for i = 1, 3 do print(i) end\n  return 'hello ' .. name\nend\nlocal n = 42\n",
    );

    {
        let mut ui = UiContext::interactive(&mut list, &input, &mut state, &theme);
        ui.text_area_syntax(1, &mut buffer, "", Some(320.0), 6, &syntax);
    }

    let clear = wgpu::Color {
        r: 0.08,
        g: 0.09,
        b: 0.11,
        a: 1.0,
    };
    let device = gpu.device().clone();
    let queue = gpu.queue().clone();
    let pixels = capture_draw_list(
        &device,
        &queue,
        gpu.renderer(),
        &list,
        (400, 220),
        1.0,
        clear,
    );
    std::fs::create_dir_all("test_output").unwrap();
    write_png("test_output/lua_highlight_probe.png", &pixels, (400, 220)).unwrap();
    eprintln!("wrote test_output/lua_highlight_probe.png");
}
