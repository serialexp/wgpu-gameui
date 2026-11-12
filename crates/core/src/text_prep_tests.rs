use std::iter::once;

use super::*;

fn grid() -> PixelGrid {
    PixelGrid {
        snap: GlyphSnap::default(),
        scale: 1.0,
        view_origin: [0.0, 0.0],
        hint: true,
        target: None,
    }
}

/// A CPU half on a font system of its own, its glyph and icon atlases
/// starting at `atlas` px square if given, and a bundled Plex Sans handle.
fn cpu(atlas: Option<u32>) -> (MsdfCpu, FontHandle) {
    let fs = shared_font_system();
    let font = register_bundled_fonts(&fs).expect("register bundled fonts");
    let mut cpu = MsdfCpu::new(fs);
    if let Some(size) = atlas {
        cpu.atlas = MsdfGlyphAtlas::new().starting_at(size);
        #[cfg(feature = "phosphor-icons")]
        {
            cpu.icon_atlas =
                MsdfGlyphAtlas::with_params(ICON_REF_PX, DEFAULT_PX_RANGE).starting_at(size);
        }
    }
    (cpu, font)
}

fn block(text: &str, font: &FontHandle, y: f32) -> TextBlock {
    TextBlock::new(text, 10.0, y)
        .with_size(18.0)
        .with_font(font.clone())
}

fn len(range: &Range<u32>) -> u32 {
    range.end - range.start
}

#[test]
fn a_command_without_shadows_or_glows_is_a_unit_per_block() {
    let (mut cpu, font) = cpu(None);
    let mut out = MsdfPrepared::default();
    let texts = [
        block("one", &font, 10.0),
        block("two", &font, 40.0),
        block("three", &font, 70.0),
    ];
    cpu.prepare_texts(&mut out, grid(), &texts, once(0..3));

    let blocks: Vec<_> = out.text_units.iter().map(|u| u.blocks.clone()).collect();
    assert_eq!(blocks, [0..1, 1..2, 2..3]);
    // The units follow one another and make up the command.
    let total = out.text_instances.len() as u32;
    assert_eq!(out.text_units[0].instances.start, 0);
    for pair in out.text_units.windows(2) {
        assert_eq!(pair[0].instances.end, pair[1].instances.start);
    }
    assert_eq!(out.text_units[2].instances.end, total);
    assert_eq!(out.text_commands, Vec::from_iter(once(0..total)));
    // Each block draws only its fill, one instance a glyph.
    for (i, text) in texts.iter().enumerate() {
        let [shadow, glow, fill] = &out.block_sweeps[i];
        assert_eq!((len(shadow), len(glow)), (0, 0));
        assert_eq!(fill, &out.text_units[i].instances);
        assert_eq!(len(fill), text.content.chars().count() as u32);
    }
}

#[test]
fn a_command_with_a_shadow_or_glow_is_one_unit_drawn_in_three_sweeps() {
    let (mut cpu, font) = cpu(None);
    let mut out = MsdfPrepared::default();
    let texts = [
        block("lit", &font, 10.0).with_glow(255, 200, 0, 255, 2.0),
        block("plain", &font, 40.0),
        block("dark", &font, 70.0).with_shadow(0, 0, 0, 200, 1.0, 1.0, 1.0),
    ];
    cpu.prepare_texts(&mut out, grid(), &texts, once(0..3));

    assert_eq!(out.text_units.len(), 1);
    assert_eq!(out.text_units[0].blocks, 0..3);
    let total = out.text_instances.len() as u32;
    assert_eq!(out.text_units[0].instances, 0..total);
    assert_eq!(out.text_commands, Vec::from_iter(once(0..total)));

    let [lit, plain, dark] = [0, 1, 2].map(|i| out.block_sweeps[i].clone());
    // Only the blocks with an effect draw its sweep.
    assert_eq!((len(&lit[0]), len(&lit[1])), (0, 3));
    assert_eq!((len(&plain[0]), len(&plain[1])), (0, 0));
    assert_eq!((len(&dark[0]), len(&dark[1])), (4, 0));
    // Every shadow, then every glow, then every fill, each in block order.
    assert_eq!(dark[0], 0..4);
    assert_eq!(lit[1], 4..7);
    assert_eq!(lit[2].start, 7);
    assert_eq!(lit[2].end, plain[2].start);
    assert_eq!(plain[2].end, dark[2].start);
    assert_eq!(dark[2].end, total);
}

/// Blocks in two commands, one of them a unit with a shadow and a glow, coloured
/// by `hue`.
fn recoloured(font: &FontHandle, hue: u8) -> [TextBlock; 4] {
    [
        block("lit", font, 10.0)
            .with_color(hue, 20, 20)
            .with_glow(hue, 200, 0, 255, 2.0),
        block("plain", font, 40.0).with_color(20, hue, 20),
        block("dark", font, 70.0)
            .with_color(20, 20, hue)
            .with_shadow(hue, 0, 0, 200, 1.0, 1.0, 1.0)
            .with_outline(0, hue, 0, 255, 1.0),
        block("still", font, 100.0),
    ]
}

/// A block only recoloured is prepared again in place, and what comes out is
/// what preparing the list from nothing makes; a block the same is not looked
/// at.
#[test]
fn refreshing_recoloured_blocks_matches_preparing_them_from_nothing() {
    let (mut cpu, font) = cpu(None);
    let commands = || [0..3, 3..4];
    let mut kept = MsdfPrepared::default();
    cpu.prepare_texts(&mut kept, grid(), &recoloured(&font, 40), commands());

    let next = recoloured(&font, 220);
    let changes = [Change::Look, Change::Look, Change::Look, Change::Same];
    assert!(cpu.refresh_texts(&mut kept, grid(), &next, &changes));
    let mut fresh = MsdfPrepared::default();
    cpu.prepare_texts(&mut fresh, grid(), &next, commands());

    assert_eq!(kept.text_instances, fresh.text_instances);
    assert_eq!(kept.text_units, fresh.text_units);
    assert_eq!(kept.text_commands, fresh.text_commands);
    assert_eq!(kept.block_sweeps, fresh.block_sweeps);
    // The three recoloured blocks were placed again, "still" was not.
    let placed = ["lit", "plain", "dark"].map(str::len).iter().sum::<usize>() as u32;
    assert_eq!(
        kept.work,
        PrepareWork {
            blocks_looked_up: 3,
            glyphs_placed: placed,
            instances_built: fresh.work.instances_built - "still".len() as u32,
        }
    );

    // Nothing changed: nothing is placed or built.
    assert!(cpu.refresh_texts(&mut kept, grid(), &next, &[Change::Same; 4]));
    assert_eq!(kept.work, PrepareWork::default());
    assert_eq!(kept.text_instances, fresh.text_instances);
}

/// `blocks`, each with the layout a prepared text makes for it.
fn with_prepared(fs: &FontSystemHandle, blocks: &[TextBlock]) -> Vec<TextBlock> {
    let mut measurer = crate::TextMeasurer::with_font_system(fs.clone());
    blocks
        .iter()
        .map(|block| {
            let layout = measurer.prepare(block).layout(&mut measurer, block);
            block.clone().with_prepared(layout)
        })
        .collect()
}

fn shaped(fs: &FontSystemHandle) -> u64 {
    fs.lock().unwrap().layout_stats().shaped
}

/// Prepared blocks place the glyphs the shared cache gives, shaping nothing,
/// whether prepared from nothing or refreshed in place; once the fonts have
/// changed their layouts are stale and the cache's are placed.
#[test]
fn prepared_blocks_place_what_the_cache_gives_and_shape_nothing() {
    let (mut cpu, font) = cpu(None);
    let fs = Arc::clone(&cpu.font_system);
    let commands = || [0..3, 3..4];
    let plain = recoloured(&font, 40);
    let prepared = with_prepared(&fs, &plain);
    let start = shaped(&fs);
    let mut kept = MsdfPrepared::default();
    cpu.prepare_texts(&mut kept, grid(), &prepared, commands());
    assert_eq!(shaped(&fs), start, "placed from the prepared layouts");
    let mut fresh = MsdfPrepared::default();
    cpu.prepare_texts(&mut fresh, grid(), &plain, commands());
    assert_eq!(shaped(&fs) - start, 4, "the cache shapes each block");
    assert_eq!(kept.text_instances, fresh.text_instances);
    assert_eq!(kept.block_sweeps, fresh.block_sweeps);

    // Recoloured, the layouts still hold: a refresh places them again.
    let next: Vec<TextBlock> = recoloured(&font, 220)
        .into_iter()
        .zip(&prepared)
        .map(|(block, was)| block.with_prepared(was.prepared.clone().unwrap()))
        .collect();
    let changes = [Change::Look, Change::Look, Change::Look, Change::Same];
    let start = shaped(&fs);
    assert!(cpu.refresh_texts(&mut kept, grid(), &next, &changes));
    let mut fresh = MsdfPrepared::default();
    cpu.prepare_texts(&mut fresh, grid(), &recoloured(&font, 220), commands());
    assert_eq!(kept.text_instances, fresh.text_instances);
    assert_eq!(kept.work.blocks_looked_up, 3);
    assert_eq!(
        shaped(&fs),
        start,
        "the cache had them; the handles placed them"
    );

    // New fonts: the handles are stale, so the cache shapes the blocks again.
    fs.lock().unwrap().clear_caches();
    let start = shaped(&fs);
    let mut stale = MsdfPrepared::default();
    cpu.prepare_texts(&mut stale, grid(), &prepared, commands());
    assert_eq!(shaped(&fs) - start, 4, "stale handles are not used");
    let mut fresh = MsdfPrepared::default();
    cpu.prepare_texts(&mut fresh, grid(), &plain, commands());
    assert_eq!(stale.text_instances, fresh.text_instances);
}

/// A block changed after its layout was attached is laid out as it is now.
#[test]
fn a_handle_on_a_block_changed_since_is_not_used() {
    let (mut cpu, font) = cpu(None);
    let fs = Arc::clone(&cpu.font_system);
    let plain = [block("before", &font, 10.0)];
    let prepared = with_prepared(&fs, &plain);
    let mut changed = prepared.clone();
    changed[0].content = "after, and longer".into();
    changed[0].max_width = 40.0;
    let mut got = MsdfPrepared::default();
    cpu.prepare_texts(&mut got, grid(), &changed, once(0..1));
    let mut cleared = changed.clone();
    cleared[0].prepared = None;
    let mut want = MsdfPrepared::default();
    cpu.prepare_texts(&mut want, grid(), &cleared, once(0..1));
    assert_eq!(got.text_instances, want.text_instances);
    assert_eq!(got.text_instances.len(), "after,andlonger".len());
}

/// What a refresh cannot do it declines, and the caller prepares from nothing.
#[test]
fn a_refresh_declines_a_block_that_moved_or_a_list_of_another_shape() {
    let (mut cpu, font) = cpu(None);
    let texts = recoloured(&font, 40);
    let mut kept = MsdfPrepared::default();
    cpu.prepare_texts(&mut kept, grid(), &texts, [0..3, 3..4]);

    let moved = [Change::Same, Change::Place, Change::Same, Change::Same];
    assert!(!cpu.refresh_texts(&mut kept, grid(), &texts, &moved));
    assert!(!cpu.refresh_texts(&mut kept, grid(), &texts[..3], &[Change::Same; 3]));
    // Told a block only looks different when its text grew: it places more
    // glyphs than it had.
    let mut grown = texts.clone();
    grown[3] = block("stiller", &font, 100.0);
    let look = [Change::Same, Change::Same, Change::Same, Change::Look];
    assert!(!cpu.refresh_texts(&mut kept, grid(), &grown, &look));
}

#[test]
fn each_command_forms_its_own_units_in_paint_order() {
    let (mut cpu, font) = cpu(None);
    let mut out = MsdfPrepared::default();
    let texts = [
        block("a", &font, 10.0),
        block("b", &font, 40.0),
        block("c", &font, 70.0).with_glow(255, 255, 255, 255, 2.0),
        block("", &font, 100.0),
    ];
    cpu.prepare_texts(&mut out, grid(), &texts, [0..2, 2..4]);

    let blocks: Vec<_> = out.text_units.iter().map(|u| u.blocks.clone()).collect();
    assert_eq!(blocks, [0..1, 1..2, 2..4]);
    let total = out.text_instances.len() as u32;
    assert_eq!(out.text_commands, [0..2, 2..total]);
    // A glyph's glow and fill: two instances for "c", none for "".
    assert_eq!(total, 4);
    assert!(out.block_sweeps[3].iter().all(|r| r.is_empty()));
    assert_eq!(out.block_instances(3).map(<[_]>::len).sum::<usize>(), 0);

    // Preparing again starts afresh.
    cpu.prepare_texts(&mut out, grid(), &texts[..1], once(0..1));
    assert_eq!(out.text_units.len(), 1);
    assert_eq!(out.text_commands, Vec::from_iter(once(0..1)));
    assert_eq!(out.block_sweeps.len(), 1);
}

#[test]
fn a_text_command_after_an_empty_one_has_its_own_instances() {
    let (mut cpu, font) = cpu(None);
    let mut out = MsdfPrepared::default();
    let texts = [
        block("A", &font, 10.0),
        block("", &font, 40.0),
        block("BC", &font, 70.0),
    ];
    cpu.prepare_texts(&mut out, grid(), &texts, [0..1, 1..2, 2..3]);
    let sizes: Vec<_> = out.text_commands.iter().map(|r| r.len()).collect();
    assert_eq!(sizes, [1, 0, 2]);
    // The third command is "BC"'s quads, below the others.
    let third = &out.text_instances[out.text_commands[2].start as usize..];
    assert!(third.iter().flat_map(|q| q.corners()).all(|c| c[1] > 60.0));
}

#[test]
fn a_later_unit_that_grows_the_atlas_leaves_earlier_ones_unchanged() {
    const SMALL: u32 = 128;
    let early = |font: &FontHandle| block("ab", font, 10.0);
    let late = |font: &FontHandle| block("ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789", font, 40.0);

    // The early text alone fits the small atlas.
    let (mut alone, font) = cpu(Some(SMALL));
    let mut alone_out = MsdfPrepared::default();
    alone.prepare_texts(&mut alone_out, grid(), &[early(&font)], once(0..1));
    let early_size = alone.atlas.width();

    // After it, as a separate unit, comes text that doesn't.
    let (mut both, font) = cpu(Some(SMALL));
    let mut out = MsdfPrepared::default();
    let texts = [early(&font), late(&font)];
    both.prepare_texts(&mut out, grid(), &texts, [0..1, 1..2]);
    let final_size = both.atlas.width();
    assert!(
        final_size > early_size,
        "the late unit grows the atlas: {early_size} then {final_size}"
    );
    let first = out.text_units[0].instances.clone();
    let first = out.text_instances[first.start as usize..first.end as usize].to_vec();
    // A quad names its tile in texels, and growing the atlas moves no tile,
    // so the early unit is the same whether the atlas grew after it or not.
    assert_eq!(first, alone_out.text_instances);

    // The atlas is final now, so preparing again builds every unit against
    // the same size, and nothing comes out different.
    let built = out.text_instances.clone();
    both.prepare_texts(&mut out, grid(), &texts, [0..1, 1..2]);
    assert_eq!(both.atlas.width(), final_size);
    assert_eq!(out.text_instances, built);
}

#[cfg(feature = "phosphor-icons")]
mod icons {
    use super::*;
    use crate::render::PhosphorIcon;
    use crate::widgets::DrawList;

    fn icons(which: &[PhosphorIcon]) -> Vec<IconMsdf> {
        let mut list = DrawList::new();
        for (i, &icon) in which.iter().enumerate() {
            list.phosphor_icon(
                Rect::new(10.0 + 30.0 * i as f32, 10.0, 24.0, 24.0),
                icon,
                [1.0; 4],
            );
        }
        list.icons_msdf
    }

    #[test]
    fn each_icon_has_its_instance_in_icon_order() {
        let (mut cpu, _) = cpu(None);
        let mut out = MsdfPrepared::default();
        let drawn = icons(&[PhosphorIcon::Plus, PhosphorIcon::Check, PhosphorIcon::X]);
        cpu.prepare_icons(&mut out, &drawn);
        assert_eq!(out.icon_starts, [0, 1, 2, 3]);
        assert_eq!(out.icon_range(1..3), 1..3);
        assert_eq!(out.icon_instances.len(), 3);
    }

    #[test]
    fn an_icon_that_grows_the_atlas_leaves_earlier_ones_unchanged() {
        const SMALL: u32 = 64;
        let all = PhosphorIcon::ALL;
        let (mut alone, _) = cpu(Some(SMALL));
        let mut alone_out = MsdfPrepared::default();
        alone.prepare_icons(&mut alone_out, &icons(&all[..1]));
        let early_size = alone.icon_atlas.width();

        let (mut many, _) = cpu(Some(SMALL));
        let mut out = MsdfPrepared::default();
        let drawn = icons(&all[..all.len().min(24)]);
        many.prepare_icons(&mut out, &drawn);
        let final_size = many.icon_atlas.width();
        assert!(
            final_size > early_size,
            "the later icons grow the atlas: {early_size} then {final_size}"
        );
        assert_eq!(out.icon_instances[..1], alone_out.icon_instances[..]);

        let built = out.icon_instances.clone();
        many.prepare_icons(&mut out, &drawn);
        assert_eq!(many.icon_atlas.width(), final_size);
        assert_eq!(out.icon_instances, built);
    }
}

#[test]
fn the_target_is_measured_in_logical_px_at_any_scale() {
    let (mut cpu, font) = cpu(None);
    // A 600 x 200 device-px target at scale 2 shows 300 x 100 logical px.
    let at_scale_2 =
        PixelGrid::new(GlyphSnap::default(), 2.0, [0.0, 0.0], true).with_target(300.0, 100.0);
    assert_eq!(at_scale_2.viewport(), Some([0.0, 0.0, 300.0, 100.0]));
    let from =
        PixelGrid::new(GlyphSnap::default(), 2.0, [10.0, 20.0], true).with_target(300.0, 100.0);
    assert_eq!(from.viewport(), Some([10.0, 20.0, 310.0, 120.0]));

    // A label at y = 80 logical is on screen, though it is below 50 (the
    // height in logical px if the scale were divided out twice).
    let texts = [block("visible", &font, 80.0), block("below", &font, 140.0)];
    let mut out = MsdfPrepared::default();
    cpu.prepare_texts(&mut out, at_scale_2, &texts, once(0..2));
    assert_eq!(out.block_glyphs_len(0), 7);
    assert_eq!(out.block_glyphs_len(1), 0);
}

#[test]
fn glyphs_outside_the_target_are_not_placed_and_those_in_view_are_unchanged() {
    let (mut cpu, font) = cpu(None);
    let text = vec!["abcdefgh"; 60].join("\n");
    let tall = [block(&text, &font, 0.0).with_max_width(300.0)];

    let mut all = MsdfPrepared::default();
    cpu.prepare_texts(&mut all, grid(), &tall, once(0..1));

    // A target 100 px tall shows the first few of the sixty lines (18 px
    // font, so about 23 px a line); a taller block starting above the target
    // shows a middle slice.
    let mut seen = MsdfPrepared::default();
    cpu.prepare_texts(
        &mut seen,
        grid().with_target(300.0, 100.0),
        &tall,
        once(0..1),
    );
    assert!(!seen.text_instances.is_empty());
    assert!(
        seen.text_instances.len() * 4 < all.text_instances.len(),
        "{} of {} glyphs placed for a target showing a few lines",
        seen.text_instances.len(),
        all.text_instances.len()
    );
    assert_eq!(
        seen.text_instances[..],
        all.text_instances[..seen.text_instances.len()],
        "the glyphs placed are the same ones, quad for quad"
    );

    // Scrolled so the target shows a slice from the middle: every glyph that
    // could touch the target is placed, and none of the first lines is.
    let scrolled = [block(&text, &font, -600.0).with_max_width(300.0)];
    let mut middle = MsdfPrepared::default();
    cpu.prepare_texts(
        &mut middle,
        grid().with_target(300.0, 100.0),
        &scrolled,
        once(0..1),
    );
    let mut whole = MsdfPrepared::default();
    cpu.prepare_texts(&mut whole, grid(), &scrolled, once(0..1));
    assert!(!middle.text_instances.is_empty());
    assert!(middle.text_instances.len() * 4 < whole.text_instances.len());
    for quad in &middle.text_instances {
        assert!(
            whole.text_instances.contains(quad),
            "a placed glyph is one the whole block places"
        );
    }
    // Every quad of the whole block that overlaps the target is among them.
    for quad in &whole.text_instances {
        let ys = quad.corners().map(|c| c[1]);
        let (top, bottom) = (
            ys.iter().cloned().fold(f32::MAX, f32::min),
            ys.iter().cloned().fold(f32::MIN, f32::max),
        );
        if bottom > 0.0 && top < 100.0 {
            assert!(
                middle.text_instances.contains(quad),
                "a glyph that reaches the target was dropped"
            );
        }
    }

    // A block wholly outside the target places nothing.
    let away = [block(&text, &font, 5_000.0).with_max_width(300.0)];
    let mut none = MsdfPrepared::default();
    cpu.prepare_texts(
        &mut none,
        grid().with_target(300.0, 100.0),
        &away,
        once(0..1),
    );
    assert!(none.text_instances.is_empty());
}
