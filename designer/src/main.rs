//! `gameui-designer` — edit a wgpu-gameui theme preset with a live preview.
//!
//! ```text
//! cargo run -p gameui-designer -- [preset.toml]
//! cargo run -p gameui-designer -- [preset.toml] --capture out.png [--size 1440x900] [--select accent]
//! ```
//!
//! `--capture` renders the designer headlessly to a PNG and exits.
//!
//! This file is the window host: winit events become an [`InputState`], each
//! frame [`Designer::frame`] draws into a [`LayerStack`], and the renderer
//! presents it. It redraws only when an event or the UI asks for it.

mod app;
mod fields;
mod preview;
mod seeds;
mod tokens;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    ClickTracker, CursorIcon, DragTracker, HeadlessGpu, InputState, LayerStack, StyleKey,
    UiRenderer, write_png,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use app::{Command, Designer};

/// The windowing system's cursor for the UI's request.
fn to_winit_cursor(icon: CursorIcon) -> winit::window::CursorIcon {
    use winit::window::CursorIcon as W;
    match icon {
        CursorIcon::Default => W::Default,
        CursorIcon::Pointer => W::Pointer,
        CursorIcon::Text => W::Text,
        CursorIcon::Grab => W::Grab,
        CursorIcon::Grabbing => W::Grabbing,
        CursorIcon::ResizeHorizontal => W::EwResize,
        CursorIcon::ResizeVertical => W::NsResize,
        CursorIcon::ResizeDiagonal => W::NwseResize,
        CursorIcon::NotAllowed => W::NotAllowed,
    }
}

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    ui: UiRenderer,
    /// Kept across frames and cleared each one: a fresh stack would re-scan
    /// the system fonts and re-shape every label.
    layers: LayerStack,
}

struct Host {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    designer: Designer,
    input: InputState,
    drag: DragTracker,
    clicks: ClickTracker,
    /// Keyboard shortcut seen since the last frame, handled by the designer.
    command: Option<Command>,
    start: Instant,
    last_frame: Instant,
    /// When the UI next needs a frame without any input (a scroll glide).
    next_deadline: Option<Instant>,
    title: String,
}

impl Host {
    fn scale(&self) -> f32 {
        self.window
            .as_ref()
            .map_or(1.0, |window| window.scale_factor() as f32)
    }

    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn key(&mut self, code: KeyCode, pressed: bool) {
        let input = &mut self.input;
        match code {
            KeyCode::ArrowLeft => input.key_left = pressed,
            KeyCode::ArrowRight => input.key_right = pressed,
            KeyCode::ArrowUp => input.key_up = pressed,
            KeyCode::ArrowDown => input.key_down = pressed,
            KeyCode::Home => input.key_home = pressed,
            KeyCode::End => input.key_end = pressed,
            KeyCode::Delete => input.key_delete = pressed,
            KeyCode::ShiftLeft | KeyCode::ShiftRight => input.shift_pressed = pressed,
            KeyCode::ControlLeft | KeyCode::ControlRight => input.ctrl_pressed = pressed,
            KeyCode::AltLeft | KeyCode::AltRight => {
                input.alt_down = pressed;
                if pressed {
                    input.alt_pressed = true;
                } else {
                    input.alt_released = true;
                }
            }
            _ if !pressed => {}
            KeyCode::Space => input.key_space = true,
            KeyCode::Tab => input.key_tab = true,
            KeyCode::Escape => input.key_escape = true,
            KeyCode::Backspace => input.backspace_pressed = true,
            KeyCode::Enter | KeyCode::NumpadEnter => input.enter_pressed = true,
            KeyCode::KeyA if input.ctrl_pressed => input.key_select_all = true,
            KeyCode::KeyC if input.ctrl_pressed => input.key_copy = true,
            KeyCode::KeyX if input.ctrl_pressed => input.key_cut = true,
            KeyCode::KeyV if input.ctrl_pressed => input.key_paste = true,
            KeyCode::KeyN if input.ctrl_pressed => self.command = Some(Command::New),
            KeyCode::KeyO if input.ctrl_pressed => self.command = Some(Command::Open),
            KeyCode::KeyS if input.ctrl_pressed && input.shift_pressed => {
                self.command = Some(Command::SaveAs)
            }
            KeyCode::KeyS if input.ctrl_pressed => self.command = Some(Command::Save),
            KeyCode::KeyQ if input.ctrl_pressed => self.command = Some(Command::Quit),
            _ => {}
        }
    }

    fn render(&mut self, event_loop: &ActiveEventLoop) {
        let scale = self.scale();
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let Some(window) = self.window.clone() else {
            return;
        };
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

        let t = self.start.elapsed().as_secs_f64();
        self.drag.update(&mut self.input);
        self.clicks.update(&mut self.input, t);
        self.input.frame_dt = self.last_frame.elapsed().as_secs_f32();
        self.last_frame = Instant::now();
        wgpu_gameui::map_keyboard(&mut self.input);

        let screen = Rect::new(
            0.0,
            0.0,
            gpu.config.width as f32 / scale,
            gpu.config.height as f32 / scale,
        );
        gpu.layers.clear();
        let out = self.designer.frame(
            &mut gpu.layers,
            &mut self.input,
            screen,
            self.command.take(),
        );

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("designer encoder"),
            });
        gpu.ui.begin_frame();
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(gpu.ui.clear_color(out.clear).color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        gpu.ui.render_layers(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &view,
            (gpu.config.width, gpu.config.height),
            scale,
            &gpu.layers,
        );
        gpu.queue.submit(Some(encoder.finish()));
        frame.present();
        self.input.end_frame();

        window.set_cursor(to_winit_cursor(out.cursor));
        if out.title != self.title {
            window.set_title(&out.title);
            self.title = out.title;
        }
        if out.quit {
            event_loop.exit();
            return;
        }
        self.next_deadline = out
            .next_frame_in
            .map(|seconds| Instant::now() + Duration::from_secs_f32(seconds.max(0.001)));
        match self.next_deadline {
            Some(when) => event_loop.set_control_flow(ControlFlow::WaitUntil(when)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}

impl ApplicationHandler for Host {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(winit::dpi::LogicalSize::new(1440.0, 900.0));
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
        .expect("no GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("designer device"),
                ..Default::default()
            },
            None,
        ))
        .expect("request device");

        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        // A plain 8-bit surface: the UI draws straight into it and blends the
        // way a browser does, so the preview matches the Forge design pages.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
                )
            })
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
        let fonts = wgpu_gameui::shared_font_system();
        let layers = LayerStack::with_font_system(fonts.clone());
        let mut ui = UiRenderer::new(&device, &queue, format, fonts);
        ui.resize(
            &device,
            &queue,
            config.width,
            config.height,
            window.scale_factor() as f32,
        );
        self.gpu = Some(Gpu {
            surface,
            device,
            queue,
            config,
            ui,
            layers,
        });
        window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.command = Some(Command::Quit);
                self.redraw();
            }
            WindowEvent::Resized(size) => {
                let scale = self.scale();
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.config.width = size.width.max(1);
                    gpu.config.height = size.height.max(1);
                    gpu.surface.configure(&gpu.device, &gpu.config);
                    gpu.ui.resize(
                        &gpu.device,
                        &gpu.queue,
                        gpu.config.width,
                        gpu.config.height,
                        scale,
                    );
                }
                self.redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                // The UI lays out in logical pixels.
                let scale = self.scale();
                self.input.mouse_x = position.x as f32 / scale;
                self.input.mouse_y = position.y as f32 / scale;
                self.redraw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                let input = &mut self.input;
                let (down, clicked, released) = match button {
                    MouseButton::Left => (
                        &mut input.mouse_down,
                        &mut input.mouse_clicked,
                        &mut input.mouse_released,
                    ),
                    MouseButton::Right => (
                        &mut input.mouse_right_down,
                        &mut input.mouse_right_clicked,
                        &mut input.mouse_right_released,
                    ),
                    MouseButton::Middle => (
                        &mut input.mouse_middle_down,
                        &mut input.mouse_middle_clicked,
                        &mut input.mouse_middle_released,
                    ),
                    _ => return,
                };
                if pressed && !*down {
                    *clicked = true;
                } else if !pressed && *down {
                    *released = true;
                }
                *down = pressed;
                self.redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x, y),
                    MouseScrollDelta::PixelDelta(p) => ((p.x / 20.0) as f32, (p.y / 20.0) as f32),
                };
                self.input.scroll_delta += dy;
                self.input.scroll_delta_x += dx;
                self.redraw();
            }
            WindowEvent::KeyboardInput { event: ref key, .. } => {
                let pressed = key.state == ElementState::Pressed;
                if let PhysicalKey::Code(code) = key.physical_key {
                    self.key(code, pressed);
                }
                if pressed
                    && !self.input.ctrl_pressed
                    && let Some(text) = &key.text
                {
                    self.input
                        .text_input
                        .extend(text.chars().filter(|c| !c.is_control()));
                }
                self.redraw();
            }
            WindowEvent::RedrawRequested => self.render(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(when) = self.next_deadline {
            if Instant::now() >= when {
                self.next_deadline = None;
                self.redraw();
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(when));
            }
        }
    }
}

/// The command line: an optional preset, and the capture options.
struct Args {
    preset: Option<PathBuf>,
    /// Render to this PNG without a window, then exit.
    capture: Option<PathBuf>,
    /// The capture's size in pixels.
    size: (u32, u32),
    /// A token to open in the inspector before capturing.
    select: Option<StyleKey>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        preset: None,
        capture: None,
        size: (1440, 900),
        select: None,
    };
    let mut it = std::env::args_os().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |flag: &str| {
            it.next()
                .map(|v| v.to_string_lossy().into_owned())
                .ok_or(format!("{flag} needs a value"))
        };
        match arg.to_str() {
            Some("--capture") => args.capture = Some(PathBuf::from(value("--capture")?)),
            Some("--size") => {
                let size = value("--size")?;
                let (w, h) = size
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    .ok_or(format!("--size wants WIDTHxHEIGHT, not {size}"))?;
                args.size = (w, h);
            }
            Some("--select") => {
                let name = value("--select")?;
                let key = StyleKey::from_name(&name).ok_or(format!("no token named {name}"))?;
                args.select = Some(key);
            }
            Some(flag) if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            _ if args.preset.is_none() => args.preset = Some(PathBuf::from(arg)),
            _ => return Err("only one preset at a time".to_string()),
        }
    }
    Ok(args)
}

/// Draw a few frames headlessly (enough for one-frame-late layouts to
/// settle) and write the last to `out`: a way to look at the designer
/// without a window.
fn capture(mut designer: Designer, out: &std::path::Path, (w, h): (u32, u32)) {
    let mut gpu = HeadlessGpu::new().expect("no GPU adapter for the capture");
    let mut layers = gpu.layer_stack();
    let mut input = InputState::default();
    let screen = Rect::new(0.0, 0.0, w as f32, h as f32);
    let mut clear = [0.0; 4];
    for _ in 0..3 {
        layers.clear();
        wgpu_gameui::map_keyboard(&mut input);
        clear = designer.frame(&mut layers, &mut input, screen, None).clear;
        input.end_frame();
    }
    let background = gpu.clear_color(clear);
    let pixels = gpu.capture_layers(&layers, (w, h), background);
    write_png(out, &pixels, (w, h)).expect("write the capture");
}

fn main() {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("gameui-designer: {error}");
            eprintln!(
                "usage: gameui-designer [preset.toml] [--capture out.png [--size WxH] [--select token]]"
            );
            std::process::exit(2);
        }
    };
    let mut designer = Designer::new(args.preset);
    if let Some(key) = args.select {
        designer.select_token(key);
    }
    if let Some(out) = args.capture {
        capture(designer, &out, args.size);
        return;
    }
    let title = designer.title();
    let mut host = Host {
        window: None,
        gpu: None,
        designer,
        input: InputState::default(),
        drag: DragTracker::new(),
        clicks: ClickTracker::new(),
        command: None,
        start: Instant::now(),
        last_frame: Instant::now(),
        next_deadline: None,
        title,
    };
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.run_app(&mut host).expect("run designer");
}
