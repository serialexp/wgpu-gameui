//! The retained tree in a window (`docs/design/retained-mode.md`, Phase 6):
//! a button that counts its clicks, and a scrolling list of a thousand rows
//! that light up under the pointer and are picked by a click.
//!
//! What it shows is the host's half: the tree is built once, input is handed
//! to it as it comes, and a frame is drawn only when [`Ui::needs_frame`] says
//! the tree would draw something new (a hover moved, a click, the wheel), so
//! between them the window idles. Each frame drawn prints what the tree did
//! for it: how many nodes it laid out and painted, how many records it
//! patched in place, and how many lists it recorded.
//!
//! ```text
//! cargo run -p wgpu-gameui-retained --example retained_window
//! ```
//!
//! Quit with Escape or by closing the window.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use wgpu_gameui::color::oklch;
use wgpu_gameui::{FontSystemHandle, TextAlign, UiRenderer, Weight};
use wgpu_gameui_retained::{
    Align, Button, FrameContext, Insets, Look, Node, Signal, Size, Spec, Stack, Text, Ui,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// Rows in the list.
const ROWS: usize = 1000;
/// How far a wheel's line step scrolls, in logical pixels.
const LINE: f32 = 40.0;

/// The colours, from the Forge dark palette's lightnesses.
struct Palette {
    page: [f32; 4],
    panel: [f32; 4],
    row: [f32; 4],
    row_hovered: [f32; 4],
    row_picked: [f32; 4],
    button: [f32; 4],
    button_hovered: [f32; 4],
    button_pressed: [f32; 4],
    on_button: [f32; 4],
    text: [f32; 4],
    muted: [f32; 4],
}

fn palette() -> Palette {
    Palette {
        page: oklch(0.17, 0.01, 250.0, 1.0),
        panel: oklch(0.21, 0.012, 250.0, 1.0),
        row: oklch(0.21, 0.012, 250.0, 1.0),
        row_hovered: oklch(0.27, 0.015, 250.0, 1.0),
        row_picked: oklch(0.42, 0.08, 200.0, 1.0),
        button: oklch(0.62, 0.10, 200.0, 1.0),
        button_hovered: oklch(0.70, 0.11, 200.0, 1.0),
        button_pressed: oklch(0.55, 0.10, 200.0, 1.0),
        on_button: oklch(0.16, 0.01, 250.0, 1.0),
        text: oklch(0.93, 0.005, 250.0, 1.0),
        muted: oklch(0.68, 0.01, 250.0, 1.0),
    }
}

/// What the host (and the tests) reach into the tree by.
#[cfg_attr(not(test), allow(dead_code))]
struct Scene {
    button: Node,
    list: Node,
    rows: Vec<Node>,
    clicks: Signal<u32>,
    picked: Signal<String>,
}

/// Build the tree: a header with the button and its count, and the list.
fn build(ui: &mut Ui) -> Scene {
    let colors = palette();
    let root = ui.root();
    let header = ui.add(
        root,
        Spec::new()
            .width(Size::Fill(1.0))
            .stack(Stack::row().gap(16.0).align(Align::Start, Align::Center)),
    );
    ui.add(
        header,
        Spec::new().text(
            Text {
                weight: Weight::SEMIBOLD,
                ..Text::new("Retained tree", 20.0).no_wrap()
            },
            colors.text,
        ),
    );

    // The button: its fill follows its hover and press signals, its label a
    // count the click writes. Only the button's own drawing and its label
    // change when it is used; the rest of the frame is left as it was.
    let clicks = ui.signal(0u32);
    let hovered = ui.signal(false);
    let pressed = ui.signal(false);
    let [idle, over, down] = [colors.button, colors.button_hovered, colors.button_pressed];
    let fill = ui.memo(move |r| {
        if r.get(pressed) {
            down
        } else if r.get(hovered) {
            over
        } else {
            idle
        }
    });
    let button = ui.add(
        header,
        Spec::new()
            .width(Size::Fixed(180.0))
            .height(Size::Fixed(36.0))
            .stack(Stack::column().align(Align::Center, Align::Center))
            .fill(fill)
            .look(Look {
                radius: 6.0,
                ..Look::default()
            })
            .hover(hovered)
            .press(pressed)
            .on_click(move |ui, button| {
                if button == Button::Primary {
                    let n = ui.get(clicks);
                    ui.set(clicks, n + 1);
                }
            }),
    );
    let label = ui.memo(move |r| {
        let n = r.get(clicks);
        let label = match n {
            0 => "Click me".to_owned(),
            1 => "Clicked once".to_owned(),
            n => format!("Clicked {n} times"),
        };
        Text::new(label, 14.0).no_wrap()
    });
    ui.add(button, Spec::new().text(label, colors.on_button));

    let picked_label = ui.signal(String::from("Nothing picked"));
    let caption = ui.memo(move |r| Text::new(r.get(picked_label), 14.0).no_wrap());
    ui.add(header, Spec::new().text(caption, colors.muted));

    // The list scrolls; only the rows near its view are laid out and only
    // those in it drawn. Each row lights up under the pointer, and a click
    // picks it, putting down the one picked before.
    let list = ui.add(
        root,
        Spec::new()
            .width(Size::Fill(1.0))
            .height(Size::Fill(1.0))
            .stack(Stack::column().gap(1.0).padding(Insets::all(4.0)))
            .fill(colors.panel)
            .look(Look {
                radius: 8.0,
                ..Look::default()
            })
            .scroll(),
    );
    let picked: Rc<Cell<Option<Signal<bool>>>> = Rc::new(Cell::new(None));
    let rows = (0..ROWS)
        .map(|i| add_row(ui, list, i, &colors, &picked, picked_label))
        .collect();
    Scene {
        button,
        list,
        rows,
        clicks,
        picked: picked_label,
    }
}

fn add_row(
    ui: &mut Ui,
    list: Node,
    i: usize,
    colors: &Palette,
    picked: &Rc<Cell<Option<Signal<bool>>>>,
    picked_label: Signal<String>,
) -> Node {
    let hovered = ui.signal(false);
    let chosen = ui.signal(false);
    let [plain, over, on] = [colors.row, colors.row_hovered, colors.row_picked];
    let fill = ui.memo(move |r| {
        if r.get(chosen) {
            on
        } else if r.get(hovered) {
            over
        } else {
            plain
        }
    });
    let picked = picked.clone();
    let row = ui.add(
        list,
        Spec::new()
            .width(Size::Fill(1.0))
            .height(Size::Fixed(28.0))
            .stack(
                Stack::row()
                    .padding(Insets::xy(12.0, 0.0))
                    .gap(12.0)
                    .align(Align::Start, Align::Center),
            )
            .fill(fill)
            .look(Look {
                radius: 4.0,
                ..Look::default()
            })
            .hover(hovered)
            .on_click(move |ui, button| {
                if button != Button::Primary {
                    return;
                }
                if let Some(before) = picked.replace(Some(chosen)) {
                    ui.set(before, false);
                }
                ui.set(chosen, true);
                ui.set(picked_label, format!("Picked row {}", i + 1));
            }),
    );
    ui.add(
        row,
        Spec::new().width(Size::Fixed(48.0)).text(
            Text::new(format!("{}", i + 1), 13.0)
                .no_wrap()
                .align(TextAlign::End, Align::Center),
            colors.muted,
        ),
    );
    ui.add(
        row,
        Spec::new().text(
            Text::new(format!("Row {} of {ROWS}", i + 1), 14.0).no_wrap(),
            colors.text,
        ),
    );
    row
}

struct Gpu {
    renderer: UiRenderer,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    fonts: FontSystemHandle,
    ui: Ui,
    /// The clock the tree's frames are timed by.
    start: Instant,
    frames: u64,
}

impl App {
    fn new() -> Self {
        let fonts = wgpu_gameui::shared_font_system();
        let root = Spec::new()
            .width(Size::Fill(1.0))
            .height(Size::Fill(1.0))
            .stack(Stack::column().gap(16.0).padding(Insets::all(20.0)))
            .fill(palette().page);
        let mut ui = Ui::with_root(fonts.clone(), root);
        build(&mut ui);
        Self {
            window: None,
            gpu: None,
            fonts,
            ui,
            start: Instant::now(),
            frames: 0,
        }
    }

    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    /// Ask for a frame if the tree would draw a new one.
    fn redraw_if_needed(&self) {
        if self.ui.needs_frame(self.now())
            && let Some(window) = &self.window
        {
            window.request_redraw();
        }
    }

    fn draw(&mut self) {
        let (Some(gpu), Some(window)) = (self.gpu.as_mut(), self.window.as_ref()) else {
            return;
        };
        let scale = window.scale_factor() as f32;
        let size = (gpu.config.width, gpu.config.height);
        let now = self.start.elapsed().as_secs_f64();
        let list = self.ui.frame(FrameContext {
            now,
            viewport: [size.0 as f32 / scale, size.1 as f32 / scale],
            fonts: 0,
            scale,
        });
        let frame = match gpu.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(_) => {
                gpu.surface.configure(&gpu.device, &gpu.config);
                window.request_redraw();
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("retained_window"),
            });
        gpu.renderer.begin_frame();
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(gpu.renderer.clear_color(palette().page).color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        gpu.renderer.render(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &view,
            size,
            scale,
            list,
        );
        gpu.queue.submit(Some(encoder.finish()));
        window.pre_present_notify();
        frame.present();

        self.frames += 1;
        let s = self.ui.frame_stats();
        println!(
            "frame {:>4}: laid out {:>4}, painted {:>4}, patched {:>4} records, \
             recorded {} lists, placed {}, dropped {}",
            self.frames, s.laid_out, s.painted, s.patched, s.recorded, s.placed, s.dropped
        );
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("retained_window — wgpu-gameui")
            .with_inner_size(winit::dpi::LogicalSize::new(640.0, 520.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("request adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("retained_window device"),
                ..Default::default()
            },
            None,
        ))
        .expect("request device");
        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let mut renderer = UiRenderer::new(&device, &queue, format, self.fonts.clone());
        // The tree's list says what each frame patched in it: the renderer
        // rewrites only that.
        renderer.set_frame_comparison(true);
        self.gpu = Some(Gpu {
            renderer,
            surface,
            config,
            device,
            queue,
        });
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let scale = self
            .window
            .as_ref()
            .map_or(1.0, |window| window.scale_factor() as f32);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed
                    && event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.config.width = size.width.max(1);
                    gpu.config.height = size.height.max(1);
                    gpu.surface.configure(&gpu.device, &gpu.config);
                }
                // A new viewport is a new layout: draw it whatever the tree
                // says.
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                // The tree lays out in logical pixels; winit reports physical.
                self.ui
                    .pointer_moved(position.x as f32 / scale, position.y as f32 / scale);
                self.redraw_if_needed();
            }
            WindowEvent::CursorLeft { .. } => {
                self.ui.pointer_left();
                self.redraw_if_needed();
            }
            WindowEvent::Focused(false) => {
                self.ui.cancel_pointer();
                self.redraw_if_needed();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let button = match button {
                    MouseButton::Left => Button::Primary,
                    MouseButton::Right => Button::Secondary,
                    MouseButton::Middle => Button::Middle,
                    _ => return,
                };
                self.ui
                    .pointer_button(button, state == ElementState::Pressed);
                self.redraw_if_needed();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // Positive toward the list's end: the wheel turned down.
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (-x * LINE, -y * LINE),
                    MouseScrollDelta::PixelDelta(p) => {
                        (-(p.x as f32) / scale, -(p.y as f32) / scale)
                    }
                };
                self.ui.wheel(dx, dy);
                self.redraw_if_needed();
            }
            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Sleep until input, or until a repaint the tree asked for comes;
        // the example's own nodes ask for none, but a painter could.
        let flow = match self.ui.next_repaint() {
            Some(at) => {
                let wait = (at - self.now()).max(0.0);
                ControlFlow::WaitUntil(Instant::now() + std::time::Duration::from_secs_f64(wait))
            }
            None => ControlFlow::Wait,
        };
        event_loop.set_control_flow(flow);
        self.redraw_if_needed();
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App::new();
    event_loop.run_app(&mut app).expect("run app");
}

/// The scene driven as the window drives it, without a window: what each
/// input costs the tree, and that it changes what it should.
#[cfg(test)]
mod tests {
    use super::*;

    fn context(now: f64) -> FrameContext {
        FrameContext {
            now,
            viewport: [640.0, 520.0],
            fonts: 0,
            scale: 1.0,
        }
    }

    /// The middle of `node` in the window, the list not scrolled: its
    /// place in each box above it added up.
    fn centre(ui: &Ui, node: Node) -> (f32, f32) {
        let ([mut x, mut y], [w, h]) = ui.rect(node);
        let mut at = ui.parent(node);
        while let Some(parent) = at {
            let ([px, py], _) = ui.rect(parent);
            (x, y) = (x + px, y + py);
            at = ui.parent(parent);
        }
        (x + w / 2.0, y + h / 2.0)
    }

    /// Whether the last frame drew a text saying `text`.
    fn shows(ui: &Ui, text: &str) -> bool {
        ui.list()
            .expect("a frame")
            .texts
            .iter()
            .any(|block| block.content.as_str() == text)
    }

    fn scene() -> (Ui, Scene) {
        let root = Spec::new()
            .width(Size::Fill(1.0))
            .height(Size::Fill(1.0))
            .stack(Stack::column().gap(16.0).padding(Insets::all(20.0)))
            .fill(palette().page);
        let mut ui = Ui::with_root(wgpu_gameui::shared_font_system(), root);
        let scene = build(&mut ui);
        ui.frame(context(0.0));
        (ui, scene)
    }

    fn click(ui: &mut Ui, node: Node) {
        let (x, y) = centre(ui, node);
        ui.pointer_moved(x, y);
        ui.pointer_button(Button::Primary, true);
        ui.pointer_button(Button::Primary, false);
    }

    /// Hovering the button repaints its box alone, patched where the root's
    /// list holds it; a pointer that stays over it asks for no frame;
    /// leaving it does.
    #[test]
    fn hovering_the_button_repaints_it_alone() {
        let (mut ui, scene) = scene();
        assert!(!ui.needs_frame(0.0));
        let (x, y) = centre(&ui, scene.button);
        ui.pointer_moved(x, y);
        assert!(ui.needs_frame(0.0));
        ui.frame(context(0.1));
        let stats = ui.frame_stats();
        assert_eq!((stats.laid_out, stats.painted, stats.recorded), (0, 1, 0));
        assert!(stats.patched > 0);
        ui.pointer_moved(x + 1.0, y);
        assert!(!ui.needs_frame(0.1), "still over it");
        ui.pointer_left();
        assert!(ui.needs_frame(0.1));
    }

    /// Three clicks count three, and the label says so.
    #[test]
    fn clicking_the_button_counts() {
        let (mut ui, scene) = scene();
        assert!(shows(&ui, "Click me"));
        for _ in 0..3 {
            click(&mut ui, scene.button);
            ui.frame(context(0.1));
        }
        assert_eq!(ui.get(scene.clicks), 3);
        assert!(shows(&ui, "Clicked 3 times"));
        assert!(!shows(&ui, "Click me"));
    }

    /// Picking a row puts down the one picked before and says which; the
    /// wheel scrolls the list, and the rows scrolled into view are drawn in
    /// place of those scrolled out.
    #[test]
    fn rows_are_picked_and_the_list_scrolls() {
        let (mut ui, scene) = scene();
        for row in [2, 5] {
            click(&mut ui, scene.rows[row]);
            ui.frame(context(0.1));
        }
        assert_eq!(ui.get(scene.picked), "Picked row 6");
        assert!(shows(&ui, "Picked row 6"));
        let (x, y) = centre(&ui, scene.list);
        ui.pointer_moved(x, y);
        ui.wheel(0.0, 10_000.0);
        ui.frame(context(0.2));
        assert_eq!(ui.get(ui.scroll_offset(scene.list)), 10_000.0);
        assert!(!shows(&ui, "Row 1 of 1000"));
        assert!(shows(&ui, "Row 350 of 1000"));
    }
}
