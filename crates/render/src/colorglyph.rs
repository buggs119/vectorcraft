//! Colour glyphs (COLR): each layer filled with its palette colour or gradient, layers in the
//! text's colour with the run's fill.

use std::sync::Arc;

use vectorcraft_geom::Affine;
use vectorcraft_text::color::{ColorLayer, GradientExtend, LayerColor, LayerPaint};
use vello_cpu::RenderContext;
use vello_cpu::peniko::{self, ColorStop};

use crate::Frame;

/// A colour glyph placed by the layout.
#[derive(Clone, Debug)]
pub(crate) struct ColorGlyph {
    /// The run whose fill its layers in the text's colour take.
    pub(crate) run: usize,
    /// Font units (y down) → text space.
    pub(crate) xf: Affine,
    pub(crate) layers: Arc<Vec<ColorLayer>>,
}

/// Draw `cg`'s layers through `xf` (font units → device), `fill` being its run's fill.
pub(crate) fn draw(ctx: &mut RenderContext, f: &Frame, xf: Affine, cg: &ColorGlyph, fill: &vectorcraft_color::Paint) {
    for layer in cg.layers.iter() {
        ctx.set_transform(xf);
        ctx.reset_paint_transform();
        if set_layer_paint(ctx, f, layer, fill) {
            ctx.set_fill_rule(peniko::Fill::NonZero);
            ctx.fill_path(&layer.path);
        }
    }
    ctx.reset_paint_transform();
}

/// The text's colour at `alpha`: its fill's colour when that is solid, black for a gradient or
/// pattern fill, nothing without a fill.
fn foreground(f: &Frame, fill: &vectorcraft_color::Paint, alpha: f32) -> Option<peniko::Color> {
    match fill {
        vectorcraft_color::Paint::None => None,
        vectorcraft_color::Paint::Solid { color, .. } => Some(f.ink.color(color, alpha)),
        _ => Some(f.ink.fixed([0, 0, 0, (alpha * 255.0).round() as u8])),
    }
}

fn color(f: &Frame, fill: &vectorcraft_color::Paint, c: LayerColor) -> peniko::Color {
    match c {
        LayerColor::Rgba(rgba) => f.ink.fixed(rgba),
        LayerColor::Foreground(a) => foreground(f, fill, a).unwrap_or(peniko::Color::TRANSPARENT),
    }
}

fn set_layer_paint(ctx: &mut RenderContext, f: &Frame, layer: &ColorLayer, fill: &vectorcraft_color::Paint) -> bool {
    let stops = |stops: &[(f32, LayerColor)]| -> Vec<ColorStop> {
        stops.iter().filter(|(o, _)| o.is_finite()).map(|(o, c)| ColorStop::from((*o, color(f, fill, *c)))).collect()
    };
    let extend = |e: GradientExtend| match e {
        GradientExtend::Pad => peniko::Extend::Pad,
        GradientExtend::Repeat => peniko::Extend::Repeat,
        GradientExtend::Reflect => peniko::Extend::Reflect,
    };
    let grad = match &layer.paint {
        LayerPaint::Solid(c) => {
            let c = color(f, fill, *c);
            if c.components[3] <= 0.0 {
                return false;
            }
            ctx.set_paint(c);
            return true;
        }
        LayerPaint::Linear { p0, p1, stops: s, extend: e } => {
            peniko::Gradient::new_linear(*p0, *p1).with_stops(stops(s).as_slice()).with_extend(extend(*e))
        }
        LayerPaint::Radial { c0, r0, c1, r1, stops: s, extend: e } => {
            peniko::Gradient::new_two_point_radial(*c0, *r0 as f32, *c1, *r1 as f32).with_stops(stops(s).as_slice()).with_extend(extend(*e))
        }
        LayerPaint::Sweep { center, start, end, stops: s, extend: e } => {
            peniko::Gradient::new_sweep(*center, start.to_radians(), end.to_radians()).with_stops(stops(s).as_slice()).with_extend(extend(*e))
        }
    };
    ctx.set_paint_transform(layer.paint_xf);
    ctx.set_paint(grad);
    true
}
