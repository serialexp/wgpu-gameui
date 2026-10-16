use std::iter::once;

use super::*;

fn grid() -> PixelGrid {
    PixelGrid {
        snap: GlyphSnap::default(),
        scale: 1.0,
        view_origin: [0.0, 0.0],
        hint: true,
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
    let total = out.text_vertices.len() as u32;
    assert_eq!(out.text_units[0].vertices.start, 0);
    for pair in out.text_units.windows(2) {
        assert_eq!(pair[0].vertices.end, pair[1].vertices.start);
    }
    assert_eq!(out.text_units[2].vertices.end, total);
    assert_eq!(out.text_commands, Vec::from_iter(once(0..total)));
    // Each block draws only its fill, six vertices a glyph.
    for (i, text) in texts.iter().enumerate() {
        let [shadow, glow, fill] = &out.block_sweeps[i];
        assert_eq!((len(shadow), len(glow)), (0, 0));
        assert_eq!(fill, &out.text_units[i].vertices);
        assert_eq!(len(fill), 6 * text.content.chars().count() as u32);
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
    let total = out.text_vertices.len() as u32;
    assert_eq!(out.text_units[0].vertices, 0..total);
    assert_eq!(out.text_commands, Vec::from_iter(once(0..total)));

    let [lit, plain, dark] = [0, 1, 2].map(|i| out.block_sweeps[i].clone());
    // Only the blocks with an effect draw its sweep.
    assert_eq!((len(&lit[0]), len(&lit[1])), (0, 18));
    assert_eq!((len(&plain[0]), len(&plain[1])), (0, 0));
    assert_eq!((len(&dark[0]), len(&dark[1])), (24, 0));
    // Every shadow, then every glow, then every fill, each in block order.
    assert_eq!(dark[0], 0..24);
    assert_eq!(lit[1], 24..42);
    assert_eq!(lit[2].start, 42);
    assert_eq!(lit[2].end, plain[2].start);
    assert_eq!(plain[2].end, dark[2].start);
    assert_eq!(dark[2].end, total);
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
    let total = out.text_vertices.len() as u32;
    assert_eq!(out.text_commands, [0..12, 12..total]);
    // A glyph's glow and fill: twelve vertices for "c", none for "".
    assert_eq!(total, 24);
    assert!(out.block_sweeps[3].iter().all(|r| r.is_empty()));
    assert_eq!(out.block_vertices(3).map(<[_]>::len).sum::<usize>(), 0);

    // Preparing again starts afresh.
    cpu.prepare_texts(&mut out, grid(), &texts[..1], once(0..1));
    assert_eq!(out.text_units.len(), 1);
    assert_eq!(out.text_commands, Vec::from_iter(once(0..6)));
    assert_eq!(out.block_sweeps.len(), 1);
}

#[test]
fn a_text_command_after_an_empty_one_has_its_own_vertices() {
    let (mut cpu, font) = cpu(None);
    let mut out = MsdfPrepared::default();
    let texts = [
        block("A", &font, 10.0),
        block("", &font, 40.0),
        block("BC", &font, 70.0),
    ];
    cpu.prepare_texts(&mut out, grid(), &texts, [0..1, 1..2, 2..3]);
    let sizes: Vec<_> = out.text_commands.iter().map(|r| r.len()).collect();
    assert_eq!(sizes, [6, 0, 12]);
    // The third command is "BC"'s quads, below the others.
    let third = &out.text_vertices[out.text_commands[2].start as usize..];
    assert!(third.iter().all(|v| v.position[1] > 60.0));
}

#[test]
fn a_later_unit_that_grows_the_atlas_leaves_earlier_ones_built_against_its_final_size() {
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
    let first = out.text_units[0].vertices.clone();
    let first = out.text_vertices[first.start as usize..first.end as usize].to_vec();
    // Built against the small atlas, the early unit's UVs would be these.
    assert_ne!(first, alone_out.text_vertices);

    // The atlas is final now, so preparing again builds every unit against
    // the same size, and nothing comes out different.
    let built = out.text_vertices.clone();
    both.prepare_texts(&mut out, grid(), &texts, [0..1, 1..2]);
    assert_eq!(both.atlas.width(), final_size);
    assert_eq!(out.text_vertices, built);
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
    fn each_icon_has_its_six_vertices_in_icon_order() {
        let (mut cpu, _) = cpu(None);
        let mut out = MsdfPrepared::default();
        let drawn = icons(&[PhosphorIcon::Plus, PhosphorIcon::Check, PhosphorIcon::X]);
        cpu.prepare_icons(&mut out, &drawn);
        assert_eq!(out.icon_starts, [0, 6, 12, 18]);
        assert_eq!(out.icon_range(1..3), 6..18);
        assert_eq!(out.icon_vertices.len(), 18);
    }

    #[test]
    fn an_icon_that_grows_the_atlas_leaves_earlier_ones_built_against_its_final_size() {
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
        assert_ne!(out.icon_vertices[..6], alone_out.icon_vertices[..]);

        let built = out.icon_vertices.clone();
        many.prepare_icons(&mut out, &drawn);
        assert_eq!(many.icon_atlas.width(), final_size);
        assert_eq!(out.icon_vertices, built);
    }
}
