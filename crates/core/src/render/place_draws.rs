//! A place's draws kept on the GPU from frame to frame
//! (`docs/design/frame-reuse.md`, Phase 5): its records in draw order, a copy
//! of them on the CPU, where each of the list's records went in them, and a
//! buffer per family that holds the same bytes on the GPU.
//!
//! A list drawn from the plan kept for its place (every record where it was)
//! is not filled and uploaded whole again: only the records whose look
//! changed are written, into the copy and then into the buffers, in runs of
//! neighbouring records. Any other list fills its place from nothing and
//! uploads every family whole.
//!
//! A place is written once per frame (the renderer gives each place to one
//! list between `begin_frame`s, and a frame is one submission), and
//! `Queue::write_buffer` takes effect before the submission that follows it,
//! after the ones before: so what a frame writes is what that frame draws,
//! and the frame before read what it wrote.

use std::mem::size_of;
use std::ops::Range;

use super::batch_draws::{BatchDraws, NOT_DRAWN, Positions};
use super::draw_plan::DrawPlan;
use super::frame_diff::{Change, Changes};
use super::ui_renderer::{icon_instance, nine_slice_instance};
use crate::RenderStats;
use crate::text::MsdfPrepared;
use crate::widgets::DrawList;

/// Records closer than this many bytes apart are written in one run, the
/// bytes between them included: a write call costs more than a few
/// kilobytes copied.
const GAP_BYTES: usize = 4096;

/// A GPU buffer kept from frame to frame, grown when what it holds outgrows it.
pub(crate) struct Retained {
    label: &'static str,
    usage: wgpu::BufferUsages,
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
}

impl Retained {
    fn new(label: &'static str, usage: wgpu::BufferUsages) -> Self {
        Self {
            label,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            buffer: None,
            capacity: 0,
        }
    }

    /// The buffer, once anything was written.
    pub(crate) fn buffer(&self) -> Option<&wgpu::Buffer> {
        self.buffer.as_ref()
    }

    /// Bytes reserved on the GPU.
    fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Write `data` from the start, growing the buffer to the next power of
    /// two (at most the device's largest buffer) when it doesn't fit. The
    /// buffer replaced is dropped: a frame writes a place once, and work
    /// already submitted keeps what it reads alive.
    fn write_all(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &[u8],
        stats: &mut RenderStats,
    ) {
        if data.is_empty() {
            return;
        }
        let needed = data.len() as u64;
        if needed > self.capacity || self.buffer.is_none() {
            if self.buffer.is_some() {
                stats.buffer_reallocations += 1;
            }
            let largest = device.limits().max_buffer_size;
            self.capacity = needed.next_power_of_two().min(largest).max(needed);
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: self.capacity,
                usage: self.usage,
                mapped_at_creation: false,
            }));
        }
        self.write_at(queue, 0, data, stats);
    }

    /// Write `data` at byte `offset` of what was written whole before.
    fn write_at(&self, queue: &wgpu::Queue, offset: u64, data: &[u8], stats: &mut RenderStats) {
        let buffer = self
            .buffer
            .as_ref()
            .expect("a place's records are written whole before any is rewritten");
        debug_assert!(offset + data.len() as u64 <= self.capacity);
        queue.write_buffer(buffer, offset, data);
        stats.buffer_write_calls += 1;
        stats.buffer_bytes_uploaded += data.len() as u64;
    }
}

/// A place's GPU buffers, one per family of [`BatchDraws`], and the soup's
/// vertices as the list has them.
pub(crate) struct PlaceBuffers {
    pub(crate) vertices: Retained,
    pub(crate) indices: Retained,
    pub(crate) analytic: Retained,
    pub(crate) circles: Retained,
    pub(crate) sprites: Retained,
    pub(crate) nine_slices: Retained,
    pub(crate) text: Retained,
    #[cfg(feature = "phosphor-icons")]
    pub(crate) icons: Retained,
}

impl Default for PlaceBuffers {
    fn default() -> Self {
        use wgpu::BufferUsages as U;
        Self {
            vertices: Retained::new("place soup vertices", U::VERTEX),
            indices: Retained::new("place soup indices", U::INDEX),
            analytic: Retained::new("place analytic instances", U::VERTEX),
            circles: Retained::new("place circle instances", U::VERTEX),
            sprites: Retained::new("place sprite instances", U::VERTEX),
            nine_slices: Retained::new("place nine-slice instances", U::VERTEX),
            text: Retained::new("place text instances", U::VERTEX),
            #[cfg(feature = "phosphor-icons")]
            icons: Retained::new("place icon instances", U::VERTEX),
        }
    }
}

impl PlaceBuffers {
    fn capacity(&self) -> u64 {
        let icons = {
            #[cfg(feature = "phosphor-icons")]
            {
                self.icons.capacity()
            }
            #[cfg(not(feature = "phosphor-icons"))]
            0
        };
        self.vertices.capacity()
            + self.indices.capacity()
            + self.analytic.capacity()
            + self.circles.capacity()
            + self.sprites.capacity()
            + self.nine_slices.capacity()
            + self.text.capacity()
            + icons
    }
}

/// Positions of records to write again, per family.
#[derive(Default)]
struct Dirty {
    analytic: Vec<Range<u32>>,
    circles: Vec<Range<u32>>,
    sprites: Vec<Range<u32>>,
    nine_slices: Vec<Range<u32>>,
    text: Vec<Range<u32>>,
    #[cfg(feature = "phosphor-icons")]
    icons: Vec<Range<u32>>,
    /// The soup's vertices, written whole.
    vertices: bool,
}

/// One place's draws, kept from frame to frame.
#[derive(Default)]
pub(crate) struct PlaceDraws {
    /// The records as the GPU buffers hold them, and the draws over them.
    pub(crate) draws: BatchDraws,
    /// Where each of the last list's records went in `draws`.
    pos: Positions,
    pub(crate) gpu: PlaceBuffers,
    /// `draws` and `gpu` hold the last list drawn in this place, from the plan
    /// kept for it: its records can be rewritten in place.
    pub(crate) current: bool,
    /// Drawn in since the last `begin_frame`.
    pub(crate) used: bool,
    dirty: Dirty,
}

impl PlaceDraws {
    /// Bytes held, on the CPU and the GPU, about.
    pub(crate) fn bytes(&self) -> usize {
        self.draws.bytes() + self.pos.bytes() + self.gpu.capacity() as usize
    }

    /// Fill the place from nothing with `plan`'s units, planned from `list`
    /// with its text and icons `prepared`, and soup `tail` after them; and
    /// upload every family whole.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fill(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        plan: &DrawPlan,
        list: &DrawList,
        prepared: &MsdfPrepared,
        tail: &[u32],
        stats: &mut RenderStats,
    ) {
        self.draws.fill(plan, list, prepared, Some(&mut self.pos));
        self.draws.push_soup(tail);
        let (draws, gpu) = (&self.draws, &mut self.gpu);
        let mut all = |buffer: &mut Retained, data: &[u8]| {
            buffer.write_all(device, queue, data, stats);
        };
        if !draws.indices.is_empty() {
            all(&mut gpu.vertices, bytemuck::cast_slice(&list.vertices));
            all(&mut gpu.indices, bytemuck::cast_slice(&draws.indices));
        }
        all(&mut gpu.analytic, bytemuck::cast_slice(&draws.analytic));
        all(&mut gpu.circles, bytemuck::cast_slice(&draws.circles));
        all(&mut gpu.sprites, bytemuck::cast_slice(&draws.sprites));
        let nine_slices = bytemuck::cast_slice(&draws.nine_slices);
        all(&mut gpu.nine_slices, nine_slices);
        all(&mut gpu.text, bytemuck::cast_slice(&draws.text));
        #[cfg(feature = "phosphor-icons")]
        all(&mut gpu.icons, bytemuck::cast_slice(&draws.icons));
        self.current = true;
    }

    /// Bring the place up to `list`, which keeps every record's place against
    /// the list last drawn here (`changes` says how each record differs from
    /// it) and is drawn from the same plan, with its text refreshed in place
    /// (`prepared`): write again only the records whose look changed, and
    /// upload them. The records rewritten, or `None`, with nothing uploaded,
    /// when the place cannot be brought up so (an icon drawing a different
    /// number of instances): fill it instead.
    pub(crate) fn rewrite(
        &mut self,
        queue: &wgpu::Queue,
        plan: &DrawPlan,
        list: &DrawList,
        prepared: &MsdfPrepared,
        changes: &Changes,
        stats: &mut RenderStats,
    ) -> Option<usize> {
        debug_assert!(self.current, "only a current place is rewritten");
        let mut rewritten = 0;
        // Icons are prepared again every frame, so they are compared rather
        // than told; first, so a count that moved writes nothing.
        #[cfg(feature = "phosphor-icons")]
        {
            self.dirty.icons.clear();
            for (i, &[at, n]) in self.pos.icons.iter().enumerate() {
                if at == NOT_DRAWN {
                    continue;
                }
                let range = prepared.icon_range(i as u32..i as u32 + 1);
                if range.len() != n as usize {
                    return None;
                }
                let new = &prepared.icon_instances[range.start as usize..range.end as usize];
                let old = &mut self.draws.icons[at as usize..(at + n) as usize];
                if bytemuck::cast_slice::<_, u8>(old) != bytemuck::cast_slice::<_, u8>(new) {
                    old.copy_from_slice(new);
                    self.dirty.icons.push(at..at + n);
                    rewritten += 1;
                }
            }
        }
        let draws = &mut self.draws;
        let pos = &self.pos;
        let dirty = &mut self.dirty;

        dirty.analytic.clear();
        for i in looks(&changes.analytic) {
            let at = pos.analytic[i];
            if at != NOT_DRAWN {
                draws.analytic[at as usize] = list.analytic_instances[i];
                dirty.analytic.push(at..at + 1);
                rewritten += 1;
            }
        }
        dirty.circles.clear();
        for i in looks(&changes.circles) {
            let at = pos.circles[i];
            if at != NOT_DRAWN {
                draws.circles[at as usize] = list.circle_instances[i];
                dirty.circles.push(at..at + 1);
                rewritten += 1;
            }
        }
        dirty.sprites.clear();
        for i in looks(&changes.icons) {
            let [at, entry] = pos.sprites[i];
            if at != NOT_DRAWN {
                let textured = plan.textured(entry);
                draws.sprites[at as usize] = icon_instance(&list.icons[i], &textured.at);
                dirty.sprites.push(at..at + 1);
                rewritten += 1;
            }
        }
        dirty.nine_slices.clear();
        for i in looks(&changes.nine_slices) {
            let [at, entry] = pos.nine_slices[i];
            if at != NOT_DRAWN {
                let textured = plan.textured(entry);
                draws.nine_slices[at as usize] =
                    nine_slice_instance(&list.nine_slices[i], &textured.at, textured.border);
                dirty.nine_slices.push(at..at + 1);
                rewritten += 1;
            }
        }
        dirty.text.clear();
        for b in looks(&changes.texts) {
            let unit = pos.block_unit[b];
            let Some(shift) = pos.text_shift.get(unit as usize).copied().flatten() else {
                continue;
            };
            for sweep in &prepared.block_sweeps[b] {
                if sweep.is_empty() {
                    continue;
                }
                let at = (i64::from(sweep.start) + shift) as u32;
                let end = at + sweep.len() as u32;
                draws.text[at as usize..end as usize].copy_from_slice(
                    &prepared.text_instances[sweep.start as usize..sweep.end as usize],
                );
                dirty.text.push(at..end);
            }
            rewritten += 1;
        }
        // A soup vertex that changed colour: the soup's vertices are written
        // whole, as the list has them, when a soup draws.
        let soup = changes.soup.iter().any(|c| *c != Change::Same) || changes.tail != Change::Same;
        dirty.vertices = soup && !draws.indices.is_empty();
        rewritten += usize::from(dirty.vertices);

        let gpu = &self.gpu;
        let mut out = (queue, stats);
        flush(
            &mut dirty.analytic,
            &draws.analytic,
            &gpu.analytic,
            &mut out,
        );
        flush(&mut dirty.circles, &draws.circles, &gpu.circles, &mut out);
        flush(&mut dirty.sprites, &draws.sprites, &gpu.sprites, &mut out);
        flush(
            &mut dirty.nine_slices,
            &draws.nine_slices,
            &gpu.nine_slices,
            &mut out,
        );
        flush(&mut dirty.text, &draws.text, &gpu.text, &mut out);
        #[cfg(feature = "phosphor-icons")]
        flush(&mut dirty.icons, &draws.icons, &gpu.icons, &mut out);
        let (queue, stats) = out;
        if dirty.vertices {
            gpu.vertices
                .write_at(queue, 0, bytemuck::cast_slice(&list.vertices), stats);
        }
        Some(rewritten)
    }
}

/// The records `changes` calls [`Change::Look`].
fn looks(changes: &[Change]) -> impl Iterator<Item = usize> + '_ {
    changes
        .iter()
        .enumerate()
        .filter(|(_, c)| **c == Change::Look)
        .map(|(i, _)| i)
}

/// Write the records at positions `dirty` of `data`, which `gpu` holds, in
/// runs: neighbours closer than [`GAP_BYTES`] share one, and the family is
/// written whole when the runs come to more than half of it.
fn flush<T: bytemuck::Pod>(
    dirty: &mut [Range<u32>],
    data: &[T],
    gpu: &Retained,
    (queue, stats): &mut (&wgpu::Queue, &mut RenderStats),
) {
    if dirty.is_empty() {
        return;
    }
    let size = size_of::<T>();
    let gap = (GAP_BYTES / size.max(1)) as u32;
    dirty.sort_unstable_by_key(|r| r.start);
    // Merge into the front of `dirty`.
    let mut runs = 0;
    for i in 0..dirty.len() {
        let next = dirty[i].clone();
        if runs > 0 && next.start <= dirty[runs - 1].end + gap {
            let last = &mut dirty[runs - 1];
            last.end = last.end.max(next.end);
        } else {
            dirty[runs] = next;
            runs += 1;
        }
    }
    let runs = &dirty[..runs];
    let covered: usize = runs.iter().map(|r| r.len()).sum();
    if covered * 2 > data.len() {
        gpu.write_at(queue, 0, bytemuck::cast_slice(data), stats);
        return;
    }
    for run in runs {
        let bytes = &data[run.start as usize..run.end as usize];
        let offset = run.start as u64 * size as u64;
        gpu.write_at(queue, offset, bytemuck::cast_slice(bytes), stats);
    }
}
