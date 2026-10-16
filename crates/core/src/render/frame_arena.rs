//! A per-frame bump arena in one growing GPU buffer: the renderer's vertex,
//! index and instance scratch.
//!
//! Each write takes the bytes after the frame's earlier writes, so the passes
//! of one submission read their own data rather than whatever was written
//! last (`Queue::write_buffer` lands at submit, before any pass runs; see
//! `uniform_arena`). [`reset`](FrameArena::reset) starts the next frame from
//! zero.

use crate::RenderStats;

pub(crate) struct FrameArena {
    label: &'static str,
    usage: wgpu::BufferUsages,
    buffer: wgpu::Buffer,
    capacity: u64,
    /// Bytes written this frame.
    offset: u64,
}

impl FrameArena {
    /// An arena of `capacity` bytes to start with, for `usage` (`COPY_DST` is
    /// added).
    pub(crate) fn new(
        device: &wgpu::Device,
        label: &'static str,
        usage: wgpu::BufferUsages,
        capacity: u64,
    ) -> Self {
        let usage = usage | wgpu::BufferUsages::COPY_DST;
        Self {
            label,
            usage,
            buffer: create(device, label, usage, capacity),
            capacity,
            offset: 0,
        }
    }

    /// The buffer this frame's latest writes went into.
    pub(crate) fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// Bytes written this frame.
    pub(crate) fn used(&self) -> u64 {
        self.offset
    }

    /// Start the next frame's writes from the beginning.
    pub(crate) fn reset(&mut self) {
        self.offset = 0;
    }

    /// Write `data` after this frame's earlier writes, and count the write in
    /// `stats`. Returns the byte offset it went to.
    ///
    /// Grows by replacing the buffer with one twice the size needed. The old
    /// one goes into `retired`, which keeps it alive for the renderer's
    /// lifetime: passes encoded earlier in the frame still read it, and work
    /// can outlive the frame it was submitted in.
    pub(crate) fn write(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &[u8],
        retired: &mut Vec<wgpu::Buffer>,
        stats: &mut RenderStats,
    ) -> u64 {
        let offset = self.offset;
        let needed = offset + data.len() as u64;
        if needed > self.capacity {
            stats.buffer_reallocations += 1;
            self.capacity = needed.next_power_of_two();
            let replacement = create(device, self.label, self.usage, self.capacity);
            retired.push(std::mem::replace(&mut self.buffer, replacement));
        }
        queue.write_buffer(&self.buffer, offset, data);
        stats.buffer_write_calls += 1;
        stats.buffer_bytes_uploaded += data.len() as u64;
        self.offset = needed;
        offset
    }
}

fn create(
    device: &wgpu::Device,
    label: &str,
    usage: wgpu::BufferUsages,
    size: u64,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    })
}
