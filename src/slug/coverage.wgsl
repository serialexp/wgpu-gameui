// Slug analytic quadratic coverage, adapted from Eric Lengyel's MIT reference.
// Copyright (c) 2017 Eric Lengyel. See LICENSE-MIT in this directory.
struct Curve { p0: vec2f, p1: vec2f, p2: vec2f, padding: vec2f }
struct Band { start: u32, count: u32 }
struct Instance {
    bounds: vec4f,
    origin_size: vec4f, // device pen x, baseline y, pixels/em, padding
    color: vec4f,
    clip: vec4f,
    band_start: u32,
    padding0: u32, padding1: u32, padding2: u32,
}
struct View { size: vec2f, padding: vec2f }
@group(0) @binding(0) var<storage, read> curves: array<Curve>;
@group(0) @binding(1) var<storage, read> bands: array<Band>;
@group(0) @binding(2) var<storage, read> indices: array<u32>;
@group(0) @binding(3) var<storage, read> instances: array<Instance>;
@group(0) @binding(4) var<uniform> view: View;
struct Vertex {
    @builtin(position) position: vec4f,
    @location(0) em: vec2f,
    @location(1) @interpolate(flat) instance_id: u32,
}
@vertex fn vs(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> Vertex {
    let corners = array<vec2f,6>(vec2f(0,0),vec2f(1,0),vec2f(0,1),vec2f(0,1),vec2f(1,0),vec2f(1,1));
    let g=instances[i];
    // Axis-aligned orthographic equivalent of Slug's half-device-pixel dilation.
    let dilation=0.5/g.origin_size.z;
    let em=mix(g.bounds.xy-vec2f(dilation),g.bounds.zw+vec2f(dilation),corners[v]);
    let pixel=g.origin_size.xy+vec2f(em.x,-em.y)*g.origin_size.z;
    var out: Vertex;
    out.position=vec4f(pixel/view.size*vec2f(2,-2)+vec2f(-1,1),0,1);
    out.em=em;
    out.instance_id=i;
    return out;
}
fn root_code(p: vec3f) -> u32 {
    let signs=bitcast<vec3u>(p)>>vec3u(31u);
    let shift=signs.x | (signs.y<<1u) | (signs.z<<2u);
    return (0x2e74u>>shift)&0x101u;
}
// Relative curves are swizzled for vertical rays. Return crossing distances.
fn solve(p0: vec2f,p1: vec2f,p2: vec2f) -> vec2f {
    let a=p0-2.0*p1+p2;
    let b=p0-p1;
    var t: vec2f;
    if abs(a.y)<1.0/65536.0 {
        t=vec2f(p0.y*(0.5/b.y));
    } else {
        let d=sqrt(max(b.y*b.y-a.y*p0.y,0.0));
        t=(vec2f(b.y)+vec2f(-d,d))/a.y;
    }
    return (a.x*t-2.0*b.x)*t+p0.x;
}
fn ray(band: Band, sample: vec2f, pixels_per_em: f32, vertical: bool) -> vec2f {
    var cov=0.0;
    var weight=0.0;
    for(var j=0u;j<band.count;j++) {
        let c=curves[indices[band.start+j]];
        var p0=c.p0-sample; var p1=c.p1-sample; var p2=c.p2-sample;
        if vertical { p0=p0.yx; p1=p1.yx; p2=p2.yx; }
        // Preparation sorts by descending max x (or max y for vertical).
        if max(max(p0.x,p1.x),p2.x)*pixels_per_em < -0.5 { break; }
        let code=root_code(vec3f(p0.y,p1.y,p2.y));
        if code!=0u {
            let r=solve(p0,p1,p2)*pixels_per_em;
            if (code&1u)!=0u { cov+=clamp(r.x+0.5,0.0,1.0); weight=max(weight,clamp(1.0-abs(r.x)*2.0,0.0,1.0)); }
            if code>1u { cov-=clamp(r.y+0.5,0.0,1.0); weight=max(weight,clamp(1.0-abs(r.y)*2.0,0.0,1.0)); }
        }
    }
    return vec2f(cov,weight);
}
@fragment fn fs(v: Vertex) -> @location(0) vec4f {
    let g=instances[v.instance_id];
    let ppem=1.0/fwidth(v.em);
    let bi=vec2u(clamp((v.em-g.bounds.xy)/(g.bounds.zw-g.bounds.xy)*8.0,vec2f(0),vec2f(7)));
    let h=ray(bands[g.band_start+bi.y],v.em,ppem.x,false);
    let vert=ray(bands[g.band_start+8u+bi.x],v.em,ppem.y,true);
    let xcov=h.x; let ycov=-vert.x;
    let coverage=clamp(max(abs(xcov*h.y+ycov*vert.y)/max(h.y+vert.y,1.0/65536.0),min(abs(xcov),abs(ycov))),0.0,1.0);
    if v.position.x<g.clip.x || v.position.y<g.clip.y || v.position.x>=g.clip.z || v.position.y>=g.clip.w { discard; }
    return vec4f(g.color.rgb*g.color.a*coverage,g.color.a*coverage);
}
