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
    AtlasId, DrawList, FontHandle, HeadlessGpu, Placement, SpriteId, Srgb, TextBlock,
    load_font_bytes,
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
        ui.register_nine_slice("panel", panel, [look.border; 4]);
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
            art: Art { red, blue, panel },
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
    assert!(
        stats[1].snapshot_bytes > 0,
        "the kept frames hold something"
    );
    assert!(stats[1].records_same > 0 && stats[1].records_place == 0);
    assert!(stats[2].records_look > 0 && stats[2].records_place == 0);
    assert!(stats[3].records_place > 0);
}
