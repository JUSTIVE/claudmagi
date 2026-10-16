//! Serialises a shape list as a standalone SVG document.

use std::fmt::Write as _;

use crate::font;
use crate::geom::Pt;
use crate::logos;
use crate::render::scene::{Screen, Shape};
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
    // Every pattern the shapes ask for, defined once and referred to by
    // name. The app has these in a fragment shader (#149); here they are
    // tiles, laid on the page rather than on each shape, which is the one
    // thing the two cannot agree on.
    let mut screens: Vec<(Screen, Rgba)> = Vec::new();
    for shape in shapes {
        if let Shape::Poly { color, screen, .. } = shape {
            if *screen != Screen::Solid && !screens.iter().any(|(sc, c)| sc == screen && *c == *color) {
                screens.push((*screen, *color));
            }
        }
    }
    if !screens.is_empty() {
        let _ = writeln!(s, "<defs>");
        for (k, (screen, color)) in screens.iter().enumerate() {
            let c = svg_color(*color);
            match screen {
                Screen::Solid => {}
                Screen::Slash { width, gap } => {
                    // The shader measures across the stripes, so the tile is
                    // the period taken at right angles to them.
                    let (w, period) = (width * 0.7071, (width + gap) * 0.7071);
                    let _ = writeln!(
                        s,
                        r#"<pattern id="screen{k}" patternUnits="userSpaceOnUse" width="{period:.3}" height="{period:.3}" patternTransform="rotate(45)"><rect x="0" y="0" width="{w:.3}" height="{period:.3}" fill="{c}"/></pattern>"#
                    );
                }
                Screen::Check { size } => {
                    let two = size * 2.0;
                    let _ = writeln!(
                        s,
                        r#"<pattern id="screen{k}" patternUnits="userSpaceOnUse" width="{two:.3}" height="{two:.3}"><rect x="0" y="0" width="{size:.3}" height="{size:.3}" fill="{c}"/><rect x="{size:.3}" y="{size:.3}" width="{size:.3}" height="{size:.3}" fill="{c}"/></pattern>"#
                    );
                }
            }
        }
        let _ = writeln!(s, "</defs>");
    }
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
            Shape::Poly { points, color, screen } => {
                let pts: Vec<String> = points.iter().map(|p| format!("{:.2},{:.2}", p.x, p.y)).collect();
                let paint = match screens.iter().position(|(sc, c)| sc == screen && *c == *color) {
                    Some(k) => format!("url(#screen{k})"),
                    None => svg_color(*color),
                };
                let _ = writeln!(s, r#"<polygon points="{}" fill="{paint}"/>"#, pts.join(" "));
            }
            Shape::Mark { mark, size, angle, center, color } => {
                let d = path_data(&logos::outline(*mark, *size), *center);
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
            Shape::Text { text, scale, angle, center, color, .. } => {
                let d = path_data(&font::outline(text, *scale), *center);
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

/// Outline commands as SVG path data, laid down around `center`. Labels and
/// marks are the same kind of outline by the time they reach here (#80).
fn path_data(cmds: &[font::Cmd], center: Pt) -> String {
    let (cx, cy) = (center.x, center.y);
    let mut d = String::new();
    for cmd in cmds {
        match *cmd {
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
    d
}
