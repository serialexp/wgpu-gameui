//! wgpu-gameui: the CPU cost of one frame of the shared scene.
//!
//! Timed per frame: build a fresh `DrawList` (a rounded rectangle and a
//! `TextBlock` per cell), then `UiRenderer::render` (text shaping, instance
//! encoding, buffer upload, command encode) and `Queue::submit`. The GPU is
//! polled without waiting, so GPU execution is not in the sample.

use ui_compare_common::*;
use ui_compare_gameui::{Gpu, library, render_counts, rgba};
use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    Background, CornerRadii, DrawList, EdgeWidths, FontSystemHandle, QuadStyle, RenderStats,
    TextBlock, TextContent, WrapMode,
};

fn main() {
    let lib = library("gameui");
    let lib = lib.as_str();
    let gpu = Gpu::new(lib);
    let fonts = wgpu_gameui::shared_font_system();
    let mut ui = gpu.renderer(fonts.clone());

    for n in counts() {
        let grid = Grid::new(n);
        let labels: Vec<TextContent> = (0..n).map(|i| label(i).into()).collect();
        // The frame must contain the scene: the renderer's own stats are the
        // check, since a harness that silently drew nothing would be fast.
        {
            let mut list = DrawList::with_font_system(fonts.clone());
            build(&mut list, &grid, &labels, 0, Mode::Animated);
            let stats = gpu.draw(&mut ui, &list);
            eprintln!(
                "{lib} {n} cells: paint_runs={} draw_calls={} primitives={} \
                 blocks_looked_up={} glyphs_placed={} text_instances_built={} boxes_computed={}",
                stats.paint_runs,
                stats.draw_calls,
                stats.primitives,
                stats.text_blocks_looked_up,
                stats.glyphs_placed,
                stats.text_instances_built,
                stats.boxes_computed
            );
            report_counts(
                lib,
                "grid",
                &[
                    ("cells", n as u64),
                    ("list_bytes", list.heap_bytes() as u64),
                ],
            );
            assert!(
                stats.primitives >= n,
                "{lib} drew {} primitives",
                stats.primitives
            );
            gpu.wait();
            check_frame(lib, n, &gpu.read());
        }
        for (scenario, mode) in MODES {
            let stats = measure(|frame| {
                let mut list = DrawList::with_font_system(fonts.clone());
                build(&mut list, &grid, &labels, frame, mode);
                gpu.draw(&mut ui, &list);
            });
            gpu.wait();
            report(lib, scenario, n, &stats);
        }
    }

    if text_scenes_wanted() {
        let doc: Vec<TextContent> = (0..DOC_LINES).map(|i| doc_line(i).into()).collect();
        let para: TextContent = para_text().into();
        let scenes: [(&str, SceneFn); 3] = [
            ("scroll", &|list, frame| scroll_lines(list, &doc, frame)),
            ("churn", &|list, frame| churn_lines(list, frame)),
            ("para", &|list, frame| paragraph(list, &para, frame)),
        ];
        for (name, scene) in scenes {
            if !scene_wanted(name) {
                continue;
            }
            {
                let mut list = DrawList::with_font_system(fonts.clone());
                scene(&mut list, 40);
                let stats = gpu.draw(&mut ui, &list);
                eprintln!(
                    "{lib} {name}: blocks_looked_up={} glyphs_placed={} text_instances_built={} boxes_computed={}",
                    stats.text_blocks_looked_up,
                    stats.glyphs_placed,
                    stats.text_instances_built,
                    stats.boxes_computed
                );
                gpu.wait();
                check_text_frame(lib, name, &gpu.read());
            }
            let stats = measure(|frame| {
                let mut list = DrawList::with_font_system(fonts.clone());
                scene(&mut list, frame);
                gpu.draw(&mut ui, &list);
            });
            gpu.wait();
            report_scene(lib, name, &stats);
        }
    }

    // The rows and deep scenes, to set beside the retained tree's
    // (`ui-compare-gameui-retained`). Immediate mode has nothing to build
    // before its first frame, so its "build" is one frame.
    if group_wanted("rows") {
        let labels: Vec<TextContent> = (0..ROWS).map(|i| row_text(i, 0).into()).collect();
        for (name, mode) in ROW_MODES {
            if !scene_wanted(name) {
                continue;
            }
            let mut list = DrawList::with_font_system(fonts.clone());
            rows(&mut list, &labels, mode, 0);
            let stats = gpu.draw(&mut ui, &list);
            report_counts(
                lib,
                name,
                &[
                    ("texts", list.texts.len() as u64),
                    ("primitives", stats.primitives as u64),
                    ("glyphs_placed", stats.glyphs_placed as u64),
                    ("list_bytes", list.heap_bytes() as u64),
                ],
            );
            gpu.wait();
            check_text_frame(lib, name, &gpu.read());
            let before = shaped(&fonts);
            let mut last = RenderStats::default();
            let stats = measure(|frame| {
                let mut list = DrawList::with_font_system(fonts.clone());
                rows(&mut list, &labels, mode, frame);
                last = gpu.draw(&mut ui, &list);
            });
            gpu.wait();
            report_scene(lib, name, &stats);
            report_counts(lib, name, &render_counts(&last));
            let frames = (WARMUP_FRAMES + stats.frames) as u64;
            report_counts(
                lib,
                name,
                &[("frames", frames), ("shaped", shaped(&fonts) - before)],
            );
        }
    }
    if group_wanted("panel") {
        for (name, mode) in PANEL_MODES {
            if !scene_wanted(name) {
                continue;
            }
            let mut list = DrawList::with_font_system(fonts.clone());
            panel(&mut list, mode, 0);
            gpu.draw(&mut ui, &list);
            gpu.wait();
            check_text_frame(lib, name, &gpu.read());
            let before = shaped(&fonts);
            let mut last = RenderStats::default();
            let stats = measure(|frame| {
                let mut list = DrawList::with_font_system(fonts.clone());
                panel(&mut list, mode, frame);
                last = gpu.draw(&mut ui, &list);
            });
            gpu.wait();
            report_scene(lib, name, &stats);
            report_counts(lib, name, &render_counts(&last));
            let frames = (WARMUP_FRAMES + stats.frames) as u64;
            report_counts(
                lib,
                name,
                &[("frames", frames), ("shaped", shaped(&fonts) - before)],
            );
        }
    }
    if group_wanted("deep") {
        for (scenario, mode) in MODES {
            let name = format!("deep-{scenario}");
            if !scene_wanted(&name) {
                continue;
            }
            let mut list = DrawList::with_font_system(fonts.clone());
            deep(&mut list, mode, 0);
            gpu.draw(&mut ui, &list);
            report_counts(lib, &name, &[("list_bytes", list.heap_bytes() as u64)]);
            gpu.wait();
            check_scene_frame(lib, &name, &gpu.read());
            let mut last = RenderStats::default();
            let stats = measure(|frame| {
                let mut list = DrawList::with_font_system(fonts.clone());
                deep(&mut list, mode, frame);
                last = gpu.draw(&mut ui, &list);
            });
            gpu.wait();
            report_scene(lib, &name, &stats);
            report_counts(lib, &name, &render_counts(&last));
        }
    }
}

/// A text scene: draws frame `n` into a list.
type SceneFn<'a> = &'a dyn Fn(&mut DrawList, u64);

/// Text layouts shaped so far by the shared font system.
fn shaped(fonts: &FontSystemHandle) -> u64 {
    fonts.lock().expect("font system").layout_stats().shaped
}

fn background(list: &mut DrawList) {
    list.rounded_rect(Rect::new(0.0, 0.0, WIDTH, HEIGHT), 0.0, rgba(TEXT_BG));
}

fn line(content: impl Into<TextContent>, x: f32, y: f32) -> TextBlock {
    let mut block = TextBlock::new(content, x, y)
        .with_line_height(LINE_H)
        .with_max_width(LINE_COL_W);
    block.font_size = LINE_PX;
    block
}

/// Lines of retained text scrolling past: the strings repeat from frame to
/// frame, only their positions move.
fn scroll_lines(list: &mut DrawList, doc: &[TextContent], frame: u64) {
    background(list);
    for c in 0..LINE_COLS {
        for r in 0..LINE_ROWS {
            let (i, y) = scrolled_line(c, r, frame);
            list.text(line(doc[i].clone(), c as f32 * LINE_COL_W, y));
        }
    }
}

/// Lines whose text is new every frame, so none can be found in a cache.
fn churn_lines(list: &mut DrawList, frame: u64) {
    background(list);
    for c in 0..LINE_COLS {
        for r in 0..LINE_ROWS {
            let (i, y) = scrolled_line(c, r, 0);
            list.text(line(churn_line(i, frame), c as f32 * LINE_COL_W, y));
        }
    }
}

/// One large wrapped block, partly off screen, scrolled by moving it.
fn paragraph(list: &mut DrawList, text: &TextContent, frame: u64) {
    background(list);
    let mut block = TextBlock::new(text.clone(), PARA_X, para_y(frame))
        .with_line_height(PARA_LINE_H)
        .with_max_width(PARA_W);
    block.font_size = PARA_PX;
    list.text(block);
}

/// The scene as a `DrawList`: a rounded rectangle and a label per cell.
fn build(list: &mut DrawList, grid: &Grid, labels: &[TextContent], frame: u64, mode: Mode) {
    let (w, h) = grid.size();
    for (i, label) in labels.iter().enumerate().take(grid.n) {
        let (x, y) = grid.origin(i);
        list.rounded_rect(
            Rect::new(x, y, w, h),
            grid.radius,
            rgba(fill_in(mode, i, frame)),
        );
        let mut block = TextBlock::new(label.clone(), x + 1.0, y + 1.0)
            .with_line_height(grid.font_px * 1.25)
            .with_max_width(w);
        block.font_size = grid.font_px;
        list.text(block);
    }
}

fn solid(color: [u8; 3]) -> QuadStyle {
    QuadStyle {
        background: Background::Solid(rgba(color)),
        border_widths: EdgeWidths::uniform(0.0),
        border_color: [0.0; 4],
        corner_radii: CornerRadii::uniform(0.0),
    }
}

/// The rows in view, found by arithmetic as an immediate-mode list would,
/// inside the list's clip. Each row is a box and a label; labels are the
/// caller's, made once, except the one whose value changes.
fn rows(list: &mut DrawList, labels: &[TextContent], mode: RowMode, frame: u64) {
    let offset = row_offset(mode, frame);
    let hovered =
        (mode == RowMode::Hover).then(|| ((row_pointer(frame)[1] + offset) / ROW_H) as usize);
    list.push_clip_viewport(Rect::new(0.0, 0.0, WIDTH, HEIGHT));
    let first = (offset / ROW_H) as usize;
    let end = (((offset + HEIGHT) / ROW_H).ceil() as usize).min(ROWS);
    for (i, label) in labels.iter().enumerate().take(end).skip(first) {
        let y = i as f32 * ROW_H - offset;
        list.paint_quad(
            Rect::new(0.0, y, WIDTH, ROW_H),
            solid(row_fill(i, hovered == Some(i))),
        );
        let value = row_value(mode, i, frame);
        let content: TextContent = if value == 0 {
            label.clone()
        } else {
            row_text(i, value).into()
        };
        list.text(
            TextBlock::new(content, ROW_PAD[0], y + ROW_PAD[1])
                .with_size(ROW_PX)
                .with_line_height((ROW_PX * 1.25).ceil())
                .with_max_width(WIDTH - 2.0 * ROW_PAD[0])
                .with_wrap(WrapMode::None)
                .with_color_f32(rgba(ROW_TEXT)),
        );
    }
    list.pop_clip();
}

/// The panel scenes as an immediate-mode caller draws them: a box, and its
/// texts stacked down it, each paragraph measured at the panel's width to
/// find where the next goes (a cache hit when it is drawn).
fn panel(list: &mut DrawList, mode: PanelMode, frame: u64) {
    background(list);
    let (width, texts, gap): (f32, Vec<TextBlock>, f32) = match mode {
        PanelMode::Resize => {
            let width = panel_width(frame);
            let blocks = (0..PANEL_PARAS)
                .map(|i| {
                    TextBlock::new(panel_para(i), 0.0, 0.0)
                        .with_size(PANEL_PX)
                        .with_line_height(PANEL_LINE_H)
                        .with_max_width(width - 2.0 * PANEL_PAD)
                        .with_wrap(WrapMode::WordOrGlyph)
                        .with_color_f32(rgba(ROW_TEXT))
                })
                .collect();
            (width, blocks, PANEL_PAD)
        }
        PanelMode::Change => {
            let blocks = (0..PANEL_LABELS)
                .map(|i| {
                    TextBlock::new(panel_label(i, frame), 0.0, 0.0)
                        .with_size(PANEL_LABEL_PX)
                        .with_line_height(PANEL_LABEL_H)
                        .with_max_width(PANEL_LABELS_W - 2.0 * PANEL_PAD)
                        .with_wrap(WrapMode::None)
                        .with_color_f32(rgba(ROW_TEXT))
                })
                .collect();
            (PANEL_LABELS_W, blocks, 0.0)
        }
    };
    let heights: Vec<f32> = texts
        .iter()
        .map(|b| list.measure_block(b).1.ceil())
        .collect();
    let height = 2.0 * PANEL_PAD + heights.iter().sum::<f32>() + gap * (texts.len() - 1) as f32;
    let [x, mut y] = PANEL_AT;
    list.paint_quad(Rect::new(x, y, width, height), solid(row_fill(0, false)));
    y += PANEL_PAD;
    for (mut block, h) in texts.into_iter().zip(heights) {
        block.x = x + PANEL_PAD;
        block.y = y;
        list.text(block);
        y += h + gap;
    }
}

/// The deep scene's leaves: an immediate-mode caller has no tree to keep,
/// so it draws the leaves where the tree would put them.
fn deep(list: &mut DrawList, mode: Mode, frame: u64) {
    for i in 0..DEEP_LEAVES {
        let [x, y, w, h] = deep_leaf(i);
        list.paint_quad(Rect::new(x, y, w, h), solid(fill_in(mode, i, frame)));
    }
}
