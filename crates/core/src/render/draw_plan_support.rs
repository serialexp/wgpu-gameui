//! Real widget lists, with their text prepared on the CPU in the bundled font:
//! the fixtures the plan tests and the frame oracle share.

use std::collections::HashMap;
use std::sync::Arc;

use crate::render::AtlasRegion;
use crate::render::draw_box::Target;
use crate::render::draw_plan::{DrawPlan, ResolveTextures};
use crate::render::textures::{Binding, Resolved};
use crate::text::{
    FontSystemHandle, GlyphSnap, MsdfCpu, MsdfPrepared, PixelGrid, register_bundled_fonts,
    shared_font_system,
};
use crate::widgets::{DrawList, IconDraw, NineSliceDraw, PaintCmd};

/// Sprites stand in a texture per name; "video" is YUV, "missing" doesn't
/// resolve.
pub(crate) struct FakeTextures(HashMap<&'static str, Binding>);

impl FakeTextures {
    pub(crate) fn new() -> Self {
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
pub(crate) struct Lists {
    pub(crate) fs: FontSystemHandle,
    pub(crate) cpu: MsdfCpu,
    pub(crate) prepared: MsdfPrepared,
    pub(crate) textures: FakeTextures,
}

impl Lists {
    pub(crate) fn new() -> Self {
        let fs = shared_font_system();
        register_bundled_fonts(&fs).expect("register bundled fonts");
        Self {
            cpu: MsdfCpu::new(Arc::clone(&fs)),
            fs,
            prepared: MsdfPrepared::default(),
            textures: FakeTextures::new(),
        }
    }

    pub(crate) fn list(&self) -> DrawList {
        DrawList::with_font_system(Arc::clone(&self.fs))
    }

    /// Prepare `list`'s text and icons as the renderer does for `target`, and
    /// plan it.
    pub(crate) fn plan(&mut self, list: &DrawList, target: &Target, plan: &mut DrawPlan) {
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
