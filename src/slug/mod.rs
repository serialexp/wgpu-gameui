//! Opt-in Slug analytic text experiment. Does not replace the default MSDF path.
//! Natural unhinted quadratic outlines, shared cosmic layouts, baseline snapping
//! identical to default MSDF. Effects and styled spans are explicitly unsupported.
pub mod outline;
mod gpu;
pub use gpu::SlugGpu;
use std::collections::HashMap;
use bytemuck::{Pod, Zeroable};
use cosmic_text::fontdb;
use crate::{FontSystemHandle, TextBlock};
use crate::shaping::LayoutSpec;
use outline::{Glyph, Outlines};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Instance {
    bounds: [f32;4],
    origin_size: [f32;4],
    color: [f32;4],
    clip: [f32;4],
    band_start: u32,
    padding: [u32;3],
}
/// CPU cache and reusable per-frame instance storage. Cache includes empty glyphs.
#[derive(Default)]
pub struct SlugScene {
    pub outlines: Outlines,
    cache: HashMap<(fontdb::ID,u16,u16),Option<Glyph>>,
    pub(crate) instances: Vec<Instance>,
    /// Resolved faces and missing glyphs, collected once per face/glyph.
    pub fonts: HashMap<fontdb::ID,String>,
    pub missing: Vec<(fontdb::ID,u16)>,
    pub shaped_glyphs: usize,
}
impl SlugScene {
    pub fn glyph_count(&self) -> usize { self.instances.len() }
    pub fn cached_glyphs(&self) -> usize { self.cache.len() }
    /// Baseline policy matches MSDF's default; `snap_baseline=false` provides
    /// the common fully fractional/unhinted placement baseline.
    pub fn prepare(&mut self, fonts:&FontSystemHandle, blocks:&[TextBlock], scale:f32, snap_baseline:bool) -> Result<(),String> {
        if !scale.is_finite() || scale<=0.0 { return Err("invalid scale".into()); }
        self.instances.clear(); self.shaped_glyphs=0;
        let mut shared=fonts.lock().map_err(|_| "font lock poisoned")?;
        for block in blocks {
            if block.outline.is_some() || block.shadow.is_some() || block.glow.is_some() || !block.spans.is_empty() || !block.style_ranges.is_empty() { return Err("Slug experiment: effects/spans unsupported".into()); }
            let (layout,system)=shared.layout_and_fonts(&LayoutSpec::of_block(block),&block.content);
            for g in &layout.glyphs {
                self.shaped_glyphs+=1;
                let key=(g.font_id,g.font_weight.0,g.glyph_id);
                if !self.cache.contains_key(&key) {
                    let font=system.get_font(g.font_id,g.font_weight).ok_or("resolved font unavailable")?;
                    let face=system.db().face(g.font_id).ok_or("resolved face unavailable")?;
                    self.fonts.entry(g.font_id).or_insert_with(|| format!("{} index={} weight={} style={:?}",face.post_script_name,face.index,face.weight.0,face.style));
                    if g.glyph_id==0 { self.missing.push((g.font_id,g.glyph_id)); }
                    let glyph=self.outlines.append(font.data(),face.index,g.glyph_id)?;
                    self.cache.insert(key,glyph);
                }
                let Some(glyph)=self.cache[&key] else { continue };
                let baseline=(block.y+g.rel_y)*scale;
                let clip=block.clip.map(|r| [r.x*scale,r.y*scale,(r.x+r.width)*scale,(r.y+r.height)*scale]).unwrap_or([-1e9,-1e9,1e9,1e9]);
                let color=crate::text::color_to_rgba(block.color);
                self.instances.push(Instance { bounds:glyph.bounds, origin_size:[(block.x+g.rel_x)*scale,if snap_baseline {baseline.round()} else {baseline},g.font_size*scale,0.0], color,clip,band_start:glyph.band_start,padding:[0;3] });
            }
        }
        Ok(())
    }
}
