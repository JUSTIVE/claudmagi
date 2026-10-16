//! The board as metaballs (#140).
//!
//! The same seating as the honeycomb — a cell per session, a flower per
//! window (#41, #86) — with the cells melted. A session is a ball of a field
//! and what gets drawn is where that field crosses a level: round on its own,
//! and run together with the balls of its own cluster, because the seats of
//! one cluster are a cell apart and the rim keeps every other cluster two
//! (#121). The honeycomb says "these belong together" with a doubled wall
//! (#128); here they are one body and there is nothing left to say.
//!
//! Inside the body each session keeps its own ground: a point belongs to
//! whichever ball pulls hardest on it, so the patches meet along a seam and
//! tile the body exactly. That is what lets a merged cluster still answer
//! "which of you is this?" in colour.

use gpui::Rgba;

use crate::geom::Pt;
use crate::model::BoardModel;
use crate::render::comb::{self, Cell, Comb, Extras};
use crate::render::scene::Shape;
use crate::theme::{self, Palette};

/// How far a ball's field reaches, against the cell's radius.
///
/// The seating sets this, not taste: cluster mates sit `sqrt(3) r` apart and
/// nothing else is ever nearer than `3 r` (#121). A reach of `1.9 r` carries
/// the first pair well over the level at their midpoint and cannot touch the
/// second at all — the falloff is exactly zero past the reach, which is also
/// what keeps one cluster's field somebody else's business.
const REACH: f32 = 1.9;

/// Where the surface is, against a ball's own weight.
const LEVEL: f32 = 0.5;

/// How finely the surface is sampled, against the cell's radius. About thirty
/// samples across a ball, which is enough that the polygon reads as a curve
/// at the size a session is drawn.
const STEP: f32 = 0.07;

/// How far a ball's ground is let past the line where its pull falls behind
/// the next ball's.
///
/// Two patches meeting share a seam exactly, but where the seam itself ends —
/// at the body's edge, and where three of them meet — each patch's outline is
/// cut by a different pair of fields, and they part by a fraction of a sample.
/// Nothing a lattice is sampled on can fix that, so the patches are grown a
/// hair instead: they overlap by less than a pixel rather than leaving the
/// board showing through at the one place the body should look most solid.
const BLEED: f32 = 0.03;

/// A ball of the field.
#[derive(Clone, Copy, Debug)]
pub struct Ball {
    pub at: Pt,
    pub reach: f32,
}

impl Ball {
    /// Wyvill's falloff: one at the centre, nought at the reach, and flat
    /// where it arrives, so two balls meet in a fillet rather than a crease.
    /// Compact support is the point of it — a ball past its reach costs
    /// nothing and, more to the point, says nothing.
    pub fn pull(&self, p: Pt) -> f32 {
        let d2 = ((p.x - self.at.x).powi(2) + (p.y - self.at.y).powi(2)) / (self.reach * self.reach);
        let t = (1.0 - d2).max(0.0);
        t * t * t
    }
}

/// How big a ball on its own comes out: `(1 - d²)³ = LEVEL`.
///
/// With the reach above this is `0.86 r`, which is the honeycomb cell's own
/// inradius — a session alone takes up what its cell used to.
pub fn lone(reach: f32) -> f32 {
    reach * (1.0 - LEVEL.cbrt()).max(0.0).sqrt()
}

/// The field of one body, sampled on a lattice.
///
/// Carries the whole field and, at every sample, the two strongest pulls and
/// who is pulling hardest. That is all the patches need: a ball holds a point
/// when its own pull is the top one, and the margin it holds it by is the
/// distance to the runner-up.
struct Sampled {
    at: Pt,
    step: f32,
    nx: usize,
    ny: usize,
    sum: Vec<f32>,
    top: Vec<f32>,
    second: Vec<f32>,
    who: Vec<u16>,
}

impl Sampled {
    fn of(balls: &[Ball], step: f32) -> Sampled {
        let (mut lo, mut hi) = (Pt::new(f32::MAX, f32::MAX), Pt::new(f32::MIN, f32::MIN));
        for b in balls {
            lo = Pt::new(lo.x.min(b.at.x - b.reach), lo.y.min(b.at.y - b.reach));
            hi = Pt::new(hi.x.max(b.at.x + b.reach), hi.y.max(b.at.y + b.reach));
        }
        // A sample's worth of margin all round: the field is nought out
        // there, so every loop closes inside the lattice instead of being
        // cut off at its edge.
        let at = Pt::new(lo.x - step, lo.y - step);
        let nx = ((hi.x - lo.x) / step).ceil() as usize + 3;
        let ny = ((hi.y - lo.y) / step).ceil() as usize + 3;
        let n = nx * ny;
        let mut s =
            Sampled { at, step, nx, ny, sum: vec![0.0; n], top: vec![0.0; n], second: vec![0.0; n], who: vec![u16::MAX; n] };
        // Scattered, not gathered: a ball only ever touches the lattice it
        // can reach, so a body of seven costs seven discs rather than seven
        // passes over the whole box.
        for (b, ball) in balls.iter().enumerate() {
            let (i0, i1) = s.span(ball.at.x - ball.reach, ball.at.x + ball.reach, s.at.x, s.nx);
            let (j0, j1) = s.span(ball.at.y - ball.reach, ball.at.y + ball.reach, s.at.y, s.ny);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let v = ball.pull(s.pt(i, j));
                    if v <= 0.0 {
                        continue;
                    }
                    let k = j * s.nx + i;
                    s.sum[k] += v;
                    if v > s.top[k] {
                        s.second[k] = s.top[k];
                        s.top[k] = v;
                        s.who[k] = b as u16;
                    } else if v > s.second[k] {
                        s.second[k] = v;
                    }
                }
            }
        }
        s
    }

    fn span(&self, lo: f32, hi: f32, from: f32, n: usize) -> (usize, usize) {
        let a = (((lo - from) / self.step).floor().max(0.0)) as usize;
        let b = (((hi - from) / self.step).ceil().max(0.0) as usize).min(n - 1);
        (a.min(n - 1), b)
    }

    fn pt(&self, i: usize, j: usize) -> Pt {
        Pt::new(self.at.x + i as f32 * self.step, self.at.y + j as f32 * self.step)
    }

    /// The corner of the lattice a ball can possibly reach, and how much of
    /// it that is.
    ///
    /// A patch never leaves its own ball's reach: out there the ball pulls
    /// at nothing, and whatever body is out there is somebody else's, who by
    /// then is pulling harder than the seam is grown by. Walking the whole
    /// cluster for each of its members instead costs the square of the
    /// cluster, and that was the frame.
    fn window(&self, ball: &Ball) -> (usize, usize, usize, usize) {
        let (i0, i1) = self.span(ball.at.x - ball.reach, ball.at.x + ball.reach, self.at.x, self.nx);
        let (j0, j1) = self.span(ball.at.y - ball.reach, ball.at.y + ball.reach, self.at.y, self.ny);
        // A sample's margin either side, so the loop closes inside the
        // window rather than against its edge.
        let (i0, j0) = (i0.saturating_sub(1), j0.saturating_sub(1));
        let (i1, j1) = ((i1 + 1).min(self.nx - 1), (j1 + 1).min(self.ny - 1));
        (i0, j0, i1 - i0 + 1, j1 - j0 + 1)
    }

    /// The body itself: positive where the field is over the level.
    fn body(&self, i: usize, j: usize) -> f32 {
        self.sum[j * self.nx + i] - LEVEL
    }

    /// Ball `b`'s own ground: inside the body, and nearer to `b` than to
    /// anybody else. Two fields met with a `min`, which is the one way to
    /// intersect two regions and keep a surface to march along.
    fn ground(&self, balls: &[Ball], b: usize, i: usize, j: usize) -> f32 {
        let k = j * self.nx + i;
        let mine = if self.who[k] == b as u16 { self.top[k] } else { balls[b].pull(self.pt(i, j)) };
        let theirs = if self.who[k] == b as u16 { self.second[k] } else { self.top[k] };
        self.body(i, j).min(mine - theirs + BLEED)
    }
}

/// The outline of `f >= 0` as closed loops: marching squares.
///
/// Crossings are worked out per lattice edge rather than per cell, so the two
/// cells that share an edge share the very point — the loops then close on
/// equality rather than on a tolerance, and a loop is a list of edges before
/// it is a list of points.
fn trace(v: &[f32], nx: usize, ny: usize, at: Pt, step: f32) -> Vec<Vec<Pt>> {
    if nx < 2 || ny < 2 || v.len() < nx * ny {
        return Vec::new();
    }

    // Where the line leaves each edge it enters, by the edge's place in a
    // count of the lattice's own edges: horizontal first, then vertical. One
    // entry per edge is all it takes — the two cells either side of a
    // crossing agree on which way the line runs, because the inside is on the
    // same hand in every case below — and walking it empties it, so nothing
    // else has to remember where it has been.
    let hn = (nx - 1) * ny;
    let mut next: Vec<u32> = vec![u32::MAX; hn + nx * (ny - 1)];
    for j in 0..ny - 1 {
        for i in 0..nx - 1 {
            let (a, b) = (v[j * nx + i], v[j * nx + i + 1]);
            let (c, d) = (v[(j + 1) * nx + i + 1], v[(j + 1) * nx + i]);
            let case = u8::from(a > 0.0) | u8::from(b > 0.0) << 1 | u8::from(c > 0.0) << 2 | u8::from(d > 0.0) << 3;
            if case == 0 || case == 15 {
                continue;
            }
            let middle = (a + b + c + d) / 4.0 > 0.0;
            // T, R, B, L of this cell, by the names given above.
            let edge = [j * (nx - 1) + i, hn + j * nx + i + 1, (j + 1) * (nx - 1) + i, hn + j * nx + i];
            const T: usize = 0;
            const R: usize = 1;
            const B: usize = 2;
            const L: usize = 3;
            let legs: &[(usize, usize)] = match case {
                1 => &[(L, T)],
                2 => &[(T, R)],
                3 => &[(L, R)],
                4 => &[(R, B)],
                5 if middle => &[(L, B), (R, T)],
                5 => &[(L, T), (R, B)],
                6 => &[(T, B)],
                7 => &[(L, B)],
                8 => &[(B, L)],
                9 => &[(B, T)],
                10 if middle => &[(T, L), (B, R)],
                10 => &[(T, R), (B, L)],
                11 => &[(B, R)],
                12 => &[(R, L)],
                13 => &[(R, T)],
                _ => &[(T, L)],
            };
            for (from, to) in legs {
                next[edge[*from]] = edge[*to] as u32;
            }
        }
    }

    // Where an edge is cut, worked out when the walk asks for it rather than
    // kept for every edge of the lattice: all but a thin ring of them are
    // never crossed, and the ones that are are asked for exactly once.
    let cut = |e: usize| -> Pt {
        let (i, j, step_x) = if e < hn { (e % (nx - 1), e / (nx - 1), true) } else { ((e - hn) % nx, (e - hn) / nx, false) };
        let a = v[j * nx + i];
        let b = if step_x { v[j * nx + i + 1] } else { v[(j + 1) * nx + i] };
        let t = a / (a - b);
        if step_x {
            Pt::new(at.x + (i as f32 + t) * step, at.y + j as f32 * step)
        } else {
            Pt::new(at.x + i as f32 * step, at.y + (j as f32 + t) * step)
        }
    };

    let mut out = Vec::new();
    for start in 0..next.len() {
        if next[start] == u32::MAX {
            continue;
        }
        let mut ring = Vec::new();
        let mut here = start;
        while next[here] != u32::MAX {
            ring.push(cut(here));
            here = std::mem::replace(&mut next[here], u32::MAX) as usize;
        }
        if ring.len() >= 3 {
            out.push(ring);
        }
    }
    out
}

/// A ring, as a stroke walks it: back round to where it started.
fn closed(mut ring: Vec<Pt>) -> Vec<Pt> {
    if let Some(first) = ring.first().copied() {
        ring.push(first);
    }
    ring
}

/// Every shape of the metaball view, in design units. `t` is seconds since
/// the view opened, which is what the bodies grow on.
pub fn build_shapes(model: &BoardModel, comb: &Comb, pal: Palette, scroll_y: f32, origin: Pt, t: f32) -> Vec<Shape> {
    let mut out = vec![Shape::Rect { x: 0.0, y: 0.0, w: 1.0e5, h: 1.0e5, color: pal.bg }];
    let r = comb.r;
    let step = (r * STEP).max(1.0);
    let place = |p: Pt| p - Pt::new(0.0, scroll_y) + origin;
    let on_board = |p: Pt| p.y > -r * 3.0 && p.y < 1.0e4;

    // The lattice the seats were taken from, kept as a field of dots. The
    // honeycomb's own grid is the furniture of that view and stands whether
    // anything is running or not (#102); here the furniture is what is left
    // of it when the walls go.
    for cell in &comb.cells {
        let at = place(cell.center);
        if cell.chip.is_some() || cell.subs.is_some() || !on_board(at) {
            continue;
        }
        let size = (r * 0.035).max(1.5);
        out.push(Shape::RoundedRect {
            center: at,
            w: size,
            h: size,
            r: size / 2.0,
            angle: 0.0,
            color: theme::with_alpha(pal.ink, 0.22 * comb::reveal(cell, t)),
            stroke: None,
        });
    }

    // Sessions of one cluster make one body; every other cluster is out of
    // reach of it, so each can be sampled on a lattice of its own.
    let mut clusters: Vec<Vec<(&Cell, usize)>> = Vec::new();
    let mut names: Vec<&str> = Vec::new();
    for cell in &comb.cells {
        let Some(i) = cell.chip else { continue };
        let Some(chip) = model.chips.get(i) else { continue };
        let group = chip.info.group.as_str();
        match names.iter().position(|n| *n == group) {
            Some(k) => clusters[k].push((cell, i)),
            None => {
                names.push(group);
                clusters.push(vec![(cell, i)]);
            }
        }
    }

    for cluster in &clusters {
        let grown: Vec<f32> = cluster.iter().map(|(c, _)| comb::reveal(c, t)).collect();
        // Growing the reach rather than the weight: a ball under the level
        // is not a small ball, it is no ball at all, so a weight that ramps
        // would have the body pop into existence halfway through its own
        // arrival (#92).
        let balls: Vec<Ball> = cluster
            .iter()
            .zip(&grown)
            .map(|((c, _), g)| Ball { at: place(c.center), reach: REACH * r * g })
            .collect();
        if balls.iter().all(|b| b.reach < 1.0) || !cluster.iter().any(|(c, _)| on_board(place(c.center))) {
            continue;
        }
        let field = Sampled::of(&balls, step);
        for (b, ((cell, chip), grown)) in cluster.iter().zip(&grown).enumerate() {
            let Some(c) = model.chips.get(*chip) else { continue };
            let info = &c.info;
            let fade = |col: Rgba| theme::with_alpha(col, *grown);
            let (skin, beat) = comb::body(info, pal, t);
            let (i0, j0, wx, wy) = field.window(&balls[b]);
            let corner = field.pt(i0, j0);
            // Sampled once: the floor below is the same ground with a line
            // drawn across it, and the pulls are the dear part.
            let ground: Vec<f32> =
                (0..wx * wy).map(|k| field.ground(&balls, b, i0 + k % wx, j0 + k / wx)).collect();
            for ring in trace(&ground, wx, wy, corner, field.step) {
                out.push(Shape::Poly { points: ring.clone(), color: fade(skin) });
                // The board's own colour along the seam, so two sessions in
                // one body are still two. Outside, where the body meets the
                // board, it is the board drawn on the board and nothing
                // shows; between patches it is the gap the honeycomb left
                // between its cells, down to a hairline.
                out.push(Shape::Stroke {
                    pieces: vec![closed(ring)],
                    width: comb::BORDER,
                    color: fade(pal.bg),
                });
            }

            let at = place(cell.center);
            // The rows the honeycomb draws, in the room a circle gives
            // instead of a cell: the body is never smaller than the ball
            // alone, so a line that fits the ball cannot leave the body.
            let reach = lone(REACH * r);
            for (row, slot) in comb::slots(info, pal, skin).iter().enumerate() {
                let Some(slot) = slot else { continue };
                let extras = Extras { origin: slot.origin.is_some(), check: slot.check, made: slot.made };
                let dy = comb::ROW_DY[row] * r;
                let tall = crate::font::height(if row == 1 { r * 0.021 } else { r * 0.016 }).max(r * 0.17);
                let half = (reach * reach - (dy.abs() + tall / 2.0).powi(2)).max(0.0).sqrt();
                let rb = comb::row_box_in(2.0 * half, r, row, &slot.label, extras);
                // The pull request's state as the floor of the body, which
                // is the same line the press is divided on (#139).
                if let Some(fill) = slot.fill {
                    let top = at.y + (comb::ROW_DY[1] + comb::ROW_DY[2]) / 2.0 * r;
                    let floor: Vec<f32> = ground
                        .iter()
                        .enumerate()
                        .map(|(k, g)| g.min(corner.y + (k / wx) as f32 * field.step - top))
                        .collect();
                    for ring in trace(&floor, wx, wy, corner, field.step) {
                        out.push(Shape::Poly { points: ring, color: fade(fill) });
                    }
                }
                let slot_beat = if row == 1 {
                    beat
                } else if slot.fill.is_some() {
                    1.0
                } else {
                    comb::row_beat(info, row, t)
                };
                let ink = fade(theme::with_alpha(slot.color, slot_beat));
                out.extend(comb::row_shapes(Pt::new(at.x, at.y + dy), &rb, slot, ink));
            }
        }
    }

    // Subagents: a colony of small balls in the cell next door, the way they
    // are a sub-grid of small hexes in the honeycomb (#88, #105). They run
    // together with each other and with nothing else — their reach is their
    // own size, and the session that spawned them is a whole cell away.
    for cell in &comb.cells {
        let Some(parent) = cell.subs else { continue };
        let at = place(cell.center);
        if !on_board(at) {
            continue;
        }
        let Some(chip) = model.chips.get(parent) else { continue };
        let agents: Vec<(String, bool)> =
            chip.subs.iter().filter(|s| !s.anim.gone).map(|s| (s.info.label(), s.info.running)).collect();
        if agents.is_empty() {
            continue;
        }
        let grown = comb::reveal(cell, t);
        let seats = comb::sub_grid(at, r, agents.len());
        let balls: Vec<Ball> =
            seats.iter().map(|(p, mr)| Ball { at: *p, reach: REACH * mr * grown }).collect();
        if balls.iter().all(|b| b.reach < 1.0) {
            continue;
        }
        let field = Sampled::of(&balls, (seats[0].1 * STEP).max(0.8));
        for (b, ((p, mr), (label, going))) in seats.iter().zip(&agents).enumerate() {
            // Solid, not an outline: a subagent is a thing doing work, and
            // what is around it is the empty part (#96). It wears the board's
            // chip colour, and a finished one is that colour given back to
            // the board (#110).
            let color = if *going { pal.chip } else { theme::lerp(pal.bg, pal.chip, 0.45) };
            let (i0, j0, wx, wy) = field.window(&balls[b]);
            let ground: Vec<f32> =
                (0..wx * wy).map(|k| field.ground(&balls, b, i0 + k % wx, j0 + k / wx)).collect();
            for ring in trace(&ground, wx, wy, field.pt(i0, j0), field.step) {
                out.push(Shape::Poly { points: ring.clone(), color: theme::with_alpha(color, grown) });
                out.push(Shape::Stroke {
                    pieces: vec![closed(ring)],
                    width: comb::BORDER,
                    color: theme::with_alpha(pal.bg, grown),
                });
            }
            let (text, scale) = comb::sub_box(lone(REACH * mr), label);
            if !text.is_empty() {
                out.push(Shape::Text {
                    text,
                    scale,
                    stroke: 1.0,
                    angle: 0.0,
                    center: *p,
                    color: theme::with_alpha(pal.bg, grown),
                });
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn circle(at: Pt, reach: f32) -> Vec<Vec<Pt>> {
        let balls = [Ball { at, reach }];
        let field = Sampled::of(&balls, reach * 0.04);
        let v: Vec<f32> = (0..field.nx * field.ny).map(|k| field.body(k % field.nx, k / field.nx)).collect();
        trace(&v, field.nx, field.ny, field.at, field.step)
    }

    /// Twice the shoelace: positive one way round, negative the other.
    fn area2(ring: &[Pt]) -> f32 {
        let mut a = 0.0;
        for k in 0..ring.len() {
            let (p, q) = (ring[k], ring[(k + 1) % ring.len()]);
            a += p.x * q.y - q.x * p.y;
        }
        a
    }

    /// A ball on its own is a circle of the size the level puts it at, and
    /// the loop closes: the walk has to come back to the edge it started on
    /// or the fill has nothing to fill (#140).
    #[test]
    fn one_ball_comes_out_a_closed_circle() {
        let at = Pt::new(300.0, 200.0);
        let rings = circle(at, 190.0);
        assert_eq!(rings.len(), 1, "one ball, one outline");
        let ring = &rings[0];
        let want = lone(190.0);
        for p in ring {
            let d = ((p.x - at.x).powi(2) + (p.y - at.y).powi(2)).sqrt();
            assert!((d - want).abs() < want * 0.01, "a point {d} off centre on a circle of {want}");
        }
        let area = area2(ring).abs() / 2.0;
        let round = std::f32::consts::PI * want * want;
        assert!((area - round).abs() < round * 0.02, "area {area} against a circle's {round}");
    }

    /// The whole point of the form (#140): the seating puts cluster mates a
    /// cell apart and everybody else two, and the reach is chosen so that
    /// the first pair is one body and the second is two.
    #[test]
    fn mates_run_together_and_strangers_do_not() {
        let r = 90.0;
        let one = |apart: f32| {
            let balls =
                [Ball { at: Pt::new(0.0, 0.0), reach: REACH * r }, Ball { at: Pt::new(apart, 0.0), reach: REACH * r }];
            let field = Sampled::of(&balls, r * STEP);
            let v: Vec<f32> = (0..field.nx * field.ny).map(|k| field.body(k % field.nx, k / field.nx)).collect();
        trace(&v, field.nx, field.ny, field.at, field.step).len()
        };
        // Neighbours in the honeycomb: a cell's height apart (#132).
        assert_eq!(one(r * 1.7320508), 1, "two sessions of one cluster are one body");
        // The nearest two cells of different clusters can ever be, the rim
        // between them being the thing that says they are different (#121).
        assert_eq!(one(r * 3.0), 2, "two clusters never touch");
    }

    /// A body is divided between the balls that made it, with no ground
    /// left over and none claimed twice: that is what lets a run-together
    /// cluster still answer in colour which session a spot belongs to.
    #[test]
    fn a_body_is_divided_between_the_balls_that_made_it() {
        let r = 90.0;
        let balls = [
            Ball { at: Pt::new(0.0, 0.0), reach: REACH * r },
            Ball { at: Pt::new(r * 1.7320508, 0.0), reach: REACH * r },
        ];
        let field = Sampled::of(&balls, r * STEP);
        let body: Vec<f32> = (0..field.nx * field.ny).map(|k| field.body(k % field.nx, k / field.nx)).collect();
        let whole: f32 = trace(&body, field.nx, field.ny, field.at, field.step)
            .iter()
            .map(|ring| area2(ring).abs() / 2.0)
            .sum();
        let parts: f32 = (0..balls.len())
            .map(|b| {
                let g: Vec<f32> =
                    (0..field.nx * field.ny).map(|k| field.ground(&balls, b, k % field.nx, k / field.nx)).collect();
                let rings = trace(&g, field.nx, field.ny, field.at, field.step);
                assert_eq!(rings.len(), 1, "a ball holds one piece of ground, not {}", rings.len());
                rings.iter().map(|ring| area2(ring).abs() / 2.0).sum::<f32>()
            })
            .sum();
        // Every bit of the body belongs to somebody, and the only ground
        // claimed twice is the hair the seams are grown by (`BLEED`).
        assert!(parts >= whole, "the parts come to {parts} of a body of {whole}: ground left over");
        assert!(parts - whole < whole * 0.03, "the parts come to {parts} of a body of {whole}: too much overlap");
    }

    /// What the view is laid out on: a ball alone takes up what its cell did,
    /// so three rows of text fit it the way they fit a cell (#94).
    #[test]
    fn a_ball_alone_is_the_size_of_the_cell_it_replaces() {
        for r in [46.0f32, 70.0, 104.0] {
            let got = lone(REACH * r);
            // The honeycomb cell's inradius, which is what its rows are cut
            // to fit.
            let inner = r * 1.7320508 / 2.0;
            assert!((got - inner).abs() < r * 0.02, "r={r}: a ball of {got} against a cell of {inner}");
        }
    }
}
