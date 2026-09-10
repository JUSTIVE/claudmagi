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
            Shape::Text { text, scale, angle, center, color, .. } => {
                let mut d = String::new();
                for cmd in font::outline(text, *scale) {
                    let (cx, cy) = (center.x, center.y);
                    match cmd {
                        font::Cmd::Move(x, y) => write!(d, "M{:.2} {:.2}", cx + x, cy + y),
                        font::Cmd::Line(x, y) => write!(d, "L{:.2} {:.2}", cx + x, cy + y),
                        font::Cmd::Quad { cx: qx, cy: qy, x, y } => {
                            write!(d, "Q{:.2} {:.2} {:.2} {:.2}", cx + qx, cy + qy, cx + x, cy + y)
                        }
                        font::Cmd::Cubic { c1x, c1y, c2x, c2y, x, y } => write!(
                            d,
                            "C{:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
                            cx + c1x,
                            cy + c1y,
                            cx + c2x,
                            cy + c2y,
                            cx + x,
                            cy + y
                        ),
                        font::Cmd::Close => write!(d, "Z"),
                    }
                    .ok();
                }
                if !d.is_empty() {
                    let _ = writeln!(
                        s,
                        r#"<path d="{d}" fill="{}" fill-rule="nonzero" transform="rotate({angle:.2} {:.2} {:.2})"/>"#,
                        svg_color(*color),
                        center.x,
                        center.y
                    );
                }
            }
        }
    }
    s.push_str("</svg>\n");
    s
}
