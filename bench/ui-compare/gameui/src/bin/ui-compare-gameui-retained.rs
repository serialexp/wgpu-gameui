//! wgpu-gameui's retained tree (`crates/retained`): the same scenes as
//! `ui-compare-gameui`, kept as a tree of nodes instead of drawn afresh.
//!
//! Timed per frame, as `retained`: the scene's changes written to the tree
//! (setters, a scroll, a pointer move), `Ui::frame` (notifications, layout,
//! recording what changed, placing lists), then the same `UiRenderer::render`
//! and `submit` as immediate mode. `ret-tree` is the tree's share of those
//! frames alone (the changes and `Ui::frame`), timed inside the same frames.
//! A frame is drawn even when the tree says none is needed (a host would
//! skip it; `needs_frame` is reported in the counts).
//!
//! `build` is a scene made from nothing: the tree built, its first frame
//! (two for the rows, which are scrolled once laid out), and that frame
//! rendered, with the renderer's caches warm from the run before.

use std::time::Instant;

use ui_compare_common::*;
use ui_compare_gameui::{Gpu, library, render_counts, rgba};
use wgpu_gameui::{FontSystemHandle, RenderStats, TextContent, UiRenderer};
use wgpu_gameui_retained::{FrameContext, Insets, Look, Node, Size, Spec, Stack, Text, Ui};

/// Builds timed per scene.
const BUILDS: usize = 5;

fn context(frame: u64) -> FrameContext {
    FrameContext {
        now: frame as f64 / 60.0,
        viewport: [WIDTH, HEIGHT],
        fonts: 0,
        scale: 1.0,
    }
}

/// A scene kept as a tree: the tree, and how to write frame `frame`'s
/// changes into it.
trait Scene {
    fn ui(&mut self) -> &mut Ui;
    fn change(&mut self, frame: u64);
}

fn main() {
    let mut bench = Bench::new();
    let fonts = bench.fonts.clone();

    for n in counts() {
        let grid = Grid::new(n);
        let labels: Vec<TextContent> = (0..n).map(|i| label(i).into()).collect();
        let make = |mode| GridTree::new(fonts.clone(), &grid, &labels, mode);
        let build = bench.builds(|| make(Mode::Static));
        report(&bench.lib, "build", n, &build);
        for (scenario, mode) in MODES {
            let (frames, tree) = bench.run(&mut make(mode), &format!("grid-{n}"));
            report(&bench.lib, scenario, n, &frames);
            report(&bench.tree_lib, scenario, n, &tree);
            if mode == Mode::Animated {
                check_frame(&bench.lib, n, &bench.gpu.read());
            }
        }
    }

    if group_wanted("rows") {
        let labels: Vec<TextContent> = (0..ROWS).map(|i| row_text(i, 0).into()).collect();
        let make = |mode| RowsTree::new(fonts.clone(), &labels, mode);
        let build = bench.builds(|| make(RowMode::Static));
        report_scene(&bench.lib, "rows-bld", &build);
        for (name, mode) in ROW_MODES {
            if !scene_wanted(name) {
                continue;
            }
            let (frames, tree) = bench.run(&mut make(mode), name);
            report_scene(&bench.lib, name, &frames);
            report_scene(&bench.tree_lib, name, &tree);
            check_text_frame(&bench.lib, name, &bench.gpu.read());
        }
    }

    if group_wanted("deep") {
        let build = bench.builds(|| DeepTree::new(fonts.clone(), Mode::Static));
        report_scene(&bench.lib, "deep-bld", &build);
        for (scenario, mode) in MODES {
            let name = format!("deep-{scenario}");
            if !scene_wanted(&name) {
                continue;
            }
            let (frames, tree) = bench.run(&mut DeepTree::new(fonts.clone(), mode), &name);
            report_scene(&bench.lib, &name, &frames);
            report_scene(&bench.tree_lib, &name, &tree);
            check_scene_frame(&bench.lib, &name, &bench.gpu.read());
        }
    }
}

/// The target, the renderer and the fonts every scene is drawn with, and
/// the names results are reported under.
struct Bench {
    /// Whole frames: the tree's work, the render and the submit.
    lib: String,
    /// The tree's share of the same frames.
    tree_lib: String,
    gpu: Gpu,
    renderer: UiRenderer,
    fonts: FontSystemHandle,
}

impl Bench {
    fn new() -> Self {
        let lib = library("retained");
        let gpu = Gpu::new(&lib);
        let fonts = wgpu_gameui::shared_font_system();
        let renderer = gpu.renderer(fonts.clone());
        Self {
            lib,
            tree_lib: library("ret-tree"),
            gpu,
            renderer,
            fonts,
        }
    }

    /// Build a scene from nothing [`BUILDS`] times and time each: the tree,
    /// its first frame and that frame's render. The scene is dropped outside
    /// the timing.
    fn builds<S: Scene>(&mut self, mut make: impl FnMut() -> S) -> Stats {
        let mut times = Vec::with_capacity(BUILDS);
        for _ in 0..BUILDS {
            let started = Instant::now();
            let mut scene = make();
            let list = scene.ui().frame(context(0));
            self.gpu.draw(&mut self.renderer, list);
            times.push(started.elapsed().as_secs_f64() * 1e6);
            self.gpu.wait();
            drop(scene);
        }
        stats_from(times)
    }

    /// Time a scene's frames and print the counts of its last frame and what
    /// its tree holds. Returns the whole frames' times and the tree's share.
    fn run<S: Scene>(&mut self, scene: &mut S, name: &str) -> (Stats, Stats) {
        let mut tree = Vec::with_capacity(WARMUP_FRAMES + TIMED_FRAMES);
        let mut needed = 0u64;
        let mut last = RenderStats::default();
        let shaped_before = shaped(&self.fonts);
        let (gpu, renderer) = (&self.gpu, &mut self.renderer);
        let stats = measure(|frame| {
            let started = Instant::now();
            scene.change(frame);
            let ui = scene.ui();
            needed += u64::from(ui.needs_frame(context(frame).now));
            let list = ui.frame(context(frame));
            tree.push(started.elapsed().as_secs_f64() * 1e6);
            last = gpu.draw(renderer, list);
        });
        gpu.wait();
        let frames = (WARMUP_FRAMES + stats.frames) as u64;
        let shaped = shaped(&self.fonts) - shaped_before;
        let timed = tree.split_off(WARMUP_FRAMES.min(tree.len()));
        let ui = scene.ui();
        let f = ui.frame_stats();
        let t = ui.tree_stats();
        let g = ui.graph().stats();
        let list = ui.list().expect("drawn");
        let lib = &self.lib;
        report_counts(lib, name, &render_counts(&last));
        report_counts(
            lib,
            name,
            &[
                ("frames", frames),
                ("needed", needed),
                ("shaped", shaped),
                ("notified", f.notified as u64),
                ("visited", f.visited as u64),
                ("laid_out", f.laid_out as u64),
                ("measured", f.measured as u64),
                ("painted", f.painted as u64),
                ("recorded", f.recorded as u64),
                ("placed", f.placed as u64),
                ("copied", f.copied),
                ("examined", f.examined as u64),
                ("memo_runs", f.memo_runs),
            ],
        );
        report_counts(
            lib,
            name,
            &[
                ("nodes", t.nodes as u64),
                ("lists", t.lists as u64),
                ("records", t.records),
                ("root_records", list.prim_counts().total() as u64),
                ("list_bytes", t.list_bytes),
                ("root_bytes", list.heap_bytes() as u64),
                ("node_bytes", t.node_bytes),
                ("graph_nodes", g.nodes as u64),
                ("edges", g.edges as u64),
                ("closures", g.closures as u64),
            ],
        );
        (stats, stats_from(timed))
    }
}

/// Text layouts shaped so far by the shared font system (measuring and
/// drawing both go through it).
fn shaped(fonts: &FontSystemHandle) -> u64 {
    fonts.lock().expect("font system").layout_stats().shaped
}

// ---- Grid ------------------------------------------------------------------

/// The grid: each cell an absolutely placed box with its label as a child.
struct GridTree {
    ui: Ui,
    cells: Vec<Node>,
    mode: Mode,
}

impl GridTree {
    fn new(fonts: FontSystemHandle, grid: &Grid, labels: &[TextContent], mode: Mode) -> Self {
        let mut ui = Ui::new(fonts);
        let root = ui.root();
        let (w, h) = grid.size();
        let mut cells = Vec::with_capacity(grid.n);
        for (i, label) in labels.iter().enumerate() {
            let (x, y) = grid.origin(i);
            let cell = ui.add(
                root,
                Spec::new()
                    .width(Size::Fixed(w))
                    .height(Size::Fixed(h))
                    .absolute([x, y])
                    .fill(rgba(fill_in(mode, i, 0)))
                    .look(Look {
                        radius: grid.radius,
                        ..Look::default()
                    }),
            );
            let mut text = Text::new(label.clone(), grid.font_px);
            text.line_height = grid.font_px * 1.25;
            ui.add(
                cell,
                Spec::new()
                    .width(Size::Fixed(w))
                    .absolute([1.0, 1.0])
                    .text(text, [1.0; 4]),
            );
            cells.push(cell);
        }
        Self { ui, cells, mode }
    }
}

impl Scene for GridTree {
    fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    fn change(&mut self, frame: u64) {
        let changed = match self.mode {
            Mode::Static => 0,
            Mode::One => 1,
            Mode::Animated => self.cells.len(),
        };
        for (i, &cell) in self.cells[..changed].iter().enumerate() {
            self.ui.set_fill(cell, rgba(fill_in(self.mode, i, frame)));
        }
    }
}

// ---- Rows ------------------------------------------------------------------

/// The long list: a scrolling column of rows, each a box whose fill follows
/// its own hover signal, holding its label.
struct RowsTree {
    ui: Ui,
    list: Node,
    changing: Node,
    mode: RowMode,
}

impl RowsTree {
    fn new(fonts: FontSystemHandle, labels: &[TextContent], mode: RowMode) -> Self {
        let mut ui = Ui::new(fonts);
        let root = ui.root();
        let list = ui.add(
            root,
            Spec::new()
                .width(Size::Fill(1.0))
                .height(Size::Fill(1.0))
                .scroll(),
        );
        let mut changing = None;
        for (i, label) in labels.iter().enumerate() {
            let hovered = ui.signal(false);
            let fill = ui.memo(move |r| rgba(row_fill(i, r.get(hovered))));
            let row = ui.add(
                list,
                Spec::new()
                    .width(Size::Fill(1.0))
                    .height(Size::Fixed(ROW_H))
                    .stack(Stack::column().padding(Insets {
                        left: ROW_PAD[0],
                        top: ROW_PAD[1],
                        right: ROW_PAD[0],
                        bottom: 0.0,
                    }))
                    .fill(fill)
                    .hover(hovered),
            );
            let text = ui.add(
                row,
                Spec::new().text(row_label(label.clone()), rgba(ROW_TEXT)),
            );
            if i == ROW_CHANGING {
                changing = Some(text);
            }
        }
        // Scrolled once laid out, as an app opening a list at a saved place.
        ui.frame(context(0));
        ui.scroll_to(list, row_offset(mode, 0));
        let mut tree = Self {
            ui,
            list,
            changing: changing.expect("the changing row"),
            mode,
        };
        tree.change(0);
        tree
    }
}

fn row_label(content: TextContent) -> Text {
    let mut text = Text::new(content, ROW_PX).no_wrap();
    text.line_height = (ROW_PX * 1.25).ceil();
    text
}

impl Scene for RowsTree {
    fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    fn change(&mut self, frame: u64) {
        match self.mode {
            RowMode::Static => {}
            RowMode::One => {
                let value = row_value(self.mode, ROW_CHANGING, frame);
                self.ui.set_text(
                    self.changing,
                    row_label(row_text(ROW_CHANGING, value).into()),
                );
            }
            RowMode::Scroll => self.ui.scroll_to(self.list, row_offset(self.mode, frame)),
            RowMode::Hover => {
                let [x, y] = row_pointer(frame);
                self.ui.pointer_moved(x, y);
            }
        }
    }
}

// ---- Deep ------------------------------------------------------------------

/// The deep tree: inner nodes stacking rows and columns in turn, sharing
/// their room equally, with a filled leaf box at the bottom.
struct DeepTree {
    ui: Ui,
    leaves: Vec<Node>,
    mode: Mode,
}

impl DeepTree {
    fn new(fonts: FontSystemHandle, mode: Mode) -> Self {
        let fill = || Spec::new().width(Size::Fill(1.0)).height(Size::Fill(1.0));
        let mut ui = Ui::with_root(fonts, fill().stack(Stack::row()));
        let mut leaves = Vec::with_capacity(DEEP_LEAVES);
        let mut level = vec![ui.root()];
        for depth in 1..DEEP_DEPTH {
            let stack = if depth % 2 == 0 {
                Stack::row()
            } else {
                Stack::column()
            };
            let mut next = Vec::with_capacity(level.len() * DEEP_FANOUT);
            for &parent in &level {
                for _ in 0..DEEP_FANOUT {
                    next.push(ui.add(parent, fill().stack(stack)));
                }
            }
            level = next;
        }
        for &parent in &level {
            for _ in 0..DEEP_FANOUT {
                let i = leaves.len();
                leaves.push(ui.add(parent, fill().fill(rgba(fill_in(mode, i, 0)))));
            }
        }
        Self { ui, leaves, mode }
    }
}

impl Scene for DeepTree {
    fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    fn change(&mut self, frame: u64) {
        let changed = match self.mode {
            Mode::Static => 0,
            Mode::One => 1,
            Mode::Animated => self.leaves.len(),
        };
        for (i, &leaf) in self.leaves[..changed].iter().enumerate() {
            self.ui.set_fill(leaf, rgba(fill_in(self.mode, i, frame)));
        }
    }
}
