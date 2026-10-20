//! Colour glyphs (OpenType COLR v0 and v1, colours from CPAL's first palette): a glyph's paint
//! graph flattened into filled layers, each with a solid colour or a gradient, in the same font
//! units (y down) as [`FontDb::outline`]. Composite modes other than source-over and nested clips
//! are approximated: each layer is clipped by its innermost clip only.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use kurbo::{Affine, BezPath, Point, Rect, Shape};
use skrifa::color::{Brush, ColorPainter, CompositeMode, Extend, Transform};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::Size;
use skrifa::raw::types::{BoundingBox, GlyphId};
use skrifa::{FontRef, MetadataProvider};

use crate::{FontDb, FontFace};

/// A colour a layer is painted with: fixed (RGBA), or the text's own fill colour at an opacity
/// (CPAL's palette index 0xFFFF).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayerColor {
    Rgba([u8; 4]),
    Foreground(f32),
}

/// How a gradient continues past its ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientExtend {
    Pad,
    Repeat,
    Reflect,
}

/// What fills a layer. Gradient geometry is in the layer's paint space: [`ColorLayer::paint_xf`]
/// maps it into the glyph's font units (y down).
#[derive(Clone, Debug, PartialEq)]
pub enum LayerPaint {
    Solid(LayerColor),
    Linear {
        p0: Point,
        p1: Point,
        stops: Vec<(f32, LayerColor)>,
        extend: GradientExtend,
    },
    Radial {
        c0: Point,
        r0: f64,
        c1: Point,
        r1: f64,
        stops: Vec<(f32, LayerColor)>,
        extend: GradientExtend,
    },
    /// Angles in degrees, clockwise.
    Sweep {
        center: Point,
        start: f32,
        end: f32,
        stops: Vec<(f32, LayerColor)>,
        extend: GradientExtend,
    },
}

/// One filled shape of a colour glyph.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorLayer {
    /// The shape, in font units with y down (as [`FontDb::outline`]).
    pub path: BezPath,
    pub paint: LayerPaint,
    /// Paint space → font units (y down), for gradients.
    pub paint_xf: Affine,
}

/// The most layers kept for one glyph (a damaged or hostile font may ask for many more).
const MAX_LAYERS: usize = 4_096;
/// The most glyphs whose layers are kept.
const CACHE_MAX: usize = 20_000;

type Cache = Mutex<HashMap<(u32, u32), Option<Arc<Vec<ColorLayer>>>>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

impl FontDb {
    /// The colour layers of glyph `gid` of the face with [`FontFace::id`] `font_id`, when the face
    /// draws it in colour (COLR); `None` for a plain glyph.
    pub fn color_layers(&self, font_id: u32, gid: u32) -> Option<Arc<Vec<ColorLayer>>> {
        if let Some(hit) = cache().lock().unwrap_or_else(|e| e.into_inner()).get(&(font_id, gid)) {
            return hit.clone();
        }
        let layers = self.face_by_id(font_id).and_then(|f| layers_of(&f, gid)).map(Arc::new);
        let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
        if c.len() >= CACHE_MAX {
            c.clear();
        }
        c.insert((font_id, gid), layers.clone());
        layers
    }
}

fn layers_of(face: &FontFace, gid: u32) -> Option<Vec<ColorLayer>> {
    let font = face.skrifa()?;
    let glyph = font.color_glyphs().get(GlyphId::new(gid))?;
    let palette: Vec<[u8; 4]> =
        font.color_palettes().get(0).map(|p| p.colors().iter().map(|c| [c.red, c.green, c.blue, c.alpha]).collect()).unwrap_or_default();
    let mut painter = Painter { font: &font, location: face.location(), palette, xf: vec![Affine::IDENTITY], clips: vec![], layers: vec![] };
    glyph.paint(face.location(), &mut painter).ok()?;
    (!painter.layers.is_empty()).then_some(painter.layers)
}

/// The fill a document gives colour layer `l` drawn through `xf` (font units → the space the
/// gradient geometry is wanted in), `fill` being the text's fill: its colour, the text's fill for the
/// text's colour, or a linear or radial gradient; and its opacity. Sweep gradients, which documents
/// don't have, become the mean of their stops' colours.
pub fn layer_fill(l: &ColorLayer, xf: Affine, fill: &vectorcraft_doc::color::Paint) -> (vectorcraft_doc::color::Paint, f32) {
    use vectorcraft_doc::color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
    let color = |c: &LayerColor| -> (Paint, f32) {
        match *c {
            LayerColor::Rgba([r, g, b, a]) => (Paint::solid(Color::rgb8(r, g, b)), f32::from(a) / 255.0),
            LayerColor::Foreground(a) => (fill.clone(), a),
        }
    };
    let gradient = |kind, stops: &[(f32, LayerColor)], geom| {
        let mut stops: Vec<GradientStop> = stops
            .iter()
            .map(|(o, c)| {
                let (p, a) = color(c);
                let mut s = GradientStop::new(if o.is_finite() { o.clamp(0.0, 1.0) } else { 0.0 }, p.color().unwrap_or(Color::BLACK));
                s.opacity = a;
                s
            })
            .collect();
        stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        Paint::Gradient(Box::new(GradientPaint { gradient: Gradient { kind, stops }, geom: Some(geom), angle: 0.0, swatch: None, freeform: None }))
    };
    let pxf = xf * l.paint_xf;
    let (paint, opacity) = match &l.paint {
        LayerPaint::Solid(c) => color(c),
        LayerPaint::Linear { p0, p1, stops, .. } => {
            (gradient(GradientKind::Linear, stops, GradientGeom { start: pxf * *p0, end: pxf * *p1, aspect: 1.0, focal: None }), 1.0)
        }
        LayerPaint::Radial { c0, r0, c1, r1, stops, .. } => {
            let start = pxf * *c1;
            let end = pxf * (*c1 + kurbo::Vec2::new(*r1, 0.0));
            let focal = (*r0 < 1e-9 && c0 != c1).then(|| pxf * *c0);
            (gradient(GradientKind::Radial, stops, GradientGeom { start, end, aspect: 1.0, focal }), 1.0)
        }
        LayerPaint::Sweep { stops, .. } => {
            let n = stops.len().max(1) as f32;
            let sum = stops.iter().fold([0.0f32; 4], |acc, (_, c)| {
                let (p, a) = color(c);
                let rgb = p.color().unwrap_or(Color::BLACK).to_rgba8(1.0);
                [acc[0] + f32::from(rgb[0]), acc[1] + f32::from(rgb[1]), acc[2] + f32::from(rgb[2]), acc[3] + a]
            });
            let mean = |v: f32| (v / n).round().clamp(0.0, 255.0) as u8;
            (Paint::solid(Color::rgb8(mean(sum[0]), mean(sum[1]), mean(sum[2]))), sum[3] / n)
        }
    };
    (paint, opacity.clamp(0.0, 1.0))
}

/// Font units y up → y down.
const FLIP: Affine = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]);

struct Painter<'a> {
    font: &'a FontRef<'a>,
    location: skrifa::instance::LocationRef<'a>,
    palette: Vec<[u8; 4]>,
    /// Current transform (font units, y up), innermost last.
    xf: Vec<Affine>,
    /// Clip shapes in font units y up, innermost last.
    clips: Vec<BezPath>,
    layers: Vec<ColorLayer>,
}

impl Painter<'_> {
    fn current(&self) -> Affine {
        self.xf.last().copied().unwrap_or(Affine::IDENTITY)
    }

    /// Glyph `gid`'s outline in font units, y up.
    fn outline(&self, gid: GlyphId) -> BezPath {
        let mut pen = Pen(BezPath::new());
        if let Some(g) = self.font.outline_glyphs().get(gid) {
            // A glyph that doesn't draw clips to nothing.
            let _ = g.draw(DrawSettings::unhinted(Size::unscaled(), self.location), &mut pen);
        }
        pen.0
    }

    fn color(&self, palette_index: u16, alpha: f32) -> LayerColor {
        let alpha = if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 1.0 };
        if palette_index == 0xFFFF {
            return LayerColor::Foreground(alpha);
        }
        let [r, g, b, a] = self.palette.get(usize::from(palette_index)).copied().unwrap_or([0, 0, 0, 255]);
        LayerColor::Rgba([r, g, b, (f32::from(a) * alpha).round() as u8])
    }

    fn stops(&self, stops: &[skrifa::color::ColorStop]) -> Vec<(f32, LayerColor)> {
        stops.iter().take(256).map(|s| (s.offset, self.color(s.palette_index, s.alpha))).collect()
    }

    /// Fill `region` (font units y up, already in place) with `brush`, whose geometry is in the
    /// space `paint` maps to font units y up.
    fn add(&mut self, mut region: BezPath, brush: Brush<'_>, paint: Affine) {
        if self.layers.len() >= MAX_LAYERS || region.elements().is_empty() {
            return;
        }
        let ext = |e: Extend| match e {
            Extend::Repeat => GradientExtend::Repeat,
            Extend::Reflect => GradientExtend::Reflect,
            _ => GradientExtend::Pad,
        };
        let pt = |p: skrifa::raw::types::Point<f32>| Point::new(f64::from(p.x), f64::from(p.y));
        let paint_kind = match brush {
            Brush::Solid { palette_index, alpha } => LayerPaint::Solid(self.color(palette_index, alpha)),
            Brush::LinearGradient { p0, p1, color_stops, extend } => {
                LayerPaint::Linear { p0: pt(p0), p1: pt(p1), stops: self.stops(color_stops), extend: ext(extend) }
            }
            Brush::RadialGradient { c0, r0, c1, r1, color_stops, extend } => LayerPaint::Radial {
                c0: pt(c0),
                r0: f64::from(r0).max(0.0),
                c1: pt(c1),
                r1: f64::from(r1).max(0.0),
                stops: self.stops(color_stops),
                extend: ext(extend),
            },
            Brush::SweepGradient { c0, start_angle, end_angle, color_stops, extend } => {
                LayerPaint::Sweep { center: pt(c0), start: start_angle, end: end_angle, stops: self.stops(color_stops), extend: ext(extend) }
            }
        };
        region.apply_affine(FLIP);
        self.layers.push(ColorLayer { path: region, paint: paint_kind, paint_xf: FLIP * paint });
    }
}

fn affine(t: Transform) -> Affine {
    Affine::new([t.xx, t.yx, t.xy, t.yy, t.dx, t.dy].map(f64::from))
}

impl ColorPainter for Painter<'_> {
    fn push_transform(&mut self, transform: Transform) {
        let next = self.current() * affine(transform);
        self.xf.push(next);
    }

    fn pop_transform(&mut self) {
        if self.xf.len() > 1 {
            self.xf.pop();
        }
    }

    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        let mut p = self.outline(glyph_id);
        p.apply_affine(self.current());
        self.clips.push(p);
    }

    fn push_clip_box(&mut self, b: BoundingBox<f32>) {
        let r = Rect::new(f64::from(b.x_min), f64::from(b.y_min), f64::from(b.x_max), f64::from(b.y_max));
        let mut p = r.to_path(0.1);
        p.apply_affine(self.current());
        self.clips.push(p);
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn fill(&mut self, brush: Brush<'_>) {
        // The innermost clip is the shape filled (a fill outside any clip has no shape).
        if let Some(region) = self.clips.last().cloned() {
            let paint = self.current();
            self.add(region, brush, paint);
        }
    }

    fn fill_glyph(&mut self, glyph_id: GlyphId, brush_transform: Option<Transform>, brush: Brush<'_>) {
        let mut region = self.outline(glyph_id);
        region.apply_affine(self.current());
        let paint = self.current() * brush_transform.map_or(Affine::IDENTITY, affine);
        self.add(region, brush, paint);
    }

    fn push_layer(&mut self, _: CompositeMode) {}
}

struct Pen(BezPath);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), f64::from(y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), f64::from(y)));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to((f64::from(cx0), f64::from(cy0)), (f64::from(x), f64::from(y)));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to((f64::from(cx0), f64::from(cy0)), (f64::from(cx1), f64::from(cy1)), (f64::from(x), f64::from(y)));
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fonts::{COLOR_FAMILY, COLOR_RED, color_font};

    #[test]
    fn a_colr_glyph_is_its_layers_in_palette_and_text_colours() {
        let db = FontDb::with_font_dirs(vec![]);
        assert!(db.add_font(color_font().expect("the test font builds")) > 0);
        let face = db.face(COLOR_FAMILY, "Regular").unwrap();
        assert_eq!(face.family, COLOR_FAMILY);
        let (big, small, plain) = (face.glyph_for('O'), face.glyph_for('o'), face.glyph_for('x'));
        let layers = db.color_layers(face.id(), big).expect("O is a colour glyph");
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].paint, LayerPaint::Solid(LayerColor::Rgba(COLOR_RED)));
        assert_eq!(layers[1].paint, LayerPaint::Solid(LayerColor::Foreground(1.0)));
        // The layers sit where the plain outlines do (font units, y down).
        assert_eq!(layers[0].path.bounding_box(), db.outline(&face, big).bounding_box());
        assert_eq!(layers[1].path.bounding_box(), db.outline(&face, small).bounding_box());
        assert!(db.color_layers(face.id(), plain).is_none());
        // Answers are kept: the same layers again.
        assert!(Arc::ptr_eq(&layers, &db.color_layers(face.id(), big).unwrap()));
    }

    #[test]
    fn a_plain_font_has_no_colour_glyphs() {
        let db = FontDb::with_font_dirs(vec![]);
        let face = db.face("Source Sans 3", "Regular").unwrap();
        assert!(db.color_layers(face.id(), face.glyph_for('O')).is_none());
        assert!(db.color_layers(u32::MAX, 1).is_none());
    }
}
