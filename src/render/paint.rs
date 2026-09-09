//! Draws a shape list with gpui's path API.

use gpui::{Bounds, Path, PathBuilder, PathStyle, Pixels, StrokeOptions, Window, fill, point, px, size};
use lyon::path::{LineCap, LineJoin};

use crate::font;
use crate::geom::Pt;
use crate::render::scene::Shape;
use crate::theme;

fn gp(p: Pt) -> gpui::Point<Pixels> {
    point(px(p.x), px(p.y))
}

fn stroke_path(pieces: &[Vec<Pt>], width: f32) -> Option<Path<Pixels>> {
    let options = StrokeOptions::default()
        .with_line_width(width)
        .with_line_join(LineJoin::Round)
        .with_line_cap(LineCap::Round);
    let mut b = PathBuilder::stroke(px(width)).with_style(PathStyle::Stroke(options));
    let mut any = false;
    for piece in pieces {
        if piece.len() < 2 {
            continue;
        }
        for (k, p) in piece.iter().enumerate() {
            if k == 0 {
                b.move_to(gp(*p));
            } else {
                b.line_to(gp(*p));
            }
        }
        any = true;
    }
    if !any {
        return None;
    }
    b.build().ok()
}

fn rounded_rect_outline(w: f32, h: f32, r: f32, mut b: PathBuilder) -> PathBuilder {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let r = r.min(hw).min(hh);
    b.move_to(point(px(-hw + r), px(-hh)));
    b.line_to(point(px(hw - r), px(-hh)));
    b.curve_to(point(px(hw), px(-hh + r)), point(px(hw), px(-hh)));
    b.line_to(point(px(hw), px(hh - r)));
    b.curve_to(point(px(hw - r), px(hh)), point(px(hw), px(hh)));
    b.line_to(point(px(-hw + r), px(hh)));
    b.curve_to(point(px(-hw), px(hh - r)), point(px(-hw), px(hh)));
    b.line_to(point(px(-hw), px(-hh + r)));
    b.curve_to(point(px(-hw + r), px(-hh)), point(px(-hw), px(-hh)));
    b.close();
    b
}

fn rounded_rect(center: Pt, w: f32, h: f32, r: f32, angle: f32, stroke: Option<f32>) -> Option<Path<Pixels>> {
    let builder = match stroke {
        Some(sw) => {
            let options = StrokeOptions::default().with_line_width(sw).with_line_join(LineJoin::Round);
            PathBuilder::stroke(px(sw)).with_style(PathStyle::Stroke(options))
        }
        None => PathBuilder::fill(),
    };
    let mut b = rounded_rect_outline(w, h, r, builder);
    if angle != 0.0 {
        b.rotate(angle);
    }
    b.translate(gp(center));
    b.build().ok()
}

pub fn paint_shapes(shapes: &[Shape], window: &mut Window) {
    for shape in shapes {
        match shape {
            Shape::Rect { x, y, w, h, color } => {
                let bounds = Bounds { origin: point(px(*x), px(*y)), size: size(px(*w), px(*h)) };
                window.paint_quad(fill(bounds, theme::hsla(*color)));
            }
            Shape::Stroke { pieces, width, color } => {
                if let Some(path) = stroke_path(pieces, *width) {
                    window.paint_path(path, theme::hsla(*color));
                }
            }
            Shape::RoundedRect { center, w, h, r, angle, color, stroke } => {
                if let Some(path) = rounded_rect(*center, *w, *h, *r, *angle, *stroke) {
                    window.paint_path(path, theme::hsla(*color));
                }
            }
            Shape::Text { text, scale, stroke, angle, center, color } => {
                if let Some(path) = font::build(text, *scale, *stroke, *angle, (center.x, center.y)) {
                    window.paint_path(path, theme::hsla(*color));
                }
            }
        }
    }
}
