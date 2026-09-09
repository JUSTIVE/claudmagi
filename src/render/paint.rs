//! Draws a shape list with gpui's path API. Tessellated paths are cached by
//! their geometry so a static board costs almost nothing per frame (#38);
//! colours are applied at paint time and never invalidate the cache.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use gpui::{Bounds, Path, PathBuilder, PathStyle, Pixels, StrokeOptions, Window, fill, point, px, size};
use lyon::path::{LineCap, LineJoin};

use crate::font;
use crate::geom::Pt;
use crate::render::scene::Shape;
use crate::theme;

/// Vertex budget for the cache (≈ 10 MB at 32 bytes a vertex). While the
/// window is resized or scrolled every frame mints new geometry, so entries
/// not used this frame are dropped once the budget is exceeded (#40).
const CACHE_BUDGET: usize = 320_000;
/// Rounded rects narrower than this (the packets) move every frame and are
/// cheap to build, so they bypass the cache.
const CACHE_MIN_W: f32 = 12.0;

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

/// Quantises a coordinate so tiny float noise still hits the cache.
fn q(v: f32) -> i32 {
    (v * 20.0).round() as i32
}

fn key_of(shape: &Shape) -> Option<u64> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match shape {
        Shape::Rect { .. } => return None,
        Shape::Stroke { pieces, width, .. } => {
            1u8.hash(&mut h);
            q(*width).hash(&mut h);
            for piece in pieces {
                piece.len().hash(&mut h);
                for p in piece {
                    q(p.x).hash(&mut h);
                    q(p.y).hash(&mut h);
                }
            }
        }
        Shape::RoundedRect { center, w, h: hh, r, angle, stroke, .. } => {
            if *w < CACHE_MIN_W {
                return None;
            }
            2u8.hash(&mut h);
            for v in [center.x, center.y, *w, *hh, *r, *angle] {
                q(v).hash(&mut h);
            }
            stroke.map(q).hash(&mut h);
        }
        Shape::Text { text, scale, stroke, angle, center, .. } => {
            3u8.hash(&mut h);
            text.hash(&mut h);
            for v in [*scale, *stroke, *angle, center.x, center.y] {
                q(v).hash(&mut h);
            }
        }
    }
    Some(h.finish())
}

fn build(shape: &Shape) -> Option<Path<Pixels>> {
    match shape {
        Shape::Rect { .. } => None,
        Shape::Stroke { pieces, width, .. } => stroke_path(pieces, *width),
        Shape::RoundedRect { center, w, h, r, angle, stroke, .. } => rounded_rect(*center, *w, *h, *r, *angle, *stroke),
        Shape::Text { text, scale, stroke, angle, center, .. } => {
            font::build(text, *scale, *stroke, *angle, (center.x, center.y))
        }
    }
}

/// Rough vertex count of a shape's tessellation, for the budget.
fn cost_of(shape: &Shape) -> usize {
    match shape {
        Shape::Rect { .. } => 0,
        Shape::Stroke { pieces, .. } => pieces.iter().map(|p| p.len()).sum::<usize>() * 14,
        Shape::RoundedRect { .. } => 48,
        Shape::Text { text, .. } => text.len() * 70,
    }
}

struct Entry {
    path: Path<Pixels>,
    cost: usize,
    last_used: u64,
}

#[derive(Default)]
pub struct PathCache {
    paths: HashMap<u64, Entry>,
    total: usize,
    frame: u64,
}

impl PathCache {
    /// Drops everything not used this frame once over budget.
    fn trim(&mut self) {
        if self.total <= CACHE_BUDGET {
            return;
        }
        let frame = self.frame;
        self.paths.retain(|_, e| e.last_used == frame);
        self.total = self.paths.values().map(|e| e.cost).sum();
        if self.total > CACHE_BUDGET {
            self.paths.clear();
            self.total = 0;
        }
    }

    pub fn paint(&mut self, shapes: &[Shape], window: &mut Window) {
        self.frame += 1;
        let frame = self.frame;
        for shape in shapes {
            let color = match shape {
                Shape::Rect { x, y, w, h, color } => {
                    let bounds = Bounds { origin: point(px(*x), px(*y)), size: size(px(*w), px(*h)) };
                    window.paint_quad(fill(bounds, theme::hsla(*color)));
                    continue;
                }
                Shape::Stroke { color, .. } | Shape::RoundedRect { color, .. } | Shape::Text { color, .. } => *color,
            };
            let path = match key_of(shape) {
                Some(key) => {
                    if !self.paths.contains_key(&key) {
                        match build(shape) {
                            Some(p) => {
                                let cost = cost_of(shape);
                                self.total += cost;
                                self.paths.insert(key, Entry { path: p, cost, last_used: frame });
                            }
                            None => continue,
                        }
                    }
                    self.paths.get_mut(&key).map(|e| {
                        e.last_used = frame;
                        e.path.clone()
                    })
                }
                None => build(shape),
            };
            if let Some(path) = path {
                window.paint_path(path, theme::hsla(color));
            }
        }
        self.trim();
    }
}

/// Uncached one-off painting (kept for callers without a cache).
#[allow(dead_code)]
pub fn paint_shapes(shapes: &[Shape], window: &mut Window) {
    PathCache::default().paint(shapes, window);
}
