//! Headless offscreen render of the menu-screen primitives → PNG.
//!
//! A focused companion to `widget_gallery` (kept separate so each page stays
//! small enough to eyeball at once). Ignored by default (needs a GPU adapter):
//! ```
//! cargo test --test menu_gallery -- --ignored --nocapture
//! ```
//! Writes `test_output/menu_gallery.png`.

use wgpu_gameui::layout::{Anchor, MainAlign, Positioned, Rect, Size, VStack};
use wgpu_gameui::{
    ArrowFocusNav, Button, DrawContext, FocusState, Image, ImageFit, InputState, KeyboardNav,
    LayerStack, MenuList, SettingsForm, SettingsFormState, SettingsSpec,
    StyleResolver, Theme, UiRenderer, default_values, draw_scrim, map_gamepad,
};

const W: u32 = 640;
const H: u32 = 980;

fn solid_with_border(size: u32, fill: [u8; 4], border: [u8; 4], thickness: u32) -> Vec<u8> {
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let on_border =
                x < thickness || y < thickness || x >= size - thickness || y >= size - thickness;
            let c = if on_border { border } else { fill };
            let idx = ((y * size + x) * 4) as usize;
            out[idx..idx + 4].copy_from_slice(&c);
        }
    }
    out
}

#[test]
#[ignore = "needs a GPU adapter; writes a PNG for manual inspection"]
fn render_menu_gallery() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter available");
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("menu gallery device"),
            ..Default::default()
        },
        None,
    ))
    .expect("request device");
    let format = wgpu_gameui::CAPTURE_FORMAT;
    let mut ui = UiRenderer::new(&device, &queue, format, wgpu_gameui::shared_font_system());

    // A tileable 32x32 texture (opaque center + distinct border so the tile
    // grid is visible in the PNG).
    let tile_pixels = solid_with_border(32, [70, 90, 110, 255], [30, 40, 55, 255], 2);
    let tile_sprite = ui.load_sprite_rgba8("tile", 32, 32, &tile_pixels);

    let theme = Theme::default();
    let input = InputState::default();
    let mut focus = FocusState::new();
    focus.begin_frame(&input);

    let mut layers = LayerStack::new();
    {
        let list = layers.base_mut();

        list.text(
            wgpu_gameui::TextBlock::new("Menu screens", 20.0, 16.0)
                .with_size(22.0)
                .with_color(255, 255, 255),
        );

        let styles = StyleResolver::new(&theme);
        let sw = W as f32;
        let sh = H as f32;

        // ---- 1. Tile fit: the tiled-texture backdrop -----------------------
        let tile_dest = Rect::new(20.0, 56.0, 280.0, 120.0);
        list.text(
            wgpu_gameui::TextBlock::new("ImageFit::Tile backdrop", 20.0, 40.0)
                .with_size(11.0)
                .with_color(150, 160, 180),
        );
        Image::sprite(tile_sprite)
            .natural_size(32.0, 32.0)
            .fit(ImageFit::Tile)
            .draw(tile_dest, list);

        // ---- 2. Main menu over a scrim (the canonical composition) ---------
        let menu_area = Rect::new(20.0, 200.0, 280.0, 380.0);
        list.text(
            wgpu_gameui::TextBlock::new("scrim + MenuList (hover row 1)", 20.0, 184.0)
                .with_size(11.0)
                .with_color(150, 160, 180),
        );
        // Darken the "world": first fill the panel area with a busy stand-in,
        // then scrim it, then draw the menu on top.
        for (i, c) in [
            [0.85, 0.30, 0.25, 1.0],
            [0.30, 0.70, 0.40, 1.0],
            [0.25, 0.45, 0.85, 1.0],
            [0.85, 0.70, 0.25, 1.0],
        ]
        .iter()
        .enumerate()
        {
            let band_w = menu_area.width / 4.0;
            list.quad(
                menu_area.x + i as f32 * band_w,
                menu_area.y,
                band_w,
                menu_area.height,
                *c,
            );
        }
        draw_scrim(menu_area, list, &styles);

        // Pointer parked over row 1 ("Continue") so the PNG shows hover-driven
        // selection + the activated click path. Menu rect: y 240..540, content
        // (5*40 + 4*8 = 232) centered → rows at 274/322/370/418/466.
        let mut hover_input = input.clone();
        hover_input.mouse_x = menu_area.x + menu_area.width * 0.5;
        hover_input.mouse_y = menu_area.y + 142.0; // 342 = inside row 1
        let mut menu_focus = FocusState::new();
        menu_focus.begin_frame(&hover_input);
        {
            let mut ctx = DrawContext::new(list, &mut menu_focus, &theme, &hover_input, sw, sh);
            let mut selected = 0usize;
            let out = MenuList::new(&["New Game", "Continue", "Settings", "Credits", "Quit"])
                .row_height(40.0)
                .gap(8.0)
                .draw(
                    Rect::new(
                        menu_area.x,
                        menu_area.y + 40.0,
                        menu_area.width,
                        menu_area.height - 80.0,
                    ),
                    &mut selected,
                    &mut ctx,
                );
            assert!(out.selected_changed, "the parked pointer selects a row");
        }

        // ---- 3. Anchor + VStack composition: settings gear bottom-right ---
        list.text(
            wgpu_gameui::TextBlock::new("Anchor::BottomRight + Button", 320.0, 40.0)
                .with_size(11.0)
                .with_color(150, 160, 180),
        );
        let panel = Rect::new(320.0, 56.0, 300.0, 120.0);
        Image::sprite(tile_sprite)
            .natural_size(32.0, 32.0)
            .fit(ImageFit::Tile)
            .tint([1.0, 1.0, 1.0, 1.0])
            .draw(panel, list);
        draw_scrim(panel, list, &styles);
        {
            let gear = PanelButton { label: "Settings" };
            let corner = Positioned::new(
                Anchor::BottomRight {
                    offset: (-8.0, -8.0),
                },
                Size::fixed(90.0, 26.0),
                VStack::new(0.0),
            );
            let laid = corner.layout_screen(panel.width, panel.height);
            let inner = laid.container();
            let mut gear_focus = FocusState::new();
            gear_focus.begin_frame(&input);
            let mut ctx = DrawContext::new(list, &mut gear_focus, &theme, &input, sw, sh);
            gear.draw(
                Rect::new(
                    panel.x + inner.x,
                    panel.y + inner.y,
                    inner.width,
                    inner.height,
                ),
                &mut ctx,
            );
        }

        // ---- 4. Title + buttons centered on a transparent background ------
        list.text(
            wgpu_gameui::TextBlock::new("centered title + corner button column", 320.0, 184.0)
                .with_size(11.0)
                .with_color(150, 160, 180),
        );
        // Transparent background: nothing behind — the menu floats on clear.
        {
            // Title first (direct list access), then the context below.
            list.text(
                wgpu_gameui::TextBlock::new("MOONSHADE", 320.0, 210.0)
                    .with_size(28.0)
                    .with_color(240, 244, 255),
            );
            let mut t_focus = FocusState::new();
            t_focus.begin_frame(&input);
            let mut ctx = DrawContext::new(list, &mut t_focus, &theme, &input, sw, sh);
            let mut selected = 0usize;
            MenuList::new(&["Start", "Load", "Options", "Exit"])
                .row_height(34.0)
                .gap(6.0)
                .focusable(700)
                .draw(
                    Rect::new(370.0, 260.0, 200.0, 170.0),
                    &mut selected,
                    &mut ctx,
                );
        }
        {
            // Small corner button in the bottom-right of this half.
            let mut c_focus = FocusState::new();
            c_focus.begin_frame(&input);
            let mut cctx = DrawContext::new(list, &mut c_focus, &theme, &input, sw, sh);
            Button::new("v1.0.4")
                .enabled(true)
                .draw(Rect::new(540.0, 440.0, 64.0, 24.0), &mut cctx);
        }
    }

    // ---- 5. Settings form (drawn before capture, onto the base list) ----
    draw_settings_section(layers.base_mut(), &theme, &input);

    let pixels = wgpu_gameui::capture_layers(
        &device,
        &queue,
        &mut ui,
        &layers,
        (W, H),
        1.0,
        wgpu::Color {
            r: 0.05,
            g: 0.06,
            b: 0.08,
            a: 1.0,
        },
    );
    wgpu_gameui::write_png("test_output/menu_gallery.png", &pixels, (W, H))
        .expect("write menu gallery png");
    eprintln!("wrote test_output/menu_gallery.png ({W}x{H})");

    let _ = map_gamepad as fn(&mut InputState, &wgpu_gameui::GamepadNav);
    let _ = ArrowFocusNav::over(KeyboardNav);
    let _ = MainAlign::Start;
    let _ = map_gamepad as fn(&mut InputState, &wgpu_gameui::GamepadNav);
}

/// ---- 5. Settings form: declarative spec + values slice --------------------
/// Drawn in its own function so the section above stays readable.
fn draw_settings_section(list: &mut wgpu_gameui::DrawList, theme: &Theme, input: &InputState) {
    list.text(
        wgpu_gameui::TextBlock::new("SettingsForm (spec + values slice)", 20.0, 736.0)
            .with_size(11.0)
            .with_color(150, 160, 180),
    );

    let spec = SettingsSpec::new()
        .section("Video")
        .toggle("VSync")
        .slider("Brightness", 0.0..=1.0)
        .choice("Quality", &["Low", "Medium", "High"])
        .section("Controls")
        .binding("Jump")
        .action("Reset to defaults");
    let mut values = default_values(
        &spec,
        &[(0, true.into()), (1, 0.7.into()), (2, 1usize.into())],
    );
    // The binding row shows a custom capture: field 3 listening (armed last
    // frame, grace passed) with a pad button in this frame's snapshot — the
    // PNG shows the "press a key…" state resolving to "D-Up".
    let mut state = SettingsFormState::new();
    state.listening = None; // show bound labels, not the capture hint
    let mut focus = FocusState::new();
    focus.begin_frame(input);
    let mut ctx = DrawContext::new(list, &mut focus, theme, input, W as f32, H as f32);
    let rect = Rect::new(20.0, 752.0, 280.0, 220.0);
    let out = SettingsForm::new(&spec).draw(&mut values, rect, &mut state, &mut ctx);
    assert!(out.changed.is_empty(), "idle form writes nothing");
}

/// Minimal stand-in for a corner "settings" button (kept local; the point of
/// the panel is the composition, not the button).
struct PanelButton {
    label: &'static str,
}

impl PanelButton {
    fn draw(&self, rect: Rect, ctx: &mut DrawContext) {
        Button::new(self.label).draw(rect, ctx);
    }
}
