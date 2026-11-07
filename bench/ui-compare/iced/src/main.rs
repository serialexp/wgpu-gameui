//! iced: the CPU cost of one frame of the shared scene.
//!
//! Timed per frame, as iced's own loop does when the view changes: build the
//! `view` (a column of rows of fixed-size containers, each holding a `text`),
//! `UserInterface::build` (diff against the previous tree, then layout),
//! `UserInterface::draw` (widgets to renderer layers) and `Renderer::present`
//! (text and quad preparation, buffer upload, render pass, submit). The GPU is
//! not waited for. A real iced app would skip the whole thing on a frame where
//! nothing changed; here a frame is forced either way, so `static` shows what
//! a forced redraw costs.

use iced_core::{Background, Border, Color, Length, Padding, Pixels, Size, mouse, renderer};
use iced_graphics::{Shell, Viewport};
use iced_runtime::user_interface::{Cache, UserInterface};
use iced_wgpu::wgpu;
use iced_widget::{column, container, row, text};
use ui_compare_common::*;

type Theme = iced_core::Theme;
type Renderer = iced_wgpu::Renderer;
type Element<'a> = iced_core::Element<'a, (), Theme, Renderer>;

fn main() {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter");
    let info = adapter.get_info();
    eprintln!(
        "iced adapter: {} ({:?}, {:?})",
        info.name, info.device_type, info.backend
    );
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("device");

    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let size = Size::new(WIDTH as u32, HEIGHT as u32);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("target"),
        size: wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let engine = iced_wgpu::Engine::new(
        &adapter,
        device.clone(),
        queue.clone(),
        format,
        None,
        Shell::headless(),
    );
    // The shared typeface, loaded into iced's font system.
    iced_graphics::text::font_system()
        .write()
        .unwrap()
        .load_font(std::borrow::Cow::Borrowed(FONT_BYTES));
    let plex = iced_core::Font::with_name(FONT_FAMILY);
    let mut renderer = Renderer::new(engine, plex, Pixels(12.0));
    let viewport = Viewport::with_physical_size(size, 1.0);
    let bounds = Size::new(WIDTH, HEIGHT);
    let theme = Theme::Dark;
    let style = renderer::Style {
        text_color: Color::WHITE,
    };

    for n in counts() {
        let grid = Grid::new(n);
        let labels: Vec<String> = (0..n).map(label).collect();

        // One frame; returns the user interface's cache for the next one.
        let mut cache = Some(Cache::new());
        let mut frame_fn = |renderer: &mut Renderer, frame: u64, mode: Mode| {
            let element = view_of(&grid, &labels, frame, mode);
            let mut ui = UserInterface::build(element, bounds, cache.take().unwrap(), renderer);
            ui.draw(renderer, &theme, &style, mouse::Cursor::Unavailable);
            cache = Some(ui.into_cache());
            renderer.present(Some(Color::BLACK), format, &view, &viewport);
            let _ = device.poll(wgpu::PollType::Poll);
        };

        // The frame must contain the scene: look at what was drawn.
        frame_fn(&mut renderer, 0, Mode::Animated);
        check_frame("iced", n, &renderer.screenshot(&viewport, Color::BLACK));

        for (scenario, mode) in MODES {
            let stats = measure(|frame| frame_fn(&mut renderer, frame, mode));
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            report("iced", scenario, n, &stats);
        }
    }

    if text_scenes_wanted() {
        let doc: Vec<String> = (0..DOC_LINES).map(doc_line).collect();
        let para = para_text();
        for name in ["scroll", "churn", "para"] {
            if !scene_wanted(name) {
                continue;
            }
            let mut cache = Some(Cache::new());
            let mut frame_fn = |renderer: &mut Renderer, frame: u64| {
                let element = text_view(name, frame, &doc, &para);
                let mut ui = UserInterface::build(element, bounds, cache.take().unwrap(), renderer);
                if name == "para" {
                    // Scroll the paragraph's scrollable to this frame's offset.
                    let offset = 20.0 - para_y(frame);
                    ui.operate(
                        renderer,
                        &mut iced_core::widget::operation::scrollable::scroll_to(
                            PARA_ID.into(),
                            iced_core::widget::operation::scrollable::AbsoluteOffset {
                                x: None,
                                y: Some(offset),
                            },
                        ),
                    );
                }
                ui.draw(renderer, &theme, &style, mouse::Cursor::Unavailable);
                cache = Some(ui.into_cache());
                renderer.present(Some(Color::BLACK), format, &view, &viewport);
                let _ = device.poll(wgpu::PollType::Poll);
            };
            for frame in 0..4 {
                frame_fn(&mut renderer, 40 + frame);
            }
            check_text_frame("iced", name, &renderer.screenshot(&viewport, Color::BLACK));
            let stats = measure(|frame| frame_fn(&mut renderer, frame));
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            report_scene("iced", name, &stats);
        }
    }
}

const PARA_ID: &str = "para";

/// One text scene (see the common crate) as an iced element tree.
fn text_view<'a>(scene: &str, frame: u64, doc: &'a [String], para: &'a str) -> Element<'a> {
    let [r, g, b] = TEXT_BG;
    let bg = Color::from_rgb8(r, g, b);
    let content: Element<'a> =
        if scene == "para" {
            let body = text(para)
                .size(PARA_PX)
                .line_height(Pixels(PARA_LINE_H))
                .width(Length::Fixed(PARA_W));
            container(
                iced_widget::scrollable(body)
                    .id(PARA_ID)
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .padding(Padding {
                left: PARA_X,
                ..Padding::ZERO
            })
            .into()
        } else {
            let mut cols = row![];
            for c in 0..LINE_COLS {
                let mut lines = column![];
                for r in 1..LINE_ROWS {
                    let (i, _) = scrolled_line(c, r, if scene == "scroll" { frame } else { 0 });
                    let line: iced_widget::Text<'a, Theme, Renderer> = if scene == "scroll" {
                        text(doc[i].as_str())
                    } else {
                        text(churn_line(i, frame))
                    };
                    lines = lines.push(
                        line.size(LINE_PX)
                            .line_height(Pixels(LINE_H))
                            .wrapping(iced_core::text::Wrapping::None),
                    );
                }
                // The first row is `scrolled_line(c, 1, ..)`, so its y is the top padding.
                let top = scrolled_line(c, 1, if scene == "scroll" { frame } else { 0 }).1;
                cols = cols.push(container(lines).width(Length::Fixed(LINE_COL_W)).padding(
                    Padding {
                        top,
                        ..Padding::ZERO
                    },
                ));
            }
            cols.into()
        };
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(bg)),
            ..container::Style::default()
        })
        .into()
}

/// The scene as an iced element tree.
fn view_of<'a>(grid: &Grid, labels: &'a [String], frame: u64, mode: Mode) -> Element<'a> {
    let (w, h) = grid.size();
    let radius = grid.radius;
    let mut rows = column![].spacing(GAP);
    for r in 0..grid.n.div_ceil(grid.cols) {
        let mut line = row![].spacing(GAP);
        for c in 0..grid.cols {
            let i = r * grid.cols + c;
            if i >= grid.n {
                break;
            }
            let [red, green, blue] = fill_in(mode, i, frame);
            let color = Color::from_rgb8(red, green, blue);
            let cell = container(text(labels[i].as_str()).size(grid.font_px))
                .width(Length::Fixed(w))
                .height(Length::Fixed(h))
                .style(move |_| container::Style {
                    background: Some(Background::Color(color)),
                    border: Border {
                        radius: radius.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                });
            line = line.push(cell);
        }
        rows = rows.push(line);
    }
    rows.into()
}
