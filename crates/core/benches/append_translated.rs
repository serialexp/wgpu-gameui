//! What `DrawList::append_translated` costs per record placed: a list
//! recorded at its own origin, placed again and again at whole-pixel offsets
//! into a list cleared each time, as a retained tree records a list from
//! its pieces. The pieces hold 1, 16 or 256 chrome rects (analytic records,
//! one paint run each), and every case places 16,384 records, so the cases
//! differ in what is paid per piece and per record.
//!
//! ```
//! cargo bench -p wgpu-gameui --bench append_translated
//! ```

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{DrawList, shared_font_system};

/// Records placed per iteration, whatever the piece's size.
const RECORDS: usize = 16_384;
const PIECE_RECORDS: [usize; 3] = [1, 16, 256];

/// A piece of `records` chrome rects in a column, at its own origin.
fn piece(records: usize) -> DrawList {
    let mut list = DrawList::with_font_system(shared_font_system());
    for i in 0..records {
        list.chrome_rect(
            Rect::new(0.0, 12.0 * i as f32, 16.0, 10.0),
            2.0,
            1.0,
            [0.2, 0.3, 0.5, 1.0],
            [0.8; 4],
        );
    }
    list
}

fn append_translated(c: &mut Criterion) {
    let mut group = c.benchmark_group("append_translated");
    group.throughput(Throughput::Elements(RECORDS as u64));
    for records in PIECE_RECORDS {
        let piece = piece(records);
        let pieces = RECORDS / records;
        let mut target = DrawList::with_font_system(shared_font_system());
        group.bench_with_input(
            BenchmarkId::from_parameter(records),
            &pieces,
            |b, &pieces| {
                b.iter(|| {
                    target.clear();
                    for i in 0..pieces {
                        let offset = [18.0 * (i % 100) as f32, 12.0 * (i / 100) as f32];
                        std::hint::black_box(target.append_translated(&piece, offset, None));
                    }
                    std::hint::black_box(target.prim_counts().total())
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, append_translated);
criterion_main!(benches);
