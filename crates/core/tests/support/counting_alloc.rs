//! A global allocator that counts the allocations made on one thread while
//! asked to, shared by the tests that check a path allocates nothing (or a
//! bounded amount). Included with `#[path]` by the integration tests that
//! need it and by the library's own tests, so each test binary installs it
//! once.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct ThreadCountingAllocator;

thread_local! {
    // Only the measured thread counts, so the test harness, wgpu's worker
    // threads and other tests running in parallel can't make an assertion
    // flaky.
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

/// Run `f`, and count the allocations (and reallocations) it made on this
/// thread.
pub(crate) fn count_allocations<T>(f: impl FnOnce() -> T) -> (T, usize) {
    // Touching both before counting initialises this thread's slots outside
    // the measured span.
    ALLOCATION_COUNT.with(|count| count.set(0));
    COUNT_ALLOCATIONS.with(|enabled| enabled.set(true));
    let value = f();
    COUNT_ALLOCATIONS.with(|enabled| enabled.set(false));
    (value, ALLOCATION_COUNT.with(Cell::get))
}
