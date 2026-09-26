//! Scale acceptance tests for [`GroupList`].
//!
//! A sidebar can hold every session of every project, so the list must cost
//! the same per frame whether it holds a hundred rows or ten thousand. These
//! tests pin that down three ways:
//!
//! - a frame's heap allocations don't depend on the total row count, only on
//!   what is visible;
//! - in release builds, one frame's layout and paint of 10,000 rows in 300
//!   groups stays under 1 ms of CPU time;
//! - rebuilding the row offsets for those rows stays under 2 ms.
//!
//! Timing budgets are only asserted in release builds (`cargo test --release
//! --test group_list_performance`); debug builds still check allocations.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::time::{Duration, Instant};

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    DrawList, GroupHeader, GroupItem, GroupLayout, GroupList, GroupListState, GroupMore, GroupRow,
    GroupRowKind, Ink, InputState, Status, StyleResolver, TextSize, Theme, Thumb,
};

const GROUPS: usize = 300;
const TOTAL_ROWS: usize = 10_000;
const VIEW: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 272.0,
    height: 720.0,
};
const FRAME_BUDGET: Duration = Duration::from_millis(1);
const REBUILD_BUDGET: Duration = Duration::from_millis(2);

struct ThreadCountingAllocator;

thread_local! {
    // Only the measured thread counts, so the test harness and other tests
    // running in parallel can't make the assertion flaky.
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

fn count_allocations<T>(f: impl FnOnce() -> T) -> (T, usize) {
    ALLOCATION_COUNT.with(|count| count.set(0));
    COUNT_ALLOCATIONS.with(|enabled| enabled.set(true));
    let value = f();
    COUNT_ALLOCATIONS.with(|enabled| enabled.set(false));
    (value, ALLOCATION_COUNT.with(Cell::get))
}

/// The caller's side of a sidebar: owned strings the rows borrow.
enum Source {
    Header {
        name: String,
        count: String,
    },
    Item {
        title: String,
        sub: String,
        meta: String,
        hue: f32,
    },
    More {
        label: String,
        meta: String,
    },
}

impl Source {
    fn kind(&self) -> GroupRowKind {
        match self {
            Source::Header { .. } => GroupRowKind::Header,
            Source::Item { .. } => GroupRowKind::Item,
            Source::More { .. } => GroupRowKind::More,
        }
    }

    fn row(&self, i: usize) -> GroupRow<'_> {
        match self {
            Source::Header { name, count } => GroupRow::Header(
                GroupHeader::new(name)
                    .count(count)
                    .thumb(Thumb::new().name(name)),
            ),
            Source::Item {
                title,
                sub,
                meta,
                hue,
            } => GroupRow::Item(
                GroupItem::new(title)
                    .subtitle(sub)
                    .chip("claude opus", *hue)
                    .meta(meta)
                    .status(if i.is_multiple_of(7) {
                        Status::Running
                    } else {
                        Status::Idle
                    })
                    .dim(i.is_multiple_of(3))
                    .menu(true),
            ),
            Source::More { label, meta } => GroupRow::More(GroupMore::new(label, meta)),
        }
    }
}

/// `groups` projects of equal size, `total` rows in all: a header, items,
/// and a trailing "N older" row per project.
fn sidebar(groups: usize, total: usize) -> Vec<Source> {
    let per_group = total / groups;
    let mut rows = Vec::with_capacity(total);
    for g in 0..groups {
        rows.push(Source::Header {
            name: format!("project-{g:03}"),
            count: format!("{} / {}", per_group / 4, per_group),
        });
        for s in 0..per_group.saturating_sub(2) {
            rows.push(Source::Item {
                title: format!("Session {s} of project {g}: a title long enough to clip"),
                sub: format!("@warm-otter · {} msgs · 2 comp", 100 + s),
                meta: format!("{}m", s + 1),
                hue: (g * 37 % 360) as f32,
            });
        }
        rows.push(Source::More {
            label: "4 older".into(),
            meta: "3d – 8d".into(),
        });
    }
    while rows.len() < total {
        rows.push(Source::Item {
            title: "Filler session".into(),
            sub: "@calm-fox · 9 msgs".into(),
            meta: "9d".into(),
            hue: 160.0,
        });
    }
    rows
}

struct Harness {
    rows: Vec<Source>,
    layout: GroupLayout,
    state: GroupListState,
    list: DrawList,
    theme: Theme,
}

impl Harness {
    fn new(rows: Vec<Source>) -> Self {
        let layout = GroupLayout::from_kinds(rows.iter().map(Source::kind));
        Self {
            rows,
            layout,
            state: GroupListState::new(),
            list: DrawList::new(),
            theme: Theme::default(),
        }
    }

    /// One frame, with the pointer over a row in the middle of the view (so
    /// the hover `⋯` key is drawn as well).
    fn frame(&mut self) {
        self.list.clear();
        let mut input = InputState {
            mouse_x: 150.0,
            mouse_y: VIEW.height * 0.5,
            ..InputState::default()
        };
        let rows = &self.rows;
        let out = GroupList::new().focused(true).draw(
            VIEW,
            &self.layout,
            Some(1),
            &mut self.state,
            &mut self.list,
            &StyleResolver::new(&self.theme),
            &mut input,
            |i| rows[i].row(i),
        );
        black_box(out);
    }

    /// Scroll so the view shows the rows around `fraction` of the content.
    fn scroll_to(&mut self, fraction: f32) {
        let y = (self.layout.height() - VIEW.height) * fraction;
        self.state.scroll.offset = [0.0, y];
        self.state.scroll.target = [0.0, y];
    }

    /// Settle caches (text shaping, buffers) so the measured frame is a
    /// steady-state one.
    fn warm(&mut self) {
        for _ in 0..3 {
            self.frame();
        }
    }
}

#[test]
fn frame_allocations_do_not_depend_on_the_total_row_count() {
    // Both sidebars show the same first screenful of rows; only the amount
    // of content below it differs.
    let per_group = TOTAL_ROWS / GROUPS;
    let mut small = Harness::new(sidebar(3, 3 * per_group));
    let mut large = Harness::new(sidebar(GROUPS, TOTAL_ROWS));
    assert!(
        small.layout.height() > VIEW.height,
        "the small list must fill the view"
    );
    small.warm();
    large.warm();

    let ((), small_allocations) = count_allocations(|| small.frame());
    let ((), large_allocations) = count_allocations(|| large.frame());
    assert_eq!(
        small_allocations,
        large_allocations,
        "a frame of {TOTAL_ROWS} rows must allocate no more than one of {} rows",
        small.rows.len()
    );

    // Scrolled deep into the large list, the cost stays bounded by what's
    // visible: the list's own fixed cost, plus at most four text blocks per
    // visible row (an item's title, sub line, chip and age; a header's
    // monogram, name and count) and nothing else.
    let mut empty = Harness::new(Vec::new());
    empty.warm();
    let ((), fixed) = count_allocations(|| empty.frame());
    let theme = Theme::default();
    let s = StyleResolver::new(&theme);
    let (block, per_text) =
        count_allocations(|| s.mono_block("@warm-otter", 0.0, 0.0, TextSize::Meta, Ink::Caption));
    drop(block);
    const TEXTS_PER_ROW: usize = 4;

    large.scroll_to(0.6);
    large.warm();
    let ((), deep_allocations) = count_allocations(|| large.frame());
    let visible_rows = (VIEW.height / GroupRowKind::Header.height()).ceil() as usize + 1;
    let bound = fixed + visible_rows * TEXTS_PER_ROW * per_text;
    assert!(
        deep_allocations <= bound,
        "{deep_allocations} allocations for at most {visible_rows} visible rows \
         (bound {bound}: {fixed} fixed, {per_text} per text block)"
    );
}

#[test]
fn a_frame_of_ten_thousand_rows_stays_within_budget() {
    let mut large = Harness::new(sidebar(GROUPS, TOTAL_ROWS));
    assert_eq!(large.layout.len(), TOTAL_ROWS);
    large.scroll_to(0.5);
    large.warm();

    const FRAMES: u32 = 200;
    let start = Instant::now();
    for _ in 0..FRAMES {
        large.frame();
    }
    let per_frame = start.elapsed() / FRAMES;
    eprintln!("GroupList frame, {TOTAL_ROWS} rows in {GROUPS} groups: {per_frame:?}");
    if !cfg!(debug_assertions) {
        assert!(
            per_frame < FRAME_BUDGET,
            "a frame took {per_frame:?}, budget {FRAME_BUDGET:?}"
        );
    }
}

#[test]
fn rebuilding_ten_thousand_row_offsets_stays_within_budget() {
    let rows = sidebar(GROUPS, TOTAL_ROWS);
    let mut layout = GroupLayout::new();
    layout.rebuild(rows.iter().map(Source::kind));

    const REBUILDS: u32 = 100;
    // Only the rebuilds are counted: printing allocates when the test
    // harness captures output.
    let (per_rebuild, allocations) = count_allocations(|| {
        let start = Instant::now();
        for _ in 0..REBUILDS {
            layout.rebuild(black_box(&rows).iter().map(Source::kind));
        }
        start.elapsed() / REBUILDS
    });
    eprintln!("GroupLayout rebuild, {TOTAL_ROWS} rows: {per_rebuild:?}");
    if !cfg!(debug_assertions) {
        assert!(
            per_rebuild < REBUILD_BUDGET,
            "a rebuild took {per_rebuild:?}, budget {REBUILD_BUDGET:?}"
        );
    }
    assert_eq!(
        allocations, 0,
        "rebuilding at the same size reuses its buffers"
    );
}
