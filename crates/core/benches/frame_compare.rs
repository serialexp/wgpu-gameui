//! What it costs to ask "did anything change since last frame?"
//! (`docs/design/frame-reuse.md`, Phase 0), next to what recomputing costs.
//!
//! Frame-to-frame reuse only pays where comparing two lists is much cheaper
//! than the stages it would skip, and the doc's budget says an all-changed
//! frame must stay within 5% of today. Nothing compares frames yet, so this
//! measures a stand-in with the work a real comparison has to do per record:
//!
//! - the chrome records, as bytes (`AnalyticInstance` is plain data);
//! - each text block's fields that decide its layout and where it draws, with
//!   its shared lists by identity.
//!
//! It is a lower bound: the real comparison also tells which stage's inputs
//! changed, and has to see spans and sprite bindings. `recompute` is the
//! renderer's CPU cost for the same list (build excluded, GPU not waited for),
//! the work a fully reused frame would skip.
//!
//! ```
//! DISPLAY=:0 cargo bench --bench frame_compare
//! ```

use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{DrawList, HeadlessGpu, TextBlock, TextContent};

const SIZE: (u32, u32) = (1920, 1080);
const COUNTS: [usize; 3] = [100, 1_000, 10_000];

/// One list: a cell and a label per `i`, laid out in rows. `shade` colours the
/// cell, `nudge` moves cell `0`'s label.
fn build(
    gpu: &HeadlessGpu,
    labels: &[TextContent],
    shade: f32,
    nudge: f32,
    shift: f32,
) -> DrawList {
    let mut list = gpu.draw_list();
    let columns = 100;
    for (i, label) in labels.iter().enumerate() {
        let (x, y) = (
            4.0 + shift + 18.0 * (i % columns) as f32,
            4.0 + 12.0 * (i / columns) as f32,
        );
        list.chrome_rect(
            Rect::new(x, y, 16.0, 10.0),
            2.0,
            1.0,
            [shade, 0.3, 0.5, 1.0],
            [0.8; 4],
        );
        let nudge = if i == 0 { nudge } else { 0.0 };
        list.text(TextBlock::new(label.clone(), x + 1.0 + nudge, y + 1.0).with_size(8.0));
    }
    list
}

/// The comparison a reuse would make per record, as far as it can be written
/// before there is one: whether `b` is `a` again.
fn same(a: &DrawList, b: &DrawList) -> bool {
    if a.analytic_instances.len() != b.analytic_instances.len() || a.texts.len() != b.texts.len() {
        return false;
    }
    if bytemuck::cast_slice::<_, u8>(&a.analytic_instances)
        != bytemuck::cast_slice::<_, u8>(&b.analytic_instances)
    {
        return false;
    }
    a.texts.iter().zip(&b.texts).all(|(p, q)| {
        p.content == q.content
            && p.x == q.x
            && p.y == q.y
            && p.font_size == q.font_size
            && p.line_height == q.line_height
            && p.max_width == q.max_width
            && p.letter_spacing == q.letter_spacing
            && p.color == q.color
            && p.clip == q.clip
            && p.align == q.align
            && p.weight == q.weight
            && p.style == q.style
            && p.spans.is_empty() == q.spans.is_empty()
            && Arc::ptr_eq(&p.style_ranges, &q.style_ranges)
            && Arc::ptr_eq(&p.face_ranges, &q.face_ranges)
    })
}

fn bench(c: &mut Criterion) {
    let Some(mut gpu) = HeadlessGpu::new() else {
        eprintln!("no GPU adapter: the recompute reference can't run");
        return;
    };
    let device = gpu.device().clone();
    let queue = gpu.queue().clone();
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("frame_compare target"),
        size: wgpu::Extent3d {
            width: SIZE.0,
            height: SIZE.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu_gameui::render::CAPTURE_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let mut group = c.benchmark_group("frame_compare");
    for n in COUNTS {
        let labels: Vec<TextContent> = (0..n).map(|i| format!("{}", i % 97).into()).collect();
        let before = build(&gpu, &labels, 0.4, 0.0, 0.0);
        let identical = build(&gpu, &labels, 0.4, 0.0, 0.0);
        let colour_only = build(&gpu, &labels, 0.6, 0.0, 0.0);
        let one_moved = build(&gpu, &labels, 0.4, 2.0, 0.0);
        let all_moved = build(&gpu, &labels, 0.4, 0.0, 1.0);
        assert!(same(&before, &identical));
        assert!(!same(&before, &colour_only));
        assert!(!same(&before, &one_moved));

        // The comparison stops at the first difference, so a list whose
        // difference comes last is the full cost, and one whose difference
        // comes first the cheapest an unchanged-looking frame can be.
        group.bench_with_input(BenchmarkId::new("compare_identical", n), &n, |b, _| {
            b.iter(|| std::hint::black_box(same(&before, &identical)))
        });
        group.bench_with_input(BenchmarkId::new("compare_first_differs", n), &n, |b, _| {
            b.iter(|| std::hint::black_box(same(&before, &colour_only)))
        });
        group.bench_with_input(BenchmarkId::new("compare_text_moved", n), &n, |b, _| {
            b.iter(|| std::hint::black_box(same(&before, &one_moved)))
        });
        // What a reused frame would skip.
        group.bench_with_input(BenchmarkId::new("recompute", n), &n, |b, _| {
            let ui = gpu.renderer();
            ui.set_frame_comparison(false);
            b.iter(|| {
                ui.begin_frame();
                let mut encoder = device.create_command_encoder(&Default::default());
                ui.render(&device, &queue, &mut encoder, &view, SIZE, 1.0, &before);
                queue.submit(Some(encoder.finish()));
                device.poll(wgpu::Maintain::Poll);
            })
        });
        // The same alternation with the comparison off: what the frames cost
        // by themselves, for the `with_comparison_*` runs to be read against.
        group.bench_with_input(BenchmarkId::new("recompute_alternating", n), &n, |b, _| {
            let ui = gpu.renderer();
            ui.set_frame_comparison(false);
            let mut flip = false;
            b.iter(|| {
                flip = !flip;
                let list = if flip { &all_moved } else { &before };
                ui.begin_frame();
                let mut encoder = device.create_command_encoder(&Default::default());
                ui.render(&device, &queue, &mut encoder, &view, SIZE, 1.0, list);
                queue.submit(Some(encoder.finish()));
                device.poll(wgpu::Maintain::Poll);
            })
        });
        device.poll(wgpu::Maintain::Wait);
        // And with one text block moving, which is where boxes are reused.
        group.bench_with_input(
            BenchmarkId::new("recompute_alternating_one_moved", n),
            &n,
            |b, _| {
                let ui = gpu.renderer();
                ui.set_frame_comparison(false);
                let mut flip = false;
                b.iter(|| {
                    flip = !flip;
                    let list = if flip { &one_moved } else { &before };
                    ui.begin_frame();
                    let mut encoder = device.create_command_encoder(&Default::default());
                    ui.render(&device, &queue, &mut encoder, &view, SIZE, 1.0, list);
                    queue.submit(Some(encoder.finish()));
                    device.poll(wgpu::Maintain::Poll);
                })
            },
        );
        device.poll(wgpu::Maintain::Wait);
        // The renderer with its frame comparison on, which compares the list
        // with the last one, keeps it, and (until a later phase consumes the
        // result) recomputes regardless. The extra over `recompute` is the
        // comparison's whole cost; the doc's budget is 5% when all changed.
        for (name, next) in [
            ("with_comparison_identical", &identical),
            ("with_comparison_colour_only", &colour_only),
            ("with_comparison_all_moved", &all_moved),
            ("with_comparison_one_moved", &one_moved),
        ] {
            group.bench_with_input(BenchmarkId::new(name, n), &n, |b, _| {
                let ui = gpu.renderer();
                ui.set_frame_comparison(true);
                // Alternate with `before`, so every frame differs from the
                // last the way `next` does from `before` (and back).
                let mut flip = false;
                b.iter(|| {
                    flip = !flip;
                    let list = if flip { next } else { &before };
                    ui.begin_frame();
                    let mut encoder = device.create_command_encoder(&Default::default());
                    ui.render(&device, &queue, &mut encoder, &view, SIZE, 1.0, list);
                    queue.submit(Some(encoder.finish()));
                    device.poll(wgpu::Maintain::Poll);
                });
                ui.set_frame_comparison(false);
            });
            device.poll(wgpu::Maintain::Wait);
        }
        device.poll(wgpu::Maintain::Wait);
    }
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
