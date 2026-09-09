//! Serialises a shape list as a standalone SVG document.

use std::fmt::Write as _;

use crate::font;
use crate::render::scene::Shape;
use crate::theme::Rgba;

fn svg_color(c: Rgba) -> String {
    format!(
        "rgba({},{},{},{:.3})",
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
        c.a
    )
}

pub fn to_svg(shapes: &[Shape], width: f32, height: f32) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">"#
    );
    for shape in shapes {
        match shape {
            Shape::Rect { x, y, w, h, color } => {
                let _ = writeln!(s, r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="{}"/>"#, svg_color(*color));
            }
            Shape::Stroke { pieces, width, color } => {
                for piece in pieces {
                    if piece.len() < 2 {
                        continue;
                    }
                    let pts: Vec<String> = piece.iter().map(|p| format!("{:.2},{:.2}", p.x, p.y)).collect();
                    let _ = writeln!(
                        s,
                        r#"<polyline points="{}" fill="none" stroke="{}" stroke-width="{width}" stroke-linecap="round" stroke-linejoin="round"/>"#,
                        pts.join(" "),
                        svg_color(*color)
                    );
                }
            }
            Shape::RoundedRect { center, w, h, r, angle, color, stroke } => {
                let (fill, stroke_attr) = match stroke {
                    Some(sw) => ("none".to_string(), format!(r#"stroke="{}" stroke-width="{sw}""#, svg_color(*color))),
                    None => (svg_color(*color), String::new()),
                };
                let _ = writeln!(
                    s,
                    r#"<rect x="{:.2}" y="{:.2}" width="{w:.2}" height="{h:.2}" rx="{r}" fill="{fill}" {stroke_attr} transform="rotate({angle:.2} {:.2} {:.2})"/>"#,
                    center.x - w / 2.0,
                    center.y - h / 2.0,
                    center.x,
                    center.y
                );
            }
            Shape::Text { text, scale, stroke, angle, center, color } => {
                let width = font::measure(text, *scale);
                let x0 = center.x - width / 2.0;
                let y0 = center.y - font::height(*scale) / 2.0;
                let _ = writeln!(
                    s,
                    r#"<g fill="none" stroke="{}" stroke-width="{stroke}" stroke-linecap="round" stroke-linejoin="round" transform="rotate({angle:.2} {:.2} {:.2})">"#,
                    svg_color(*color),
                    center.x,
                    center.y
                );
                for (i, ch) in text.chars().enumerate() {
                    let gx = x0 + i as f32 * font::ADVANCE * scale;
                    for stroke_pts in font::glyph(ch) {
                        let pts: Vec<String> = stroke_pts
                            .iter()
                            .map(|(ux, uy)| format!("{:.2},{:.2}", gx + ux * scale, y0 + uy * scale))
                            .collect();
                        if pts.len() == 1 {
                            let _ = writeln!(s, r#"<polyline points="{} {}"/>"#, pts[0], pts[0]);
                        } else {
                            let _ = writeln!(s, r#"<polyline points="{}"/>"#, pts.join(" "));
                        }
                    }
                }
                let _ = writeln!(s, "</g>");
            }
        }
    }
    s.push_str("</svg>\n");
    s
}
