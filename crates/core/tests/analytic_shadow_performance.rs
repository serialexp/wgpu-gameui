//! Structural performance acceptance tests for analytic box shadows.
//!
//! The GPU test checks batching and upload counters rather than timing, making
//! regressions deterministic across adapters. Run it with:
//! `DISPLAY=:0 cargo test --test analytic_shadow_performance -- --ignored --nocapture`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::mem::size_of;

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{AnalyticInstance, BoxShadow, CornerRadii, DrawList, RenderStats, UiRenderer};

const SHADOW_COUNT: usize = 512;
const VIEWPORT: (u32, u32) = (256, 256);
const SHADOW: BoxShadow = BoxShadow {
    offset: [2.0, 3.0],
    blur: 8.0,
    spread: 1.0,
    color: [0.0, 0.0, 0.0, 0.5],
    inset: false,
};
const RADII: CornerRadii = CornerRadii::uniform(4.0);

struct ThreadCountingAllocator;

thread_local! {
    // Counting only the measured thread prevents test-harness and wgpu worker
    // activity on other threads from making this structural assertion flaky.
    static COUNT_ALLOCATIONS: Cell<bool> = const { Cell::new(false) };
    static ALLOCATION_COUNT: Cell<usize> = const { Cell::new(0) };
}

fn record_allocation() {
    COUNT_ALLOCATIONS.with(|enabled| {
        if enabled.get() {
            ALLOCATION_COUNT.with(|count| count.set(count.get() + 1));
        }
    });
}

unsafe impl GlobalAlloc for ThreadCountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: ThreadCountingAllocator = ThreadCountingAllocator;

fn shadow_rect(index: usize) -> Rect {
    let column = index % 32;
    let row = (index / 32) % 16;
    Rect::new(2.0 + column as f32 * 7.0, 2.0 + row as f32 * 13.0, 5.0, 9.0)
}

fn build_contiguous_shadows(list: &mut DrawList) {
    for index in 0..SHADOW_COUNT {
        list.box_shadow_outset(shadow_rect(index), RADII, SHADOW);
    }
}

fn build_alternating_shadow_chrome(list: &mut DrawList) {
    for index in 0..SHADOW_COUNT {
        let rect = shadow_rect(index);
        list.box_shadow_outset(rect, RADII, SHADOW);
        list.chrome_rect(rect, 4.0, 1.0, [0.2, 0.3, 0.4, 1.0], [0.7, 0.8, 0.9, 1.0]);
    }
}

#[test]
fn clear_and_rebuild_after_warmup_allocates_nothing() {
    let mut list = DrawList::new();

    // The warm-up is also the reservation: every vector reaches the exact
    // representative high-water mark through the normal public build path.
    build_alternating_shadow_chrome(&mut list);

    // Initialize this thread's allocator TLS before entering the measured span.
    ALLOCATION_COUNT.with(|count| count.set(0));
    COUNT_ALLOCATIONS.with(|enabled| enabled.set(true));
    list.clear();
    build_alternating_shadow_chrome(&mut list);
    COUNT_ALLOCATIONS.with(|enabled| enabled.set(false));
    let allocations = ALLOCATION_COUNT.with(Cell::get);

    assert_eq!(list.shadow_instance_count(), SHADOW_COUNT);
    assert_eq!(list.chrome_instance_count(), SHADOW_COUNT);
    assert_eq!(
        allocations, 0,
        "DrawList clear + representative shadow/chrome rebuild allocated"
    );
}

struct RenderFixture {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: UiRenderer,
    view: wgpu::TextureView,
}

impl RenderFixture {
    fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
                .ok()?;
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let fonts = wgpu_gameui::shared_font_system();
        let renderer = UiRenderer::new(&device, &queue, format, fonts);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("analytic shadow performance target"),
            size: wgpu::Extent3d {
                width: VIEWPORT.0,
                height: VIEWPORT.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        Some(Self {
            device,
            queue,
            renderer,
            view,
        })
    }

    fn render(&mut self, list: &DrawList) -> RenderStats {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.renderer.begin_frame();
        let stats = self.renderer.render(
            &self.device,
            &self.queue,
            &mut encoder,
            &self.view,
            VIEWPORT,
            1.0,
            list,
        );
        self.queue.submit(Some(encoder.finish()));
        stats
    }
}

#[test]
#[ignore = "requires a GPU adapter (DISPLAY=:0)"]
fn shadow_batching_and_uploads_stay_structurally_bounded() {
    let Some(mut fixture) = RenderFixture::new() else {
        return;
    };

    // An empty render isolates the renderer's per-call uniform write without
    // depending on its private representation size.
    let empty = DrawList::new();
    let baseline = fixture.render(&empty);
    assert_eq!(baseline.buffer_write_calls, 1);

    let mut contiguous = DrawList::new();
    build_contiguous_shadows(&mut contiguous);
    let contiguous_stats = fixture.render(&contiguous);
    assert_eq!(contiguous_stats.shadow_instances, SHADOW_COUNT);
    assert_eq!(contiguous_stats.primitives, SHADOW_COUNT);
    assert_eq!(contiguous_stats.paint_runs, 1);
    assert_eq!(contiguous_stats.draw_calls, 1);
    assert_eq!(contiguous_stats.color_runs, 1);
    assert_eq!(contiguous_stats.render_passes, 1);
    assert_eq!(contiguous_stats.buffer_reallocations, 0);
    assert_eq!(
        contiguous_stats.buffer_write_calls,
        baseline.buffer_write_calls + 1
    );
    assert_eq!(
        contiguous_stats.buffer_bytes_uploaded - baseline.buffer_bytes_uploaded,
        (SHADOW_COUNT * size_of::<AnalyticInstance>()) as u64,
        "the complete contiguous analytic payload must be uploaded exactly once"
    );

    let mut alternating = DrawList::new();
    build_alternating_shadow_chrome(&mut alternating);
    let alternating_stats = fixture.render(&alternating);
    assert_eq!(alternating_stats.shadow_instances, SHADOW_COUNT);
    assert_eq!(alternating_stats.primitives, SHADOW_COUNT * 2);
    assert_eq!(alternating_stats.paint_runs, 1);
    assert_eq!(alternating_stats.draw_calls, 1);
    assert_eq!(alternating_stats.color_runs, 1);
    assert_eq!(alternating_stats.render_passes, 1);
    assert_eq!(alternating_stats.buffer_reallocations, 0);
    assert_eq!(
        alternating_stats.buffer_write_calls,
        baseline.buffer_write_calls + 1
    );

    let uploaded_payload = alternating_stats.buffer_bytes_uploaded - baseline.buffer_bytes_uploaded;
    assert_eq!(
        uploaded_payload,
        (SHADOW_COUNT * 2 * size_of::<AnalyticInstance>()) as u64,
        "alternating chrome/shadow must be one direct heterogeneous upload"
    );
}
