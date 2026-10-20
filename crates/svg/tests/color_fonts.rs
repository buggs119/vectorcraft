//! Colour fonts: outlined type writes a colour glyph as its layers, each with its own fill.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{CharStyle, Document, Node, NodeKind, TextObject};
use vectorcraft_geom::Point;
use vectorcraft_svg::{ExportOptions, export};
use vectorcraft_text::test_fonts::{COLOR_FAMILY, COLOR_RED, color_font};

#[test]
fn outlined_colour_glyphs_keep_their_colours() {
    assert!(vectorcraft_text::FontDb::global().add_font(color_font().unwrap()) > 0);
    let blue = Paint::solid(Color::rgb8(0, 0, 255));
    let st = CharStyle { size: 48.0, font_family: COLOR_FAMILY.into(), fill: blue, ..CharStyle::default() };
    let mut d = Document::new(300.0, 200.0);
    let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 100.0), "Ox", st))));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let svg = export(&d, &ExportOptions { outline_text: true, ..Default::default() }).to_lowercase();
    let [r, g, b, _] = COLOR_RED;
    let red = format!("#{r:02x}{g:02x}{b:02x}");
    assert!(svg.contains(&red), "the red layer: {svg}");
    // The layer in the text's colour and the plain x take the text's blue.
    assert!(svg.matches("#0000ff").count() >= 2, "{svg}");
}
