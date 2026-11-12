//! Ignored headless tests: the pixel level of the frame oracle
//! (`docs/design/frame-reuse.md`, Phase 0).
//!
//! A renderer that lives from frame to frame must draw every frame exactly as
//! a renderer that has seen nothing before it does. Each test plays a sequence
//! on one long-lived `HeadlessGpu` and, for every frame, builds a second one
//! from nothing in the state the first is in (fonts loaded, sprites as they
//! are now) and compares the pixels. Nothing keeps a frame yet, so today they
//! agree; the sequences are here to catch the first reuse that does not.
//!
//! The headless counterpart, which compares the plans and instances, is
//! `render/frame_oracle.rs`; this one reaches what that cannot: sprite
//! textures, atlas growth and compaction, fonts loaded mid-run, and several
//! renders recorded before one submit.
//!
//! ```
//! DISPLAY=:0 cargo test --test frame_oracle -- --ignored
//! ```

use wgpu_gameui::layout::Rect;
use wgpu_gameui::render::capture_with;
use wgpu_gameui::{
    AtlasId, DrawList, FontHandle, HeadlessGpu, NineSliceId, PlacedLog, Placement, RecordSpan,
    SpriteId, Srgb, TextBlock, load_font_bytes,
};

const SIZE: (u32, u32) = (240, 160);
const MONO_ITALIC: &[u8] = include_bytes!("../assets/fonts/ibm-plex/IBMPlexMono-Italic.ttf");
const SANS_ITALIC: &[u8] = include_bytes!("../assets/fonts/ibm-plex/IBMPlexSans-Italic.ttf");

/// The sprites a scene draws, as they are loaded now.
#[derive(Clone, Debug)]
struct Look {
    red: [u8; 4],
    blue: [u8; 4],
    panel: [u8; 4],
    border: u32,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            red: [220, 40, 40, 255],
            blue: [40, 60, 220, 200],
            panel: [200, 200, 90, 230],
            border: 4,
        }
    }
}

struct Art {
    red: SpriteId,
    blue: SpriteId,
    panel: SpriteId,
    /// The nine-slice "panel" is, to draw by id with a tint.
    nine: NineSliceId,
}

fn solid(rgba: [u8; 4], w: u32, h: u32) -> Vec<u8> {
    (0..w * h).flat_map(|_| rgba).collect()
}

/// A renderer in the state a run reaches: `fonts` loaded, sprites as `look`.
struct Run {
    gpu: HeadlessGpu,
    art: Art,
    look: Look,
    fonts: Vec<&'static [u8]>,
    /// Sprites in an atlas of their own, by name and colour, with the id the
    /// kept renderer gave them.
    spare: Vec<(String, [u8; 4], SpriteId)>,
    spare_atlas: AtlasId,
    /// The atlas `art.blue` is in.
    atlas: AtlasId,
}

impl Run {
    fn new(fonts: &[&'static [u8]], look: &Look, spare: &[(String, [u8; 4])]) -> Self {
        let mut gpu = HeadlessGpu::new().expect("no GPU adapter");
        for bytes in fonts {
            load_font_bytes(&gpu.font_system(), bytes).expect("font");
        }
        let ui = gpu.renderer();
        // Frames drawn from a kept plan are what this oracle is to find out about.
        ui.set_frame_comparison(true);
        let atlas = ui.create_atlas(256);
        let red = ui
            .load_sprite_rgba8("red", 8, 8, solid(look.red, 8, 8), Placement::Own)
            .unwrap();
        let blue = ui
            .load_sprite_rgba8(
                "blue",
                8,
                8,
                solid(look.blue, 8, 8),
                Placement::Atlas(atlas),
            )
            .unwrap();
        let panel = ui
            .load_sprite_rgba8("panel", 12, 12, solid(look.panel, 12, 12), Placement::Own)
            .unwrap();
        let nine = ui.register_nine_slice("panel", panel, [look.border; 4]);
        let spare_atlas = ui.create_atlas(256);
        let spare = spare
            .iter()
            .map(|(name, colour)| {
                let id = ui
                    .load_sprite_rgba8(
                        name,
                        16,
                        16,
                        solid(*colour, 16, 16),
                        Placement::Atlas(spare_atlas),
                    )
                    .unwrap();
                (name.clone(), *colour, id)
            })
            .collect();
        Self {
            gpu,
            art: Art {
                red,
                blue,
                panel,
                nine,
            },
            look: look.clone(),
            fonts: fonts.to_vec(),
            spare,
            spare_atlas,
            atlas,
        }
    }

    /// The same run, from nothing.
    fn fresh(&self) -> Self {
        let spare: Vec<_> = self
            .spare
            .iter()
            .map(|(name, colour, _)| (name.clone(), *colour))
            .collect();
        Self::new(&self.fonts, &self.look, &spare)
    }

    fn load_font(&mut self, bytes: &'static [u8]) {
        load_font_bytes(&self.gpu.font_system(), bytes).expect("font");
        self.fonts.push(bytes);
    }

    fn list(&self) -> DrawList {
        self.gpu.draw_list()
    }
}

/// What a frame draws: panels, text in a family, sprites by id and by name, a
/// nine-slice.
fn scene(list: &mut DrawList, art: &Art, family: Option<&FontHandle>, frame: usize) {
    shapes(list, art, frame);
    words(list, family, frame);
}

/// The part of the scene with no text in it.
fn shapes(list: &mut DrawList, art: &Art, frame: usize) {
    let shift = (frame % 7) as f32 * 3.0;
    for i in 0..4 {
        let x = 8.0 + 56.0 * i as f32;
        list.chrome_rect(
            Rect::new(x, 8.0 + shift, 50.0, 30.0),
            4.0,
            1.0,
            [0.2 + 0.1 * i as f32, 0.3, 0.5, 0.9],
            [0.8; 4],
        );
    }
    list.icon_sprite(art.red, 10.0, 60.0, 32.0, 32.0, [1.0; 4]);
    list.icon_sprite(art.blue, 50.0, 60.0, 32.0, 32.0, [1.0; 4]);
    list.icon("red", 90.0, 60.0, 24.0, 24.0);
    list.icon("blue", 120.0, 60.0, 24.0, 24.0);
    list.nine_slice(150.0, 56.0, 70.0, 44.0, "panel");
}

/// The text of the scene, in `family`.
fn words(list: &mut DrawList, family: Option<&FontHandle>, frame: usize) {
    let shift = (frame % 7) as f32 * 3.0;
    for i in 0..4 {
        let x = 8.0 + 56.0 * i as f32;
        let mut text =
            TextBlock::new(format!("Cell {}", i + frame), x + 4.0, 16.0 + shift).with_size(13.0);
        if let Some(family) = family {
            text = text.with_font(family.clone());
        }
        list.text(text);
    }
    let mut caption = TextBlock::new("Italic caption", 12.0, 110.0 + shift)
        .with_size(15.0)
        .italic();
    if let Some(family) = family {
        caption = caption.with_font(family.clone());
    }
    list.text(caption);
}

/// How far two renderers' pixels may differ in any channel.
///
/// Text is sampled from the glyph atlas with bilinear filtering, and a GPU
/// quantises the filter's weights, so a glyph at another place in the atlas
/// (and where it sits depends on what was drawn before it) can come out a
/// level off. That is `ATLAS_SAMPLING`; nothing else may differ at all.
#[derive(Clone, Copy, Debug)]
enum Tolerance {
    Exact,
    AtlasSampling,
}

impl Tolerance {
    fn levels(self) -> u8 {
        match self {
            Tolerance::Exact => 0,
            Tolerance::AtlasSampling => 1,
        }
    }
}

/// Where two images differ by more than `tolerance`: how many pixels, and the
/// first.
fn difference(a: &[u8], b: &[u8], tolerance: Tolerance) -> Option<(usize, u32, u32)> {
    assert_eq!(a.len(), b.len());
    let mut first = None;
    let mut count = 0;
    for (i, (pa, pb)) in a.chunks(4).zip(b.chunks(4)).enumerate() {
        if pa
            .iter()
            .zip(pb)
            .any(|(x, y)| x.abs_diff(*y) > tolerance.levels())
        {
            count += 1;
            first.get_or_insert(i as u32);
        }
    }
    first.map(|i| (count, i % SIZE.0, i / SIZE.0))
}

/// Capture `list` on the run and on a run built from nothing in its state, and
/// say where they differ.
fn check(
    run: &mut Run,
    list: &DrawList,
    scale: f32,
    tolerance: Tolerance,
    what: &str,
    failures: &mut Vec<String>,
) {
    let kept = run.gpu.capture_scaled(list, SIZE, scale);
    assert!(
        kept.chunks(4).any(|p| p[3] > 0),
        "{what}: the kept renderer drew nothing"
    );
    let mut fresh = run.fresh();
    // The list holds content, not layouts: the fresh renderer shapes it in a
    // font system of its own.
    let from_nothing = fresh.gpu.capture_scaled(list, SIZE, scale);
    if let Some((count, x, y)) = difference(&kept, &from_nothing, tolerance) {
        failures.push(format!(
            "{what} at scale {scale}: {count} pixels differ ({tolerance:?}), the first at ({x}, {y})"
        ));
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_long_lived_renderer_draws_each_frame_as_a_fresh_one_does() {
    let mut run = Run::new(&[], &Look::default(), &[]);
    let mono = FontHandle("IBM Plex Mono".into());
    let mut failures = Vec::new();
    for frame in 0..12 {
        let scale = [1.0, 1.25, 1.0, 1.5][frame % 4];
        let family = (frame % 3 == 1).then_some(&mono);
        let mut list = run.list();
        scene(&mut list, &run.art, family, frame);
        check(
            &mut run,
            &list,
            scale,
            Tolerance::AtlasSampling,
            &format!("frame {frame}"),
            &mut failures,
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[ignore = "requires a GPU adapter"]
fn sprites_updated_replaced_and_renamed_between_frames() {
    let mut run = Run::new(&[], &Look::default(), &[]);
    let mut failures = Vec::new();
    let mut frame = 0;
    let mut draw = |run: &mut Run, what: &str, failures: &mut Vec<String>| {
        let mut list = run.list();
        shapes(&mut list, &run.art, 0);
        check(run, &list, 1.0, Tolerance::Exact, what, failures);
        frame += 1;
    };
    draw(&mut run, "first", &mut failures);
    draw(&mut run, "again", &mut failures);

    // Pixels replaced in place, in a texture of its own and in an atlas.
    run.look.red = [20, 200, 80, 255];
    run.gpu
        .renderer()
        .update_sprite_rgba8(run.art.red, solid(run.look.red, 8, 8))
        .unwrap();
    draw(&mut run, "own texture updated", &mut failures);
    // A sprite in an atlas can't be updated in place; it leaves and a new one
    // takes its name.
    run.look.blue = [250, 250, 20, 160];
    assert!(run.gpu.renderer().unload_sprite(run.art.blue));
    run.art.blue = run
        .gpu
        .renderer()
        .load_sprite_rgba8(
            "blue",
            8,
            8,
            solid(run.look.blue, 8, 8),
            Placement::Atlas(run.atlas),
        )
        .unwrap();
    draw(&mut run, "atlas sprite replaced", &mut failures);

    // Unloaded and loaded again under the same name: the id may be reused.
    assert!(run.gpu.renderer().unload_sprite(run.art.red));
    run.look.red = [10, 10, 10, 255];
    run.art.red = run
        .gpu
        .renderer()
        .load_sprite_rgba8("red", 8, 8, solid(run.look.red, 8, 8), Placement::Own)
        .unwrap();
    draw(&mut run, "own sprite reloaded", &mut failures);

    // The nine-slice name now resolves to another sprite and border.
    run.look.panel = [60, 140, 220, 255];
    run.look.border = 3;
    run.gpu.renderer().unload_sprite(run.art.panel);
    run.art.panel = run
        .gpu
        .renderer()
        .load_sprite_rgba8(
            "panel2",
            12,
            12,
            solid(run.look.panel, 12, 12),
            Placement::Own,
        )
        .unwrap();
    run.gpu
        .renderer()
        .register_nine_slice("panel", run.art.panel, [3; 4]);
    draw(&mut run, "nine-slice re-registered", &mut failures);

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[ignore = "requires a GPU adapter"]
fn an_atlas_compacted_after_sprites_leave_it() {
    let spare: Vec<(String, [u8; 4])> = (0..12u8)
        .map(|i| (format!("spare{i}"), [i * 20, 255 - i * 20, 90, 255]))
        .collect();
    let mut run = Run::new(&[], &Look::default(), &spare);
    let mut failures = Vec::new();
    let frame = |run: &mut Run, what: &str, failures: &mut Vec<String>| {
        let mut list = run.list();
        shapes(&mut list, &run.art, 0);
        for (i, (name, _, _)) in run.spare.iter().enumerate() {
            list.icon(name, 6.0 + 18.0 * i as f32, 134.0, 16.0, 16.0);
        }
        check(run, &list, 1.0, Tolerance::Exact, what, failures);
    };
    frame(&mut run, "atlas full", &mut failures);

    // Every other sprite leaves; the rest stay where they are, then move.
    let mut kept = Vec::new();
    for (i, entry) in std::mem::take(&mut run.spare).into_iter().enumerate() {
        if i % 2 == 0 {
            assert!(run.gpu.renderer().unload_sprite(entry.2));
        } else {
            kept.push(entry);
        }
    }
    run.spare = kept;
    frame(&mut run, "after sprites left", &mut failures);
    assert!(
        run.gpu.renderer().compact_atlas(run.spare_atlas),
        "compaction moved nothing"
    );
    frame(&mut run, "after compaction", &mut failures);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[ignore = "requires a GPU adapter"]
fn fonts_loaded_between_frames() {
    let mut run = Run::new(&[], &Look::default(), &[]);
    let mut failures = Vec::new();
    for (frame, load) in [None, None, Some(MONO_ITALIC), None, Some(SANS_ITALIC), None]
        .into_iter()
        .enumerate()
    {
        if let Some(bytes) = load {
            run.load_font(bytes);
        }
        let mut list = run.list();
        scene(&mut list, &run.art, None, frame);
        check(
            &mut run,
            &list,
            1.0,
            Tolerance::AtlasSampling,
            &format!(
                "frame {frame}{}",
                if load.is_some() { " after a font" } else { "" }
            ),
            &mut failures,
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[ignore = "requires a GPU adapter"]
fn the_glyph_atlas_growing_between_frames() {
    let mut run = Run::new(&[], &Look::default(), &[]);
    let mut failures = Vec::new();
    let start = run.gpu.renderer().glyph_atlas_size();
    let mut grown_at = None;
    // Each frame brings glyphs at sizes none before it drew.
    for frame in 0..60usize {
        let mut list = run.list();
        scene(&mut list, &run.art, None, frame);
        let size = 9.0 + frame as f32 * 0.5;
        let text: String = (33u8..127).map(char::from).collect();
        for row in 0..3 {
            list.text(
                TextBlock::new(text.clone(), 4.0, 4.0 + 20.0 * row as f32)
                    .with_size(size + row as f32 * 0.25)
                    .with_max_width(230.0),
            );
        }
        check(
            &mut run,
            &list,
            2.0,
            Tolerance::AtlasSampling,
            &format!("frame {frame}"),
            &mut failures,
        );
        if grown_at.is_none() && run.gpu.renderer().glyph_atlas_size() != start {
            grown_at = Some(frame);
        }
        if grown_at.is_some_and(|at| frame >= at + 2) {
            break;
        }
    }
    assert!(
        grown_at.is_some(),
        "the glyph atlas never grew from {start:?}"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[ignore = "requires a GPU adapter"]
fn several_renders_recorded_before_one_submit() {
    let mut run = Run::new(&[], &Look::default(), &[]);
    let mut failures = Vec::new();
    for frame in 0..6usize {
        let mut a = run.list();
        scene(&mut a, &run.art, None, frame);
        let mut b = run.list();
        // A different list over the first: overlaps it, shares its text.
        b.chrome_rect(
            Rect::new(20.0, 20.0, 150.0, 90.0),
            6.0,
            2.0,
            [0.1, 0.1, 0.1, 0.8],
            [1.0; 4],
        );
        b.text(TextBlock::new(format!("Over {frame}"), 30.0, 40.0).with_size(18.0));
        if frame % 2 == 1 {
            scene(&mut b, &run.art, None, frame + 1);
        }
        let kept = both(&mut run, &a, &b);
        let mut fresh = run.fresh();
        let from_nothing = both(&mut fresh, &a, &b);
        if let Some((count, x, y)) = difference(&kept, &from_nothing, Tolerance::AtlasSampling) {
            failures.push(format!(
                "frame {frame}: {count} pixels differ, the first at ({x}, {y})"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `a`, then `b` over it, recorded before a single submit.
fn both(run: &mut Run, a: &DrawList, b: &DrawList) -> Vec<u8> {
    let device = run.gpu.device().clone();
    let queue = run.gpu.queue().clone();
    let ui = run.gpu.renderer();
    ui.begin_frame();
    capture_with(&device, &queue, SIZE, Srgb::TRANSPARENT, |encoder, view| {
        ui.render(&device, &queue, encoder, view, SIZE, 1.0, a);
        ui.render(&device, &queue, encoder, view, SIZE, 1.0, b);
    })
}

/// The renderer's own account of how each list compares with the last frame's,
/// per place in the render call, through the stats it reports.
#[test]
#[ignore = "requires a GPU adapter"]
fn the_renderer_reports_how_each_list_compares_with_last_frame() {
    let mut run = Run::new(&[], &Look::default(), &[]);
    run.gpu.renderer().set_frame_comparison(true);
    let over = |run: &Run, frame: usize, colour: f32, x: f32| {
        let mut b = run.list();
        b.chrome_rect(
            Rect::new(x, 20.0, 150.0, 90.0),
            6.0,
            2.0,
            [colour, 0.1, 0.1, 0.8],
            [1.0; 4],
        );
        b.text(TextBlock::new(format!("Over {frame}"), 30.0, 40.0).with_size(18.0));
        b
    };
    let mut stats = Vec::new();
    // (base frame, colour of the second list, its x)
    for (base, colour, x) in [
        (0, 0.1, 20.0), // nothing to compare with
        (0, 0.1, 20.0), // both again
        (0, 0.5, 20.0), // the second list recoloured
        (0, 0.5, 33.0), // the second list moved
        (1, 0.5, 33.0), // the first list changed
    ] {
        let mut a = run.list();
        scene(&mut a, &run.art, None, base);
        let b = over(&run, 0, colour, x);
        both(&mut run, &a, &b);
        stats.push(run.gpu.renderer().frame_stats());
    }
    let counts: Vec<_> = stats
        .iter()
        .map(|s| (s.lists_compared, s.lists_identical, s.lists_keeping_places))
        .collect();
    assert_eq!(
        counts,
        [(0, 0, 0), (2, 2, 2), (2, 1, 2), (2, 1, 1), (2, 1, 1)],
        "lists compared, identical and keeping their places, per frame: {stats:#?}"
    );
    let replayed: Vec<_> = stats.iter().map(|s| s.plans_replayed).collect();
    assert_eq!(replayed, [0, 2, 2, 1, 1], "plans replayed, per frame");
    // Text is prepared again only in a list in which something moved, and the
    // second list's label never changes: frames 1 and 2 place no glyph, frame
    // 3 only the moved list's and frame 4 only the first list's.
    let placed: Vec<_> = stats.iter().map(|s| s.glyphs_placed).collect();
    assert_eq!(placed[1..3], [0, 0], "glyphs placed, per frame: {placed:?}");
    assert!(
        placed[3] > 0 && placed[4] > 0 && placed[3] + placed[4] == placed[0],
        "glyphs placed, per frame: {placed:?}"
    );
    assert!(
        stats[1].snapshot_bytes > 0,
        "the kept frames hold something"
    );
    assert!(stats[1].records_same > 0 && stats[1].records_place == 0);
    assert!(stats[2].records_look > 0 && stats[2].records_place == 0);
    assert!(stats[3].records_place > 0);
}

/// A layer that empties, and fills again, keeps its place in the render call:
/// the layer over it is compared with itself, not with the one under it, the
/// refilled one with what it last drew, and every frame draws as a fresh
/// renderer draws it.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_layer_emptied_and_filled_again_leaves_the_layers_after_it_their_places() {
    let mut run = Run::new(&[], &Look::default(), &[]);
    run.gpu.renderer().set_check_plan(true);
    run.gpu.renderer().set_frame_comparison(true);
    let layer = |run: &Run, i: usize, filled: bool| {
        let mut list = run.list();
        if filled {
            let x = 8.0 + 90.0 * i as f32;
            list.chrome_rect(
                Rect::new(x, 20.0, 80.0, 50.0),
                4.0,
                1.0,
                [0.2 + 0.3 * i as f32, 0.4, 0.6, 0.9],
                [0.8; 4],
            );
            list.text(TextBlock::new(format!("Layer {i}"), x + 6.0, 30.0).with_size(14.0));
        }
        list
    };
    // Whether the middle layer has anything in it, per frame.
    let middle = [true, false, false, true, true];
    let mut compared = Vec::new();
    let mut failures = Vec::new();
    for (n, &filled) in middle.iter().enumerate() {
        let lists = [
            layer(&run, 0, true),
            layer(&run, 1, filled),
            layer(&run, 2, true),
        ];
        let kept = layered(&mut run, &lists);
        let s = run.gpu.renderer().frame_stats();
        compared.push((s.lists_compared, s.lists_keeping_places, s.plans_replayed));
        let mut fresh = run.fresh();
        let from_nothing = layered(&mut fresh, &lists);
        if let Some((count, x, y)) = difference(&kept, &from_nothing, Tolerance::AtlasSampling) {
            failures.push(format!(
                "frame {n}: {count} pixels differ, the first at ({x}, {y})"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // (lists compared, keeping their places, plans replayed). The empty
    // layer is not compared; the one over it always keeps its place, and the
    // middle one, filled again as it was, replays its plan.
    assert_eq!(
        compared,
        [(0, 0, 0), (2, 2, 2), (2, 2, 2), (3, 3, 3), (3, 3, 3)],
        "per frame"
    );
}

/// The scene again with its colours changed: where everything is stays.
fn tinted_scene(list: &mut DrawList, art: &Art, frame: usize, hue: f32) {
    let shift = (frame % 7) as f32 * 3.0;
    for i in 0..4 {
        let x = 8.0 + 56.0 * i as f32;
        list.chrome_rect(
            Rect::new(x, 8.0 + shift, 50.0, 30.0),
            4.0,
            1.0,
            [hue, 0.3 + 0.1 * i as f32, 0.5, 0.9],
            [0.8 - hue * 0.5; 4],
        );
        list.text(
            TextBlock::new(format!("Cell {}", i + frame), x + 4.0, 16.0 + shift)
                .with_size(13.0)
                .with_color_f32([1.0 - hue, hue, 0.4, 1.0]),
        );
    }
    list.icon_sprite(art.red, 10.0, 60.0, 32.0, 32.0, [1.0, hue, hue, 1.0]);
    list.icon_sprite(art.blue, 50.0, 60.0, 32.0, 32.0, [hue, 1.0, 1.0, 0.8]);
    list.nine_slice(150.0, 56.0, 70.0, 44.0, "panel");
}

/// A frame drawn from the plan kept for it looks as one planned from nothing.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_frame_drawn_from_a_kept_plan_looks_as_a_fresh_one_does() {
    let mut run = Run::new(&[], &Look::default(), &[]);
    let mut failures = Vec::new();
    let mut replayed = Vec::new();
    let mut placed = Vec::new();
    // (frame, hue): the same again, only recoloured, then moved, and again.
    for (n, (frame, hue)) in [
        (0, 0.0),
        (0, 0.0),
        (0, 0.3),
        (0, 0.6),
        (1, 0.6),
        (1, 0.6),
        (1, 0.9),
    ]
    .into_iter()
    .enumerate()
    {
        let mut list = run.list();
        tinted_scene(&mut list, &run.art, frame, hue);
        check(
            &mut run,
            &list,
            1.0,
            Tolerance::AtlasSampling,
            &format!("step {n}"),
            &mut failures,
        );
        let stats = run.gpu.renderer().frame_stats();
        replayed.push(stats.plans_replayed);
        placed.push(stats.glyphs_placed);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(replayed, [0, 1, 1, 1, 0, 1, 1], "plans replayed per frame");
    // The same frame again places no glyph; a recoloured one places the
    // recoloured labels' glyphs again, which are all of them.
    assert_eq!(
        placed,
        [placed[0], 0, placed[0], placed[0], placed[4], 0, placed[4]],
        "glyphs placed per frame"
    );
}

/// A scene in which two units move and the rest stay: the boxes of the rest
/// are taken from the last frame's plan.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_frame_in_which_a_few_units_moved_looks_as_a_fresh_one_does() {
    let mut run = Run::new(&[], &Look::default(), &[]);
    let mut failures = Vec::new();
    let mut reused = Vec::new();
    for n in 0..6usize {
        let mut list = run.list();
        scene(&mut list, &run.art, None, 0);
        let slide = n as f32 * 7.0;
        list.chrome_rect(
            Rect::new(10.0 + slide, 130.0, 40.0, 20.0),
            3.0,
            1.0,
            [0.7, 0.2, 0.2, 1.0],
            [1.0; 4],
        );
        list.text(TextBlock::new("Moving", 60.0 + slide, 132.0).with_size(14.0));
        check(
            &mut run,
            &list,
            1.0,
            Tolerance::AtlasSampling,
            &format!("step {n}"),
            &mut failures,
        );
        let stats = run.gpu.renderer().frame_stats();
        reused.push((stats.boxes_reused, stats.boxes_computed));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(reused[0].0, 0, "nothing to take boxes from the first frame");
    for (n, (taken, computed)) in reused.iter().enumerate().skip(1) {
        assert!(
            *taken >= 8 && *computed <= 3,
            "step {n}: {taken} boxes reused, {computed} computed"
        );
    }
}
/// One cell of the patched scene, drawn at the origin: a panel, a triangle
/// in the soup, a sprite and a label, in `hue`, the label `dy` down and the
/// triangle `dx` across.
fn cell(run: &Run, i: usize, hue: f32, dy: f32, dx: f32) -> DrawList {
    let mut list = run.list();
    list.chrome_rect(
        Rect::new(0.0, 0.0, 70.0, 60.0),
        4.0,
        1.0,
        [hue, 0.3, 0.2 + 0.1 * i as f32, 0.9],
        [0.8; 4],
    );
    list.triangle(
        (4.0 + dx, 40.0),
        (20.0 + dx, 40.0),
        (12.0 + dx, 54.0),
        [0.9, hue, 0.2, 1.0],
    );
    list.icon_sprite(run.art.red, 46.0, 34.0, 20.0, 20.0, [1.0, hue, 1.0, 1.0]);
    list.text(
        TextBlock::new(format!("Cell {i}"), 4.0, 6.0 + dy)
            .with_size(13.0)
            .with_color_f32([1.0 - hue, hue, 0.4, 1.0]),
    );
    list
}

/// A list kept and patched in place, whose owner reports its edits, is
/// drawn as a renderer from nothing draws it, without being compared or
/// copied; the renderer reuses its plan where the edits leave every record
/// in place. The crate's tests run with the `edit-oracle` feature, which
/// also checks each report against a comparison.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_list_patched_in_place_that_reports_its_edits_looks_as_a_fresh_one_does() {
    let mut run = Run::new(&[], &Look::default(), &[]);
    /// (hue, label shift, triangle shift)
    type Cell = (f32, f32, f32);
    fn place_all(run: &Run, list: &mut DrawList, log: &mut PlacedLog, cells: &[Cell]) {
        list.clear();
        log.clear();
        for (i, &(hue, dy, dx)) in cells.iter().enumerate() {
            let at = [6.0 + 78.0 * (i % 3) as f32, 8.0 + 72.0 * (i / 3) as f32];
            log.place(list, &cell(run, i, hue, dy, dx), at, None);
        }
    }
    let mut cells: [Cell; 6] = [(0.2, 0.0, 0.0); 6];
    let mut list = run.list();
    list.report_edits(true);
    let mut log = PlacedLog::default();
    place_all(&run, &mut list, &mut log, &cells);
    let mut failures = Vec::new();
    let mut seen = Vec::new();
    // What each step does to the cells; `None` records the list again.
    let steps: [Option<&[(usize, Cell)]>; 8] = [
        // Drawn first, then untouched.
        Some(&[]),
        Some(&[]),
        // One cell recoloured.
        Some(&[(2, (0.7, 0.0, 0.0))]),
        // A triangle moved, in the soup; then a label moved.
        Some(&[(3, (0.2, 0.0, 6.0))]),
        Some(&[(4, (0.2, 5.0, 0.0))]),
        // Two recoloured.
        Some(&[(1, (0.5, 0.0, 0.0)), (5, (0.9, 0.0, 0.0))]),
        // Recorded again, as it was; then untouched.
        None,
        Some(&[]),
    ];
    for (n, step) in steps.into_iter().enumerate() {
        match step {
            Some(changes) => {
                for &(i, look) in changes {
                    cells[i] = look;
                    let (hue, dy, dx) = look;
                    let piece = cell(&run, i, hue, dy, dx);
                    assert!(
                        log.fits(i as u32, &piece),
                        "step {n}: cell {i} keeps its shape"
                    );
                    let placed = log.placed(i as u32).expect("placed");
                    assert!(list.patch_translated(&placed, &piece, &RecordSpan::all_of(&piece)));
                }
            }
            None => place_all(&run, &mut list, &mut log, &cells),
        }
        check(
            &mut run,
            &list,
            1.0,
            Tolerance::AtlasSampling,
            &format!("step {n}"),
            &mut failures,
        );
        list.begin_edits();
        let s = run.gpu.renderer().frame_stats();
        seen.push((s.lists_reported, s.lists_compared, s.snapshot_bytes));
        seen.push((s.plans_replayed, s.lists_keeping_places, s.lists_identical));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let (counts, reuse): (Vec<_>, Vec<_>) = seen.chunks(2).map(|c| (c[0], c[1])).unzip();
    assert_eq!(
        counts,
        [(1, 0, 0); 8],
        "lists reported, lists compared, snapshot bytes, per step"
    );
    assert_eq!(
        reuse,
        [
            (0, 0, 0),
            (1, 1, 1),
            (1, 1, 0),
            (0, 0, 0),
            (0, 0, 0),
            (1, 1, 0),
            (0, 0, 0),
            (1, 1, 1),
        ],
        "plans replayed, lists keeping their places, identical lists, per step"
    );
}
/// The colours of [`every_family`]'s records, one per family, and where it is
/// drawn.
#[derive(Clone, Copy)]
struct Hues {
    chrome: f32,
    circle: f32,
    sprite: f32,
    nine: f32,
    soup: f32,
    tail: f32,
    text: f32,
    effects: f32,
    #[cfg(feature = "phosphor-icons")]
    icon: f32,
    /// Across, in logical px: off the target, every record is culled.
    x: f32,
}

impl Hues {
    const BASE: Self = Self {
        chrome: 0.2,
        circle: 0.3,
        sprite: 0.4,
        nine: 0.5,
        soup: 0.6,
        tail: 0.7,
        text: 0.8,
        effects: 0.9,
        #[cfg(feature = "phosphor-icons")]
        icon: 0.1,
        x: 0.0,
    };

    /// Every colour as [`BASE`](Self::BASE)'s, where it is drawn kept.
    fn base_colours(&mut self) {
        *self = Self {
            x: self.x,
            ..Self::BASE
        };
    }

    fn soup_and_tail(&mut self, hue: f32) {
        (self.soup, self.tail) = (hue, hue);
    }
}

/// A record of every family in a hue of its own: a panel, a circle, a sprite,
/// a nine-slice, a triangle in the soup, text plain and with a shadow and a
/// glow, an MSDF icon, and a triangle after the last paint command (the soup's
/// tail).
fn every_family(run: &Run, h: Hues) -> DrawList {
    let mut list = run.list();
    let x = h.x;
    list.chrome_rect(
        Rect::new(x + 8.0, 8.0, 60.0, 30.0),
        4.0,
        1.0,
        [h.chrome, 0.3, 0.5, 0.9],
        [0.8; 4],
    );
    list.circle((x + 90.0, 24.0), 14.0, [0.2, h.circle, 0.7, 1.0]);
    let tint = [1.0, h.sprite, 1.0, 1.0];
    list.icon_sprite(run.art.red, x + 112.0, 8.0, 28.0, 28.0, tint);
    let tint = [h.nine, 1.0, 1.0, 1.0];
    list.nine_slice_id(run.art.nine, x + 150.0, 6.0, 70.0, 40.0, tint);
    list.triangle(
        (x + 10.0, 60.0),
        (x + 40.0, 60.0),
        (x + 25.0, 90.0),
        [0.9, h.soup, 0.2, 1.0],
    );
    list.text(
        TextBlock::new("Plain words", x + 50.0, 58.0)
            .with_size(14.0)
            .with_color_f32([1.0 - h.text, h.text, 0.4, 1.0]),
    );
    list.text(
        TextBlock::new("Shadowed", x + 50.0, 84.0)
            .with_size(16.0)
            .with_color_f32([h.effects, 0.9, 0.9, 1.0])
            .with_shadow((h.effects * 255.0) as u8, 0, 0, 200, 2.0, 2.0, 2.0)
            .with_glow(0, (h.effects * 255.0) as u8, 0, 160, 3.0),
    );
    #[cfg(feature = "phosphor-icons")]
    list.phosphor_icon(
        Rect::new(x + 180.0, 70.0, 28.0, 28.0),
        wgpu_gameui::PhosphorIcon::Folder,
        [1.0, h.icon, 0.1, 1.0],
    );
    // Recorded last, so no paint command follows it: the soup's tail.
    list.triangle(
        (x + 10.0, 120.0),
        (x + 60.0, 120.0),
        (x + 35.0, 150.0),
        [0.3, 0.3, h.tail, 1.0],
    );
    list
}

/// A place keeps its records on the GPU (`docs/design/frame-reuse.md`,
/// Phase 5): a frame drawn from its kept plan writes again only the records
/// whose look changed, and looks as a fresh renderer's frame does. The
/// renderer checks, as it draws, that what each place keeps is what a fill
/// from nothing would hold.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_place_that_keeps_its_records_rewrites_only_what_changed() {
    let mut run = Run::new(&[], &Look::default(), &[]);
    run.gpu.renderer().set_check_plan(true);
    /// What a step does to the frame before it.
    type Edit = fn(&mut Hues);
    // (what, the edit, `None` for a frame that draws nothing, and the records
    // rewritten, or `None` for a place filled from nothing)
    let mut steps: Vec<(&str, Option<Edit>, Option<usize>)> = vec![
        ("first", Some(|_| {}), None),
        ("again", Some(|_| {}), Some(0)),
        ("panel", Some(|h| h.chrome = 0.9), Some(1)),
        ("circle", Some(|h| h.circle = 0.9), Some(1)),
        ("sprite", Some(|h| h.sprite = 0.9), Some(1)),
        ("nine-slice", Some(|h| h.nine = 0.9), Some(1)),
        ("soup", Some(|h| h.soup = 0.1), Some(1)),
        ("tail", Some(|h| h.tail = 0.1), Some(1)),
        // The soup's vertices are written once for both.
        ("soup and tail", Some(|h| h.soup_and_tail(0.4)), Some(1)),
        ("text", Some(|h| h.text = 0.1), Some(1)),
        ("effects", Some(|h| h.effects = 0.1), Some(1)),
    ];
    #[cfg(feature = "phosphor-icons")]
    steps.push(("icon", Some(|h| h.icon = 0.9), Some(1)));
    // Panel, circle, sprite, nine-slice, the soup's vertices, two texts, and
    // the icon.
    let every = 7 + usize::from(cfg!(feature = "phosphor-icons"));
    let base: Edit = Hues::base_colours;
    steps.extend([
        ("every family", Some(base), Some(every)),
        ("moved", Some(|h| h.x = 3.0), None),
        ("moved again", Some(|_| {}), Some(0)),
        ("culled", Some(|h| h.x = 1000.0), None),
        ("culled and recoloured", Some(|h| h.chrome = 0.1), Some(0)),
        ("back", Some(|h| h.x = 0.0), None),
        ("back and recoloured", Some(|h| h.chrome = 0.6), Some(1)),
        // A frame in which the place draws nothing gives it back; the next
        // frame replays the kept plan but fills the place from nothing.
        ("empty", None, None),
        ("after a frame without", Some(|_| {}), None),
        ("then recoloured", Some(|h| h.chrome = 0.3), Some(1)),
    ]);
    let mut hues = Hues {
        chrome: 0.25,
        circle: 0.35,
        ..Hues::BASE
    };
    let mut failures = Vec::new();
    let mut seen = Vec::new();
    let mut stats = std::collections::HashMap::new();
    for (n, &(what, edit, want)) in steps.iter().enumerate() {
        let list = match edit {
            Some(edit) => {
                edit(&mut hues);
                every_family(&run, hues)
            }
            None => run.list(),
        };
        // A frame that draws nothing has nothing to compare.
        if edit.is_some() && hues.x < SIZE.0 as f32 {
            check(
                &mut run,
                &list,
                1.0,
                Tolerance::AtlasSampling,
                &format!("step {n} ({what})"),
                &mut failures,
            );
        } else {
            run.gpu.capture_scaled(&list, SIZE, 1.0);
        }
        let s = run.gpu.renderer().frame_stats();
        stats.insert(what, s);
        // A replayed plan rewrites; anything else fills from nothing.
        let rewrote = (s.plans_replayed > 0).then_some(s.records_rewritten);
        let rewrote = match (want, what) {
            (None, "after a frame without") => {
                assert_eq!(s.plans_replayed, 1, "the kept plan is replayed");
                (s.records_rewritten > 0).then_some(s.records_rewritten)
            }
            _ => rewrote,
        };
        seen.push((what, rewrote));
        if edit.is_some() {
            let keeps = s.place_bytes > 0;
            assert!(keeps, "step {n} ({what}): the place keeps its records");
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let want: Vec<_> = steps.iter().map(|&(what, _, want)| (what, want)).collect();
    // `None` where the place was filled from nothing.
    assert_eq!(seen, want, "records rewritten per step");
    // Drawn again unchanged, a list uploads what a frame that draws nothing
    // does: the passes' uniforms.
    let uploaded = |what| {
        let s: &wgpu_gameui::RenderStats = &stats[what];
        (s.buffer_bytes_uploaded, s.buffer_write_calls)
    };
    let nothing = uploaded("empty");
    assert_eq!(uploaded("again"), nothing, "bytes and writes");
    assert_eq!(uploaded("moved again"), nothing, "bytes and writes");
    // One panel recoloured uploads its record, in one write.
    let (bytes, writes) = uploaded("again");
    let panel = size_of::<wgpu_gameui::AnalyticInstance>() as u64;
    let one = (bytes + panel, writes + 1);
    assert_eq!(uploaded("panel"), one, "bytes and writes");
    assert_eq!(panel, 176);
}

/// Draw `lists` one over another in one frame on `run`.
fn layered(run: &mut Run, lists: &[DrawList]) -> Vec<u8> {
    let device = run.gpu.device().clone();
    let queue = run.gpu.queue().clone();
    let ui = run.gpu.renderer();
    ui.begin_frame();
    capture_with(&device, &queue, SIZE, Srgb::TRANSPARENT, |encoder, view| {
        for list in lists {
            ui.render(&device, &queue, encoder, view, SIZE, 1.0, list);
        }
    })
}

/// Nine lists in a frame, the ninth past the places the renderer keeps, and
/// the comparison turned off and on again: each frame looks as a fresh
/// renderer's does, and only the lists with a place keep their records.
#[test]
#[ignore = "requires a GPU adapter"]
fn nine_lists_and_the_comparison_turned_off_and_on() {
    let mut run = Run::new(&[], &Look::default(), &[]);
    run.gpu.renderer().set_check_plan(true);
    let lists = |run: &Run, hues: &[f32; 9]| -> Vec<DrawList> {
        hues.iter()
            .enumerate()
            .map(|(i, &hue)| {
                let mut list = run.list();
                let (x, y) = (8.0 + 76.0 * (i % 3) as f32, 8.0 + 50.0 * (i / 3) as f32);
                list.chrome_rect(
                    Rect::new(x, y, 64.0, 40.0),
                    4.0,
                    1.0,
                    [hue, 0.4, 0.6, 0.9],
                    [0.8; 4],
                );
                list.text(TextBlock::new(format!("L{i}"), x + 6.0, y + 8.0).with_size(14.0));
                list
            })
            .collect()
    };
    let base = [0.2; 9];
    let mut recoloured = base;
    recoloured[0] = 0.8;
    recoloured[8] = 0.8;
    // (what, hues, the comparison on)
    let steps = [
        ("first", base, true),
        ("again", base, true),
        ("first and ninth recoloured", recoloured, true),
        ("comparison off", base, false),
        ("comparison on", base, true),
        ("again", base, true),
        ("recoloured", recoloured, true),
    ];
    let mut failures = Vec::new();
    let mut seen = Vec::new();
    for (n, &(what, hues, on)) in steps.iter().enumerate() {
        run.gpu.renderer().set_frame_comparison(on);
        let lists = lists(&run, &hues);
        let kept = layered(&mut run, &lists);
        let s = run.gpu.renderer().frame_stats();
        seen.push((s.plans_replayed, s.records_rewritten, s.place_bytes > 0));
        let mut fresh = run.fresh();
        let from_nothing = layered(&mut fresh, &lists);
        if let Some((count, x, y)) = difference(&kept, &from_nothing, Tolerance::AtlasSampling) {
            failures.push(format!(
                "step {n} ({what}): {count} pixels differ, the first at ({x}, {y})"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(
        seen,
        [
            (0, 0, true),
            (8, 0, true),
            // The ninth list has no place: only the first's record is rewritten.
            (8, 1, true),
            // Off, nothing is kept.
            (0, 0, false),
            (0, 0, true),
            (8, 0, true),
            (8, 1, true),
        ],
        "plans replayed, records rewritten, and whether places keep anything, per step"
    );
}
