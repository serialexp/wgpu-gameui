use super::*;
use crate::render::draw_order::tests_support::{Rng, Unit, random_units};

const A: BatchKey = BatchKey(0);
const B: BatchKey = BatchKey(1);

fn px(x0: u32, y0: u32, x1: u32, y1: u32) -> PixelBox {
    PixelBox { x0, y0, x1, y1 }
}

/// Where each unit, by paint index, is drawn.
fn draw_positions(plan: &DrawPlan, count: usize) -> Vec<Option<usize>> {
    let mut at = vec![None; count];
    for (i, u) in plan.units().iter().enumerate() {
        at[u.paint as usize] = Some(i);
    }
    at
}

/// Every pair of units that share a pixel is drawn in paint order.
fn assert_overlaps_keep_paint_order(plan: &DrawPlan, units: &[Unit]) {
    let at = draw_positions(plan, units.len());
    for (b, (_, later)) in units.iter().enumerate() {
        for (a, (_, earlier)) in units[..b].iter().enumerate() {
            let overlap = later.iter().any(|p| earlier.iter().any(|q| p.overlaps(q)));
            if let (true, Some(pa), Some(pb)) = (overlap, at[a], at[b]) {
                assert!(pa < pb, "unit {a} overlaps {b} but draws after it");
            }
        }
    }
}

/// Batches cover the units in order, each of one key, and neighbours differ.
fn assert_batches_cover_units(plan: &DrawPlan) {
    let mut next = 0;
    for (i, batch) in plan.batches().iter().enumerate() {
        assert_eq!(batch.units.start, next);
        assert!(batch.units.start < batch.units.end);
        let units = &plan.units()[batch.units.start as usize..batch.units.end as usize];
        assert!(units.iter().all(|u| u.key == batch.key));
        if i > 0 {
            assert_ne!(plan.batches()[i - 1].key, batch.key);
        }
        next = batch.units.end;
    }
    assert_eq!(next as usize, plan.units().len());
}

#[test]
fn units_sort_by_order_then_key_then_paint_and_each_key_run_is_a_batch() {
    let units = [
        (B, vec![px(0, 0, 10, 10)]),
        (A, vec![px(5, 5, 15, 15)]),
        (A, vec![px(50, 50, 60, 60)]),
        (B, vec![px(80, 80, 90, 90)]),
        (A, vec![PixelBox::EMPTY]),
    ];
    let mut plan = DrawPlan::new();
    plan.plan((128, 128), &units);
    // B at 0 (paint 0, 3), then the A over it at 1, with the other A at 0.
    let order: Vec<_> = plan.units().iter().map(|u| (u.order, u.paint)).collect();
    assert_eq!(order, [(0, 2), (0, 0), (0, 3), (1, 1)]);
    let batches: Vec<_> = plan
        .batches()
        .iter()
        .map(|b| (b.key, b.units.clone()))
        .collect();
    assert_eq!(batches, [(A, 0..1), (B, 1..3), (A, 3..4)]);
    assert_eq!(plan.stats().units_ordered, 4);
}

#[test]
fn plans_keep_overlaps_in_paint_order_and_match_the_brute_force_batch_count() {
    let mut rng = Rng(11);
    let mut plan = DrawPlan::new();
    for layout in 0..300 {
        let (size, units) = random_units(&mut rng);
        plan.plan(size, &units);
        assert_overlaps_keep_paint_order(&plan, &units);
        assert_batches_cover_units(&plan);
        assert_eq!(
            plan.batches().len(),
            brute_force_batch_count(&units),
            "layout {layout}"
        );
    }
}

/// Real widget lists, with their text prepared on the CPU in the bundled font.
#[cfg(feature = "bundled-font")]
mod lists {
    use std::collections::HashMap;
    use std::sync::Arc;

    use super::*;
    use crate::layout::Rect;
    use crate::render::AtlasRegion;
    use crate::style::StyleResolver;
    use crate::text::{
        FontSystemHandle, GlyphSnap, MsdfCpu, PixelGrid, TextBlock, register_bundled_fonts,
        shared_font_system,
    };
    use crate::widgets::{Button, DrawContext, ScrollState, ScrollView};
    use crate::{FocusState, InputState, Theme};

    /// Sprites stand in a texture per name; "video" is YUV, "missing" doesn't
    /// resolve.
    struct FakeTextures(HashMap<&'static str, Binding>);

    impl FakeTextures {
        fn new() -> Self {
            Self(HashMap::from([
                ("a", Binding::Own(1)),
                ("b", Binding::Own(2)),
                ("a again", Binding::Own(1)),
                ("video", Binding::Yuv(3)),
            ]))
        }
    }

    /// A 16 px sprite in a texture of its own size, drawn from `binding`.
    fn resolved(binding: Binding) -> Resolved {
        Resolved {
            binding,
            region: AtlasRegion {
                x: 0,
                y: 0,
                w: 16,
                h: 16,
            },
            texture: (16, 16),
        }
    }

    impl ResolveTextures for FakeTextures {
        fn sprite(&self, draw: &IconDraw) -> Option<Resolved> {
            self.0.get(draw.icon_key.as_str()).copied().map(resolved)
        }

        fn nine_slice(&self, draw: &NineSliceDraw) -> Option<(Resolved, [u32; 4])> {
            let at = resolved(*self.0.get(draw.texture_key.as_str())?);
            Some((at, [4; 4]))
        }
    }

    /// A list's CPU side: a font system shared with its lists, and the text and
    /// icons prepared from them.
    struct Lists {
        fs: FontSystemHandle,
        cpu: MsdfCpu,
        prepared: MsdfPrepared,
        textures: FakeTextures,
    }

    impl Lists {
        fn new() -> Self {
            let fs = shared_font_system();
            register_bundled_fonts(&fs).expect("register bundled fonts");
            Self {
                cpu: MsdfCpu::new(Arc::clone(&fs)),
                fs,
                prepared: MsdfPrepared::default(),
                textures: FakeTextures::new(),
            }
        }

        fn list(&self) -> DrawList {
            DrawList::with_font_system(Arc::clone(&self.fs))
        }

        /// Prepare `list`'s text and icons as the renderer does for `target`, and
        /// plan it.
        fn plan(&mut self, list: &DrawList, target: &Target, plan: &mut DrawPlan) {
            let grid = PixelGrid::new(GlyphSnap::default(), target.scale, target.origin, true);
            let commands = list.paint_cmds.iter().filter_map(|cmd| match cmd {
                PaintCmd::Text { draws } => Some(draws.clone()),
                _ => None,
            });
            self.cpu
                .prepare_texts(&mut self.prepared, grid, &list.texts, commands);
            #[cfg(feature = "phosphor-icons")]
            self.cpu.prepare_icons(&mut self.prepared, &list.icons_msdf);
            plan.build(list, target, &self.prepared, &self.textures);
        }
    }

    fn units_of(plan: &DrawPlan) -> Vec<UnitRef> {
        let mut units: Vec<_> = plan.units().to_vec();
        units.sort_by_key(|u| u.paint);
        units.iter().map(|u| u.unit).collect()
    }

    #[test]
    fn a_grid_of_buttons_is_two_batches() {
        let mut lists = Lists::new();
        let mut list = lists.list();
        let theme = Theme::default();
        let mut focus = FocusState::default();
        let input = InputState::default();
        let target = Target::new((1200, 520), 1.0, [0.0, 0.0]);
        {
            let mut ctx = DrawContext::new(&mut list, &mut focus, &theme, &input, 1200.0, 520.0);
            for i in 0..100 {
                let (col, row) = ((i % 10) as f32, (i / 10) as f32);
                let rect = Rect::new(10.0 + 116.0 * col, 10.0 + 50.0 * row, 100.0, 32.0);
                Button::new("OK").draw(rect, &mut ctx);
            }
        }
        let mut plan = DrawPlan::new();
        lists.plan(&list, &target, &mut plan);
        let counts = (
            plan.units()
                .iter()
                .filter(|u| matches!(u.unit, UnitRef::Analytic(_)))
                .count(),
            plan.units()
                .iter()
                .filter(|u| matches!(u.unit, UnitRef::Text(_)))
                .count(),
        );
        assert_eq!(
            counts,
            (300, 100),
            "a plinth, face and highlight, and a label"
        );
        // Every box at order 0, every label over its box at 1.
        let keys: Vec<_> = plan.batches().iter().map(|b| b.units.len()).collect();
        assert_eq!(keys, [300, 100]);
    }

    #[test]
    fn a_scrolled_list_is_three_batches_and_its_hidden_rows_are_culled() {
        let mut lists = Lists::new();
        let mut list = lists.list();
        let theme = Theme::default();
        let style = StyleResolver::new(&theme);
        let mut input = InputState::default();
        let mut state = ScrollState::default();
        state.content_size = [300.0, 200.0 * 28.0];
        let view = ScrollView::new(Rect::new(0.0, 0.0, 320.0, 400.0)).vertical_only();
        view.draw(&mut state, &mut list, &style, &mut input, |list, inner| {
            for row in 0..200 {
                let y = inner.y + 28.0 * row as f32;
                list.chrome_rect(
                    Rect::new(inner.x, y, 300.0, 26.0),
                    3.0,
                    0.0,
                    [0.2, 0.2, 0.25, 1.0],
                    [0.0; 4],
                );
                list.text(
                    TextBlock::new(format!("Row {row}"), inner.x + 8.0, y + 6.0).with_size(13.0),
                );
            }
        });
        let target = Target::new((320, 400), 1.0, [0.0, 0.0]);
        let mut plan = DrawPlan::new();
        lists.plan(&list, &target, &mut plan);
        // Rows 0 to 14 are in view (the last in part); the rest are clipped away.
        let labels = plan
            .units()
            .iter()
            .filter(|u| matches!(u.unit, UnitRef::Text(_)))
            .count();
        assert_eq!(labels, 15);
        // The row backgrounds and the scrollbar's chrome, the scrollbar's arrows
        // (soup), both at order 0 as nothing of theirs overlaps; then the labels,
        // over the backgrounds.
        let batches: Vec<_> = plan
            .batches()
            .iter()
            .map(|b| {
                (
                    family(plan.units()[b.units.start as usize].unit),
                    b.units.len(),
                )
            })
            .collect();
        assert_eq!(batches, [("analytic", 28), ("soup", 2), ("text", 15)]);
    }

    fn family(unit: UnitRef) -> &'static str {
        match unit {
            UnitRef::Soup { .. } => "soup",
            UnitRef::Analytic(_) => "analytic",
            UnitRef::Circle(_) => "circle",
            UnitRef::Sprite { .. } => "sprite",
            UnitRef::NineSlice { .. } => "nine-slice",
            UnitRef::Text(_) => "text",
            #[cfg(feature = "phosphor-icons")]
            UnitRef::IconMsdf(_) => "icon",
        }
    }

    #[test]
    fn a_shadowed_text_command_is_one_unit_and_a_plain_one_a_unit_per_block() {
        let mut lists = Lists::new();
        let mut list = lists.list();
        list.text(TextBlock::new("one", 10.0, 10.0).with_shadow(0, 0, 0, 200, 1.0, 1.0, 1.0));
        list.text(TextBlock::new("two", 10.0, 300.0));
        list.chrome_rect(
            Rect::new(0.0, 100.0, 50.0, 50.0),
            0.0,
            0.0,
            [1.0; 4],
            [0.0; 4],
        );
        list.text(TextBlock::new("three", 10.0, 160.0));
        list.text(TextBlock::new("four", 10.0, 200.0));
        let target = Target::new((400, 400), 1.0, [0.0, 0.0]);
        let mut plan = DrawPlan::new();
        lists.plan(&list, &target, &mut plan);
        assert_eq!(
            units_of(&plan),
            [
                UnitRef::Text(0),
                UnitRef::Analytic(0),
                UnitRef::Text(1),
                UnitRef::Text(2)
            ]
        );
        // The shadowed command's unit holds both its blocks; the plain one's
        // blocks are units of their own.
        let blocks: Vec<_> = lists
            .prepared
            .text_units
            .iter()
            .map(|u| u.blocks.clone())
            .collect();
        assert_eq!(blocks, [0..2, 2..3, 3..4]);
    }

    #[test]
    fn a_shadowed_unit_with_a_block_off_the_target_still_draws() {
        let mut lists = Lists::new();
        let mut list = lists.list();
        list.text(TextBlock::new("seen", 10.0, 10.0).with_glow(255, 255, 255, 255, 2.0));
        list.text(TextBlock::new("far", 10.0, 5000.0));
        let target = Target::new((400, 400), 1.0, [0.0, 0.0]);
        let mut plan = DrawPlan::new();
        lists.plan(&list, &target, &mut plan);
        assert_eq!(units_of(&plan), [UnitRef::Text(0)]);
        // And wholly off the target it is culled.
        let mut list = lists.list();
        list.text(TextBlock::new("far", 10.0, 5000.0).with_glow(255, 255, 255, 255, 2.0));
        lists.plan(&list, &target, &mut plan);
        assert!(plan.units().is_empty());
        assert_eq!(plan.stats().units_ordered, 0);
    }

    #[test]
    fn sprites_batch_by_texture_and_unresolved_ones_are_left_out() {
        let mut lists = Lists::new();
        let mut list = lists.list();
        let at = |i: f32| Rect::new(10.0 + 40.0 * i, 10.0, 30.0, 30.0);
        for (i, name) in ["a", "b", "missing", "a again", "video"].iter().enumerate() {
            list.icon(name, at(i as f32).x, 10.0, 30.0, 30.0);
        }
        let target = Target::new((400, 100), 1.0, [0.0, 0.0]);
        let mut plan = DrawPlan::new();
        lists.plan(&list, &target, &mut plan);
        // Nothing overlaps: one batch per texture, "a" and "a again" together.
        let batches: Vec<_> = plan
            .batches()
            .iter()
            .map(|b| {
                plan.units()[b.units.start as usize..b.units.end as usize]
                    .iter()
                    .map(|u| u.paint)
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(batches, [vec![0, 3], vec![1], vec![4]]);
    }

    /// The units of each batch, by paint index.
    fn batch_paints(plan: &DrawPlan) -> Vec<Vec<u32>> {
        plan.batches()
            .iter()
            .map(|b| {
                plan.units()[b.units.start as usize..b.units.end as usize]
                    .iter()
                    .map(|u| u.paint)
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_paint_order_plan_is_a_batch_per_command_and_texture_and_culls_nothing() {
        let mut lists = Lists::new();
        let mut list = lists.list();
        let rect = |y: f32| Rect::new(0.0, y, 50.0, 20.0);
        list.chrome_rect(rect(0.0), 0.0, 0.0, [1.0; 4], [0.0; 4]);
        list.chrome_rect(rect(30.0), 0.0, 0.0, [1.0; 4], [0.0; 4]);
        list.text(TextBlock::new("label", 10.0, 100.0));
        // Off the target, and apart from the first two, so batching culls it.
        list.chrome_rect(rect(5000.0), 0.0, 0.0, [1.0; 4], [0.0; 4]);
        for (i, name) in ["a", "b", "missing", "a again"].iter().enumerate() {
            list.icon(name, 100.0 + 40.0 * i as f32, 10.0, 30.0, 30.0);
        }
        let target = Target::new((400, 400), 1.0, [0.0, 0.0]);
        let mut plan = DrawPlan::new();
        lists.plan(&list, &target, &mut plan);
        plan.build_paint_order(&list, &lists.prepared, &FakeTextures::new());
        // Every draw in paint order, the unresolved sprite left out; the two
        // rects share a command, and each change of texture cuts a batch.
        assert_eq!(
            batch_paints(&plan),
            [vec![0, 1], vec![2], vec![3], vec![4], vec![5], vec![7]]
        );
        assert_eq!(plan.stats().units_ordered, 0, "nothing is ordered");
        // Planned for the target, the far rect goes and like keys join.
        lists.plan(&list, &target, &mut plan);
        assert_eq!(
            batch_paints(&plan),
            [vec![0, 1], vec![4, 7], vec![5], vec![2]]
        );
    }

    #[test]
    fn batch_draws_copy_each_batch_in_draw_order_and_mark_small_textures() {
        use crate::render::batch_draws::{BatchDraw, BatchDraws, DrawKind};
        use crate::render::ui_renderer::icon_instance;

        let mut lists = Lists::new();
        let mut list = lists.list();
        for (i, name) in ["a", "b", "missing", "a again", "video"].iter().enumerate() {
            list.icon(name, 10.0 + 40.0 * i as f32, 10.0, 30.0, 30.0);
        }
        list.text(TextBlock::new("over", 10.0, 10.0));
        let target = Target::new((400, 100), 1.0, [0.0, 0.0]);
        let mut plan = DrawPlan::new();
        lists.plan(&list, &target, &mut plan);
        let mut draws = BatchDraws::default();
        draws.fill(&plan, &list, &lists.prepared);
        draws.push_soup(&[]);
        draws.push_soup(&[0, 1, 2]);
        let text = lists.prepared.text_instances.len() as u32;
        assert!(text > 0);
        let draw = |kind, range, small_texture| BatchDraw {
            kind,
            range,
            small_texture,
        };
        assert_eq!(
            draws.draws,
            [
                draw(DrawKind::Sprite(Binding::Own(1)), 0..2, Some(1)),
                draw(DrawKind::Sprite(Binding::Own(2)), 2..3, Some(2)),
                draw(DrawKind::Sprite(Binding::Yuv(3)), 3..4, None),
                draw(DrawKind::Text, 0..text, None),
                draw(DrawKind::Soup, 0..3, None),
            ]
        );
        // The sprites in draw order: "a" and "a again" together.
        let textures = FakeTextures::new();
        let expected: Vec<_> = [0, 3, 1, 4]
            .iter()
            .map(|&i| {
                let at = textures.sprite(&list.icons[i]).expect("resolves");
                icon_instance(&list.icons[i], &at)
            })
            .collect();
        assert_eq!(draws.sprites, expected);
        assert_eq!(draws.text.len() as u32, text);
        // Filling again starts over.
        draws.fill(&plan, &list, &lists.prepared);
        assert_eq!(draws.draws.len(), 4);
        assert_eq!(draws.sprites.len(), 4);
        assert!(draws.indices.is_empty());
    }

    /// Every kind of primitive, overlapping and apart, some of it off the
    /// target.
    fn fill_mixed(list: &mut DrawList) {
        for row in 0..40 {
            let y = 8.0 + 24.0 * row as f32;
            list.chrome_rect(
                Rect::new(4.0, y, 300.0, 20.0),
                3.0,
                1.0,
                [0.2, 0.2, 0.25, 1.0],
                [0.4; 4],
            );
            list.text(TextBlock::new(format!("Row {row}"), 10.0, y + 3.0).with_size(13.0));
            list.circle((290.0, y + 10.0), 6.0, [1.0, 0.0, 0.0, 1.0]);
            list.icon(["a", "b", "video"][row % 3], 260.0, y, 16.0, 16.0);
            #[cfg(feature = "phosphor-icons")]
            list.phosphor_icon(
                Rect::new(240.0, y, 16.0, 16.0),
                crate::render::PhosphorIcon::Check,
                [1.0; 4],
            );
        }
        list.text(TextBlock::new("Shadowed", 20.0, 20.0).with_shadow(0, 0, 0, 200, 1.0, 1.0, 1.0));
        list.nine_slice(320.0, 10.0, 60.0, 40.0, "a");
        list.radial_gradient(Rect::new(320.0, 60.0, 60.0, 60.0), [1.0; 4], [0.0; 4], 24);
    }

    #[test]
    fn planning_a_warmed_frame_allocates_nothing() {
        use crate::counting_alloc::count_allocations;
        use crate::render::batch_draws::BatchDraws;

        let mut lists = Lists::new();
        let target = Target::new((400, 600), 1.25, [3.0, 5.0]);
        let mut plan = DrawPlan::new();
        let mut draws = BatchDraws::default();
        let frame = |lists: &Lists| {
            let mut list = lists.list();
            fill_mixed(&mut list);
            list
        };
        // Two warm-up frames reach every buffer's high-water mark and fill
        // the glyph atlases.
        for _ in 0..2 {
            let list = frame(&lists);
            lists.plan(&list, &target, &mut plan);
            draws.fill(&plan, &list, &lists.prepared);
            draws.push_soup(&list.indices[..3]);
        }
        // Recording the list is the caller's; planning and filling it is ours.
        let list = frame(&lists);
        let ((), allocations) = count_allocations(|| {
            lists.plan(&list, &target, &mut plan);
            draws.fill(&plan, &list, &lists.prepared);
            draws.push_soup(&list.indices[..3]);
        });
        assert!(plan.batches().len() > 4, "{:?}", plan.batches().len());
        assert!(!draws.draws.is_empty());
        assert_eq!(allocations, 0);
    }
}
