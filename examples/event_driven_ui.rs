//! Event-driven host demo — a UI frame whose redraws are scheduled from
//! [`UiFrameResult::next_deadline`] instead of a hot `request_redraw` loop.
//!
//! The interesting part is not the UI (a hover-animated button, a caret
//! indicator the app animates on its own clock) but the frame loop: after an
//! input event triggers one redraw, the app sets
//! `ControlFlow::WaitUntil(deadline)` and sleeps. It wakes exactly when the
//! next UI-visible change is due — a hover fade finishing, the caret toggling
//! — redraws once, and re-arms the next deadline from that frame's result.
//! With no pending animation and no input, the process idles with zero
//! redraws instead of relying on incidental mouse movement.
//!
//! Run with:
//! ```
//! cargo run --example event_driven_ui
//! ```
//!
//! Quit with the button or Escape.

use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use wgpu_gameui::{InputState, KeyboardNav, LayerStack, Theme, UiFrameResult, UiRenderer, UiState};

/// Seconds between caret toggles for the app-drawn indicator.
const CARET_PERIOD: f32 = 0.5;

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    input: InputState,
    theme: Theme,
    state: UiState,
    /// Frame-delta source.
    last_frame: Instant,
    /// App-owned caret phase (toggles every [`CARET_PERIOD`] while shown).
    caret_on: bool,
    /// When the caret toggles next; folded into the frame deadline each frame.
    caret_until: Instant,
    /// Instant at which the UI's next visible change is due, if any.
    next_deadline: Option<Instant>,
    quit_requested: bool,
}

struct Gpu {
    ui: UiRenderer,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl App {
    fn new() -> Self {
        Self {
            window: None,
            gpu: None,
            input: InputState::default(),
            theme: Theme::default(),
            state: UiState::default(),
            last_frame: Instant::now(),
            caret_on: true,
            caret_until: Instant::now(),
            next_deadline: None,
            quit_requested: false,
        }
    }

    /// Advance the app-owned caret clock; returns seconds until its next
    /// toggle so it can be registered with the UI frame.
    fn caret_remaining(&mut self) -> f32 {
        let now = Instant::now();
        if now >= self.caret_until {
            self.caret_on = !self.caret_on;
            self.caret_until = now + Duration::from_secs_f32(CARET_PERIOD);
            CARET_PERIOD
        } else {
            self.caret_until
                .saturating_duration_since(now)
                .as_secs_f32()
        }
    }
}

/// Build one UI frame from the caller's disjoint state fields and return the
/// aggregated result the host uses to schedule its next redraw. A free
/// function over the individual parts (rather than `&mut App`) so a host can
/// call it while the GPU state is still borrowed — the same shape `hello_ui`'s
/// hand-rolled frame takes.
fn build_ui(
    state: &mut UiState,
    input: &mut InputState,
    theme: &Theme,
    last_frame: &mut Instant,
    caret_on: bool,
    caret_remaining: f32,
    quit: &mut bool,
    layers: &mut LayerStack,
) -> UiFrameResult {
    let dt = last_frame.elapsed().as_secs_f32();
    *last_frame = Instant::now();

    wgpu_gameui::map_keyboard(input);
    let (_, frame) = state
        .frame(input, theme, &KeyboardNav)
        .dt(dt)
        .run_layers(layers, |ui| {
            // Fold the app's own timer into the frame's deadline: while this
            // label is visible, the loop wakes every CARET_PERIOD.
            ui.request_repaint_after(caret_remaining);
            ui.push();
            ui.translate(24.0, 28.0);
            ui.text("Event-driven host: idles between deadlines (Esc quits)");
            ui.translate(0.0, 8.0);
            ui.text(if caret_on {
                "app timer: caret ON "
            } else {
                "app timer: caret OFF"
            });
            ui.pop();
            ui.push();
            ui.translate(24.0, 120.0);
            *quit = ui.text_button("Quit (button)", None, None);
            ui.pop();
        });
    frame
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("event_driven_ui — wgpu-gameui")
            .with_inner_size(winit::dpi::LogicalSize::new(520.0, 240.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        self.window = Some(window.clone());

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
                label: Some("event_driven_ui device"),
                ..Default::default()
            },
            None,
        ))
        .expect("request device");

        let size = window.inner_size();
        let surface_caps = surface.get_capabilities(&adapter);
        // An sRGB surface: the UI draws into the renderer's offscreen layer and
        // composites, so blending still matches the browser. (`hello_ui` uses a
        // plain surface to exercise the direct path.)
        let format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let ui = UiRenderer::new(&device, &queue, format, wgpu_gameui::shared_font_system());
        self.gpu = Some(Gpu {
            ui,
            surface,
            config,
            device,
            queue,
        });
        window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // Advance the app caret clock before the `gpu` borrow starts, so the
        // RedrawRequested arm can read both `self` fields and `gpu` freely.
        let caret_remaining = self.caret_remaining();
        let caret_on = self.caret_on;
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let Some(window) = self.window.as_ref() else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                gpu.config.width = size.width.max(1);
                gpu.config.height = size.height.max(1);
                gpu.surface.configure(&gpu.device, &gpu.config);
                window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.input.mouse_x = position.x as f32;
                self.input.mouse_y = position.y as f32;
                window.request_redraw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    let pressed = state == ElementState::Pressed;
                    if pressed && !self.input.mouse_down {
                        self.input.mouse_clicked = true;
                    } else if !pressed && self.input.mouse_down {
                        self.input.mouse_released = true;
                    }
                    self.input.mouse_down = pressed;
                }
                window.request_redraw();
            }
            WindowEvent::KeyboardInput { event: ref ke, .. } => {
                if ke.state == ElementState::Pressed {
                    if let PhysicalKey::Code(KeyCode::Escape) = ke.physical_key {
                        event_loop.exit();
                        return;
                    }
                }
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if self.quit_requested {
                    event_loop.exit();
                    return;
                }
                let frame = match gpu.surface.get_current_texture() {
                    Ok(f) => f,
                    Err(_) => {
                        gpu.surface.configure(&gpu.device, &gpu.config);
                        return;
                    }
                };
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());

                let mut layers = LayerStack::new();
                // Field-level borrows keep the UI build and the GPU render
                // from fighting over `self` (same as `hello_ui`). The caret
                // clock was advanced at the top of `window_event`.
                let mut quit = false;
                let (result, mut encoder) = {
                    let device = &gpu.device;
                    let renderer = &mut gpu.ui;
                    let result = build_ui(
                        &mut self.state,
                        &mut self.input,
                        &self.theme,
                        &mut self.last_frame,
                        caret_on,
                        caret_remaining,
                        &mut quit,
                        &mut layers,
                    );

                    let mut encoder =
                        device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("event_driven_ui encoder"),
                        });
                    renderer.begin_frame();
                    {
                        let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("clear"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(
                                        renderer.clear_color(self.theme.background),
                                    ),
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                        });
                    }
                    (result, encoder)
                };
                gpu.ui.render_layers(
                    &gpu.device,
                    &gpu.queue,
                    &mut encoder,
                    &view,
                    (gpu.config.width, gpu.config.height),
                    window.scale_factor() as f32,
                    &layers,
                );
                gpu.queue.submit(Some(encoder.finish()));
                frame.present();
                self.input.end_frame();
                if quit {
                    self.quit_requested = true;
                }

                // The whole point: schedule the next redraw from the frame's
                // aggregated deadline instead of spinning. `None` means the
                // UI is fully settled — sleep until an input event arrives
                // (each one calls `request_redraw` above).
                self.next_deadline = result
                    .next_deadline
                    .map(|d| Instant::now() + Duration::from_secs_f32(d.max(0.001)));
                match self.next_deadline {
                    Some(when) => event_loop.set_control_flow(ControlFlow::WaitUntil(when)),
                    None => event_loop.set_control_flow(ControlFlow::Wait),
                }
                if self.quit_requested {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // A registered deadline came due: draw the frame that shows the
        // change. (Input events request redraws directly in `window_event`.)
        if let Some(when) = self.next_deadline {
            if Instant::now() >= when {
                self.next_deadline = None;
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(when));
            }
        }
    }
}

fn main() {
    let _ = env_logger::try_init();
    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App::new();
    event_loop.run_app(&mut app).expect("run app");
}
