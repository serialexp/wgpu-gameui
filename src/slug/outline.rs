//! Natural (unhinted) quadratic outline preparation, independent of wgpu.
use bytemuck::{Pod, Zeroable};
use ttf_parser::{GlyphId, OutlineBuilder};

pub const BAND_COUNT: u32 = 8;
const EPSILON: f32 = 1.0 / 1024.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Curve {
    pub p0: [f32; 2],
    pub p1: [f32; 2],
    pub p2: [f32; 2],
    pub padding: [f32; 2],
}
impl Curve {
    fn min(&self, axis: usize) -> f32 {
        self.p0[axis].min(self.p1[axis]).min(self.p2[axis])
    }
    fn max(&self, axis: usize) -> f32 {
        self.p0[axis].max(self.p1[axis]).max(self.p2[axis])
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Band {
    pub start: u32,
    pub count: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub bounds: [f32; 4], // min x/y, max x/y, y-up em coordinates
    pub band_start: u32,
}

/// Append-only flat cache. Scratch is retained across glyph preparations.
#[derive(Default)]
pub struct Outlines {
    pub curves: Vec<Curve>,
    pub bands: Vec<Band>,
    pub indices: Vec<u32>,
    scratch: Builder,
    order: Vec<u32>,
}
impl Outlines {
    /// Reject cubics explicitly; no fixed-resolution flattening or silent loss.
    pub fn append(&mut self, data: &[u8], index: u32, id: u16) -> Result<Option<Glyph>, String> {
        let face = ttf_parser::Face::parse(data, index).map_err(|e| format!("font parse: {e:?}"))?;
        self.scratch.curves.clear();
        self.scratch.unsupported = false;
        self.scratch.scale = 1.0 / face.units_per_em() as f32;
        let bounds = face.outline_glyph(GlyphId(id), &mut self.scratch);
        if self.scratch.unsupported {
            return Err(format!("glyph {id}: cubic outline unsupported by quadratic Slug experiment"));
        }
        let Some(bounds) = bounds else { return Ok(None) };
        if self.scratch.curves.is_empty() { return Ok(None); }
        let s = self.scratch.scale;
        let bounds = [bounds.x_min as f32*s, bounds.y_min as f32*s, bounds.x_max as f32*s, bounds.y_max as f32*s];
        if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] { return Ok(None); }
        let curve_start = u32::try_from(self.curves.len()).map_err(|_| "curve cache overflow")?;
        let band_start = u32::try_from(self.bands.len()).map_err(|_| "band cache overflow")?;
        // Horizontal bands select by y, sorted descending max x; vertical is converse.
        for axis in [1, 0] {
            let thickness = (bounds[axis+2] - bounds[axis]) / BAND_COUNT as f32;
            for band in 0..BAND_COUNT {
                let lo = bounds[axis] + band as f32 * thickness - EPSILON;
                let hi = lo + thickness + 2.0 * EPSILON;
                self.order.clear();
                for (i, c) in self.scratch.curves.iter().enumerate() {
                    if c.min(axis) != c.max(axis) && c.min(axis) <= hi && c.max(axis) >= lo {
                        self.order.push(i as u32);
                    }
                }
                let curves = &self.scratch.curves;
                self.order.sort_unstable_by(|&a, &b| curves[b as usize].max(1-axis).total_cmp(&curves[a as usize].max(1-axis)));
                let start = u32::try_from(self.indices.len()).map_err(|_| "index cache overflow")?;
                self.bands.push(Band { start, count: self.order.len() as u32 });
                self.indices.extend(self.order.iter().map(|i| curve_start + i));
            }
        }
        self.curves.extend_from_slice(&self.scratch.curves);
        Ok(Some(Glyph { bounds, band_start }))
    }
    pub fn bytes(&self) -> usize {
        self.curves.len()*32 + self.bands.len()*8 + self.indices.len()*4
    }
}
#[derive(Default)]
struct Builder {
    curves: Vec<Curve>,
    current: [f32; 2],
    first: [f32; 2],
    scale: f32,
    unsupported: bool,
}
impl Builder {
    fn point(&self, x: f32, y: f32) -> [f32; 2] { [x*self.scale, y*self.scale] }
}
impl OutlineBuilder for Builder {
    fn move_to(&mut self, x:f32,y:f32) { self.current=self.point(x,y); self.first=self.current; }
    fn line_to(&mut self,x:f32,y:f32) { let p=self.point(x,y); self.curves.push(Curve{p0:self.current,p1:p,p2:p,padding:[0.0;2]}); self.current=p; }
    fn quad_to(&mut self,x1:f32,y1:f32,x:f32,y:f32) { let p=self.point(x,y); self.curves.push(Curve{p0:self.current,p1:self.point(x1,y1),p2:p,padding:[0.0;2]}); self.current=p; }
    fn curve_to(&mut self,_x1:f32,_y1:f32,_x2:f32,_y2:f32,x:f32,y:f32) { self.unsupported=true; self.current=self.point(x,y); }
    fn close(&mut self) { if self.current!=self.first { let p=self.first; self.curves.push(Curve{p0:self.current,p1:p,p2:p,padding:[0.0;2]}); self.current=p; } }
}
#[cfg(test)]
mod tests {
    use super::*;
    const FONT: &[u8] = include_bytes!("../../assets/fonts/ibm-plex/IBMPlexSans-Regular.ttf");
    #[test]
    fn bands_are_flat_sorted_and_skip_parallel_lines() {
        let face=ttf_parser::Face::parse(FONT,0).unwrap();
        let mut out=Outlines::default();
        let glyph=out.append(FONT,0,face.glyph_index('B').unwrap().0).unwrap().unwrap();
        assert_eq!(out.bands.len(),16);
        for (b,band) in out.bands.iter().enumerate() {
            let axis=if b<8 {1} else {0};
            let mut last=f32::INFINITY;
            for &i in &out.indices[band.start as usize..(band.start+band.count) as usize] {
                let c=&out.curves[i as usize];
                assert_ne!(c.min(axis),c.max(axis));
                assert!(c.max(1-axis)<=last);
                last=c.max(1-axis);
            }
        }
        assert_eq!(glyph.band_start,0);
        assert!(out.append(FONT,0,face.glyph_index(' ').unwrap().0).unwrap().is_none());
    }
    #[test]
    fn cubic_is_explicitly_marked() {
        let mut b=Builder::default();
        b.curve_to(0.0,0.0,1.0,1.0,2.0,2.0);
        assert!(b.unsupported);
        assert!(b.curves.is_empty());
    }
}
