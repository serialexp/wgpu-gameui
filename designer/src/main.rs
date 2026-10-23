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
    ClickTracker, CursorIcon, DragTracker, HeadlessGpu, HostEvent, InputState, Key, LayerStack,
    StyleKey, UiRenderer, write_png,
};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use app::{Command, Designer};

/// The windowing system's cursor for the UI's request: the native one closest
/// to the Forge cursor (its CSS keyword, which winit parses).
fn to_winit_cursor(icon: CursorIcon) -> winit::window::CursorIcon {
    icon.fallback().parse().unwrap_or_default()
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

    /// The designer's own shortcuts, on a key going down with the shortcut
    /// key held. Text editing's (select all, copy, cut, paste) are
    /// [`InputState::apply`]'s.
    fn shortcut(&mut self, key: Key) {
        let input = &self.input;
        let Key::Char(c) = key else { return };
        if !input.ctrl_pressed {
            return;
        }
        self.command = match c.to_ascii_lowercase() {
            'n' => Some(Command::New),
            'o' => Some(Command::Open),
            's' if input.shift_pressed => Some(Command::SaveAs),
            's' => Some(Command::Save),
            'q' => Some(Command::Quit),
            _ => return,
        };
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
            WindowEvent::RedrawRequested => self.render(event_loop),
            _ => {
                // The UI lays out in logical pixels.
                let scale = f64::from(self.scale());
                let Some(input) = HostEvent::from_winit(&event, scale) else {
                    return;
                };
                if let HostEvent::Key {
                    key,
                    down: true,
                    repeat: false,
                    ..
                } = input
                {
                    self.shortcut(key);
                }
                self.input.apply(&input);
                self.redraw();
            }
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
