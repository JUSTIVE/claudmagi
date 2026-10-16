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
use crate::render::scene::{Screen, Shape};
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
const STEP: f32 = 0.09;

/// How hard the surface pulls itself together where two balls meet (#141),
/// against the field they make between them.
///
/// Enough to groove the waist and cut a cusp where the seam comes out; not
/// so much that it cuts the body in two, which is the thing to watch — the
/// test for a cluster being one body is also the test for this number.
const TENSION: f32 = 0.28;

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

/// What the walk and the wander know a colony by (#145).
///
/// Its parent's name marked as the colony's own: the two are cells of one
/// body and push each other about like any other pair of mates, so they must
/// not be the same thing to the walk.
pub fn colony_key(session_id: &str) -> String {
    format!("{session_id}~subs")
}

/// The session a cell answers for, and the key it is walked under: a session
/// in its own cell, or the colony of subagents it has out next door (#88,
/// #145). Both are cells of the cluster and so both are balls of its body.
fn member_of<'a>(model: &'a BoardModel, cell: &Cell) -> Option<(&'a crate::model::ChipState, String)> {
    let (chip, colony) = match (cell.chip, cell.subs) {
        (Some(i), _) => (i, false),
        (None, Some(i)) => (i, true),
        _ => return None,
    };
    let chip = model.chips.get(chip)?;
    // A colony nobody is in is not a cell at all.
    if colony && !chip.subs.iter().any(|s| !s.anim.gone) {
        return None;
    }
    let id = chip.info.session_id.as_str();
    Some((chip, if colony { colony_key(id) } else { id.to_string() }))
}

/// One cell of a cluster, which is one ball of its body: a session, or the
/// colony of subagents that session has out next door (#88, #145).
struct Member<'a> {
    cell: &'a Cell,
    chip: &'a crate::model::ChipState,
    /// What the walk and the wander know it by (#142, #144).
    key: String,
    colony: bool,
}

/// How much of its own size a ball gives in the softest direction, and how
/// long the give takes to come round, in seconds (#147).
///
/// A ball on its own came out a *perfect* circle, which is the one shape a
/// drop of anything never holds: the field is the same in every direction,
/// so the level crossing is too. The reach is let out and taken in by
/// direction instead, a little on the second harmonic and half as much on
/// the fourth — an oval with the corners knocked off it rather than a
/// flower — and both turn slowly, so the surface swims instead of sitting
/// in one pose.
const SQUISH: f32 = 0.03;
const KNEAD: [f32; 2] = [11.0, 6.5];

/// A ball of the field.
#[derive(Clone, Copy, Debug)]
pub struct Ball {
    pub at: Pt,
    pub reach: f32,
    /// What takes it off being a circle (#147): the second and fourth
    /// harmonics of its reach, each as the pair of numbers that carries
    /// both how far it goes and which way it faces. All nought is a true
    /// circle, which is what the little ones are and what the shape is
    /// measured as.
    pub give: [(f32, f32); 2],
}

impl Ball {
    /// A ball with no give in it.
    pub fn round(at: Pt, reach: f32) -> Ball {
        Ball::with_give(at, reach, [(0.0, 0.0); 2])
    }

    pub fn with_give(at: Pt, reach: f32, give: [(f32, f32); 2]) -> Ball {
        Ball { at, reach, give }
    }

    /// A ball that gives, each one in its own pose and on its own beat
    /// (#147). The key is the cell's, the same one the walk and the wander
    /// know it by (#142, #144).
    pub fn soft(at: Pt, reach: f32, key: &str, t: f32) -> Ball {
        let turn = std::f32::consts::TAU;
        let seed = seed_of(key);
        let pose = |k: usize, size: f32| {
            let a = t / KNEAD[k] * turn + ((seed >> (24 * k as u32)) & 0xffff) as f32 / 65536.0 * turn;
            (size * a.cos(), size * a.sin())
        };
        Ball::with_give(at, reach, [pose(0, SQUISH), pose(1, SQUISH / 2.0)])
    }

    /// How far out the ball can possibly reach, which is what the lattice
    /// has to cover: the give adds to the reach in the direction it faces.
    fn farthest(&self) -> f32 {
        let far: f32 = self.give.iter().map(|(c, s)| (c * c + s * s).sqrt()).sum();
        self.reach * (1.0 + far)
    }

    /// Wyvill's falloff: one at the centre, nought at the reach, and flat
    /// where it arrives, so two balls meet in a fillet rather than a crease.
    /// Compact support is the point of it — a ball past its reach costs
    /// nothing and, more to the point, says nothing.
    ///
    /// The reach is the give's, not the ball's: it is longer where the ball
    /// is let out and shorter where it is taken in, and the surface follows
    /// it (#147). Both harmonics are even, so the direction can be had from
    /// the offset itself without taking its length — `cos 2θ` and `sin 2θ`
    /// are the offset squared over the distance squared, and the fourth
    /// follows from the second.
    pub fn pull(&self, p: Pt) -> f32 {
        let (dx, dy) = (p.x - self.at.x, p.y - self.at.y);
        let d2 = dx * dx + dy * dy;
        // Past anything the give could stretch to, which is most of the
        // window a ball is walked over and all of a neighbour's: answered
        // before the direction is worked out at all.
        // Past anything the give could stretch to — which is most of the
        // window a ball is walked over and all of a neighbour's — the
        // answer comes before the direction is worked out at all.
        let out = self.reach * (1.0 + SQUISH * 1.5);
        if d2 >= out * out {
            return 0.0;
        }
        if d2 <= f32::EPSILON {
            return 1.0;
        }
        let [(c2, s2), (c4, s4)] = self.give;
        let turn = 1.0 / d2;
        let (cos2, sin2) = ((dx * dx - dy * dy) * turn, 2.0 * dx * dy * turn);
        let (cos4, sin4) = (cos2 * cos2 - sin2 * sin2, 2.0 * sin2 * cos2);
        let reach = self.reach * (1.0 + c2 * cos2 + s2 * sin2 + c4 * cos4 + s4 * sin4);
        let t = (1.0 - d2 / (reach * reach)).max(0.0);
        t * t * t
    }
}

/// The number a key gives, which is what a phase is drawn from (#144, #147).
fn seed_of(key: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    h.finish()
}

/// How a body comes in (#143).
///
/// The wave walks a cell from nothing to arrived in a straight line (#92),
/// and a ball grown to that line arrives at a dead stop — the drawn surface
/// is a fixed fraction of the reach, so it swells at one rate throughout and
/// then simply ceases. Smoothstep gives both ends back their ease: it leaves
/// nothing gently and settles into its size rather than hitting it.
pub fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// How big a ball on its own comes out: `(1 - d²)³ = LEVEL`.
///
/// With the reach above this is `0.86 r`, which is the honeycomb cell's own
/// inradius — a session alone takes up what its cell used to.
pub fn lone(reach: f32) -> f32 {
    reach * (1.0 - LEVEL.cbrt()).max(0.0).sqrt()
}

/// How hard a ball is held to its seat, and how hard it is held back (#142).
///
/// The damping is the critical one for the stiffness, so a ball comes home
/// without ringing: what this is for is to make an arrival shoulder its
/// neighbours over and a departure let them back, not to set the board
/// wobbling.
const STIFF: f32 = 70.0;
const DAMP: f32 = 16.7;

/// How much of each other's room two balls of one cluster insist on, against
/// the size they are drawn at, and how hard they insist.
///
/// A shade over touching, so a cluster settles a little puffed out rather
/// than exactly on the lattice it was seated on — which is the whole of what
/// is wanted here. The seats are a honeycomb and a honeycomb is the thing
/// this view set out not to look like.
const SNUG: f32 = 1.03;
const PUSH: f32 = 90.0;

/// How far a ball may ever get from its seat, against the cell's radius.
///
/// The seat is what the presses, the drop-down and the hit test are keyed to
/// (#86), and the nearest cell of another cluster is only two away (#121). A
/// ball that wandered would take its body out of the cell that answers for
/// it and, worse, into somebody else's field.
const DRIFT: f32 = 0.06;

/// How slow counts as stopped, in design units a second.
const ASLEEP: f32 = 0.6;

/// How far a body wanders from where the walk left it, against the cell's
/// radius, and the two periods it wanders on, in seconds (#144).
///
/// Small on purpose: the whole of it is a couple of pixels, and what it is
/// for is not to move anything anywhere but to keep the surface from
/// reading as a drawing of what was running. Between two bodies it shows up
/// as a neck that thickens and thins by about a tenth either way, which is
/// the breath.
///
/// The two periods do not divide into each other, so a body never comes back
/// to the same place on a beat anyone could feel.
const WANDER: f32 = 0.03;
const BREATH: [f32; 2] = [7.3, 9.7];

/// The slow wander a body never stops making (#144).
///
/// The walk (#142) is a settling: it answers an arrival, a departure or a
/// re-seating and then stops, and a board where none of that is happening
/// stands perfectly still. This is the other thing, and it answers nothing
/// at all — a body of liquid never quite holds still, and a board of them
/// reads as a thing that is running rather than a picture of it.
///
/// It is drawn, not pushed. Through the spring it would be a force with a
/// clock in it, which is the one thing the walk cannot have: the walk sleeps
/// as soon as it is done (#143) and a wander in it would wake it every frame
/// to work out what is already a function of the time. Here it costs the two
/// sines it is, and the balls go on pushing each other about where they
/// really are.
///
/// The phase comes from the cell's own key, so no two of them breathe
/// together — a colony included (#145): it says whose it is by being part of
/// that body, not by moving with it.
pub fn wander(key: &str, t: f32, r: f32) -> Pt {
    let seed = seed_of(key);
    let turn = std::f32::consts::TAU;
    let phase = |bits: u32| ((seed >> bits) & 0xffff) as f32 / 65536.0 * turn;
    let swing = |k: usize| (t / BREATH[k] * turn + phase(16 * k as u32)).sin();
    Pt::new(swing(0), swing(1)) * (r * WANDER)
}

/// Where a session's ball actually is, which is not quite where it was
/// seated (#142).
#[derive(Clone, Copy, Debug)]
struct Bob {
    at: Pt,
    v: Pt,
    /// The seat it was walking around last time, so a seat that moves can be
    /// told from a ball that did.
    seat: Pt,
}

/// The balls, walked (#142).
///
/// The seating says where a session belongs; this says where it is. They are
/// not the same thing while the board is changing: a ball grows into a
/// cluster and shoulders its neighbours over, a ball that goes lets them back
/// in, and a cluster re-seated under a changed board (#138) slides across
/// instead of jumping. It is held on a spring to its seat and damped to
/// stillness, so the board is quiet again within half a second and the frame
/// clock can stop (#126).
#[derive(Default)]
pub struct Sim {
    bobs: std::collections::HashMap<String, Bob>,
    last: Option<std::time::Instant>,
    moving: bool,
    /// What the board looked like when the walk last had something to do.
    asleep: bool,
    board: u64,
}

impl Sim {
    /// One step, at whatever rate the board is painting.
    pub fn step(&mut self, model: &BoardModel, comb: &Comb, t: f32, now: std::time::Instant) {
        let dt = match self.last.replace(now) {
            // A long frame is walked as a short one rather than as itself:
            // a spring stepped with a big enough `dt` throws its ball across
            // the board.
            Some(then) => now.duration_since(then).as_secs_f32().min(1.0 / 20.0),
            None => 0.0,
        };
        self.advance(model, comb, t, dt);
    }

    /// Walked to a standstill, for a still frame that has no clock (#86).
    pub fn settle(&mut self, model: &BoardModel, comb: &Comb, t: f32) {
        self.advance(model, comb, t, 0.0);
        for _ in 0..600 {
            self.advance(model, comb, t, 1.0 / 60.0);
            if !self.moving {
                break;
            }
        }
    }

    pub fn moving(&self) -> bool {
        self.moving
    }

    /// Where this session's body is just now: where the walk left it, and
    /// how far it has wandered off since (#142, #144).
    pub fn at(&self, id: &str, seat: Pt, t: f32, r: f32) -> Pt {
        self.walked(id, seat) + wander(id, t, r)
    }

    /// Where the walk left it, or its seat if the walk has not met it. This
    /// is what the walk itself works on: the wander is drawn on top of the
    /// balls rather than pushed through them, so it never becomes a force
    /// the others have to answer.
    fn walked(&self, id: &str, seat: Pt) -> Pt {
        self.bobs.get(id).map_or(seat, |b| b.at)
    }

    fn advance(&mut self, model: &BoardModel, comb: &Comb, t: f32, dt: f32) {
        let r = comb.r;
        // Who is on the board, where they are seated, and how big they are
        // drawn just now: a ball grows into its size (#92), and what it
        // takes up while it grows is what shoulders its neighbours.
        let mut seats: Vec<(String, Pt, f32, &str)> = Vec::new();
        for cell in &comb.cells {
            let Some((chip, key)) = member_of(model, cell) else { continue };
            let size = lone(REACH * r * ease(comb::reveal(cell, t)));
            seats.push((key, cell.center, size, chip.info.group.as_str()));
        }
        // Who is here, where they sit and how big they are, to the pixel the
        // drawing is cached on: anything else and the walk goes back to work.
        let print = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for (id, seat, size, _) in &seats {
                id.hash(&mut h);
                for v in [seat.x, seat.y, *size] {
                    ((v * 20.0).round() as i32).hash(&mut h);
                }
            }
            h.finish()
        };
        if print != self.board {
            self.board = print;
            self.asleep = false;
        }
        self.bobs.retain(|id, _| seats.iter().any(|(s, ..)| s == id));
        for (id, seat, ..) in &seats {
            let bob = self
                .bobs
                .entry(id.clone())
                .or_insert(Bob { at: *seat, v: Pt::new(0.0, 0.0), seat: *seat });
            // The packing can move a seat out from under a body (#138). The
            // body goes with it, keeping whatever lean it had, rather than
            // setting off across the board to catch up: the rim between two
            // clusters is one cell wide (#121), and a body crossing it would
            // run into somebody else's on the way.
            if bob.seat != *seat {
                bob.at = bob.at + (*seat - bob.seat);
                bob.seat = *seat;
            }
        }
        // A walk that has come to rest is not walked at all. A ball creeping
        // by a thousandth of a pixel is a ball whose body is tessellated
        // again every frame, and that is paid for at the refresh rate (#38,
        // #126): what wakes it is the board changing under it, not time
        // passing.
        if dt <= 0.0 || self.asleep {
            self.moving = false;
            return;
        }

        let mut fastest = 0.0f32;
        let now: Vec<Pt> = seats.iter().map(|(id, seat, ..)| self.walked(id, *seat)).collect();
        for (k, (id, seat, size, group)) in seats.iter().enumerate() {
            let here = now[k];
            let mut f = (*seat - here) * STIFF;
            // Only the balls of one body push each other: two clusters are a
            // rim apart and never touch, and a push between them would be a
            // force with nothing drawn to explain it.
            for (j, (_, _, other_size, other_group)) in seats.iter().enumerate() {
                if j == k || other_group != group {
                    continue;
                }
                let away = here - now[j];
                let len = away.len();
                let room = (size + other_size) * SNUG;
                if len > 0.001 && len < room {
                    f = f + away * ((room - len) * PUSH / len);
                }
            }
            let Some(bob) = self.bobs.get_mut(id) else { continue };
            f = f - bob.v * DAMP;
            bob.v = bob.v + f * dt;
            bob.at = bob.at + bob.v * dt;
            // Never further out than the seat can answer for.
            let off = bob.at - *seat;
            let far = off.len();
            if far > r * DRIFT {
                bob.at = *seat + off * (r * DRIFT / far);
                bob.v = bob.v * 0.5;
            }
            fastest = fastest.max(bob.v.len());
        }
        self.asleep = fastest <= ASLEEP;
        self.moving = !self.asleep;
    }
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
            let far = b.farthest();
            lo = Pt::new(lo.x.min(b.at.x - far), lo.y.min(b.at.y - far));
            hi = Pt::new(hi.x.max(b.at.x + far), hi.y.max(b.at.y + far));
        }
        // A sample's worth of margin all round: the field is nought out
        // there, so every loop closes inside the lattice instead of being
        // cut off at its edge.
        // Snapped to the step, so every body is sampled on the *same*
        // lattice rather than on one of its own: the screen the orange
        // board prints with is laid on these points, and a screen that
        // does not line up across a seam reads as two sheets (#146).
        let snap = |v: f32| (v / step).floor() * step;
        let at = Pt::new(snap(lo.x - step), snap(lo.y - step));
        let nx = ((hi.x - lo.x) / step).ceil() as usize + 3;
        let ny = ((hi.y - lo.y) / step).ceil() as usize + 3;
        let n = nx * ny;
        let mut s =
            Sampled { at, step, nx, ny, sum: vec![0.0; n], top: vec![0.0; n], second: vec![0.0; n], who: vec![u16::MAX; n] };
        // Scattered, not gathered: a ball only ever touches the lattice it
        // can reach, so a body of seven costs seven discs rather than seven
        // passes over the whole box.
        for (b, ball) in balls.iter().enumerate() {
            let far = ball.farthest();
            let (i0, i1) = s.span(ball.at.x - far, ball.at.x + far, s.at.x, s.nx);
            let (j0, j1) = s.span(ball.at.y - far, ball.at.y + far, s.at.y, s.ny);
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
        let far = ball.farthest();
        let (i0, i1) = self.span(ball.at.x - far, ball.at.x + far, self.at.x, self.nx);
        let (j0, j1) = self.span(ball.at.y - far, ball.at.y + far, self.at.y, self.ny);
        // A sample's margin either side, so the loop closes inside the
        // window rather than against its edge.
        let (i0, j0) = (i0.saturating_sub(1), j0.saturating_sub(1));
        let (i1, j1) = ((i1 + 1).min(self.nx - 1), (j1 + 1).min(self.ny - 1));
        (i0, j0, i1 - i0 + 1, j1 - j0 + 1)
    }

    /// The body itself: positive where the field is over the level.
    fn body(&self, i: usize, j: usize) -> f32 {
        let k = j * self.nx + i;
        self.sum[k] - self.tension(k) - LEVEL
    }

    /// What the surface takes back where two balls meet (#141).
    ///
    /// Adding two fields together pushes the surface *out* where they
    /// overlap, which is the one place a liquid would pull it in: a film
    /// between two bubbles stands at the waist and drags the outside of both
    /// into a groove, and where the film reaches the air the three surfaces
    /// meet in a cusp. So the field gives some back exactly where the top two
    /// pulls are alike — all of it where they are equal, none where one ball
    /// has the place to itself.
    ///
    /// Only the surface moves. A sample deep inside a body is far above the
    /// level and a nudge does not bring it down to it, so this shows up at
    /// the waist and at the cusp and nowhere else, which is where surface
    /// tension shows up too.
    fn tension(&self, k: usize) -> f32 {
        let (a, b) = (self.top[k], self.second[k]);
        if b <= 0.0 {
            return 0.0;
        }
        let share = (a - b) / (a + b);
        let alike = (1.0 - share * share).max(0.0);
        TENSION * alike * alike * (a + b)
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
pub fn build_shapes(
    model: &BoardModel,
    comb: &Comb,
    pal: Palette,
    scroll_y: f32,
    origin: Pt,
    t: f32,
    sim: &Sim,
) -> Vec<Shape> {
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
            color: theme::with_alpha(pal.ink, 0.22 * ease(comb::reveal(cell, t))),
            stroke: None,
        });
    }

    // The cells of one cluster make one body; every other cluster is out of
    // reach of it, so each can be sampled on a lattice of its own.
    //
    // A colony of subagents is one of those cells (#88, #145). It is a ball
    // like any other and runs together with the rest, which is the whole of
    // what says whose agents they are: out on its own, a colony sitting
    // between two bodies belonged to whichever one the eye picked.
    let mut clusters: Vec<Vec<Member>> = Vec::new();
    let mut names: Vec<&str> = Vec::new();
    for cell in &comb.cells {
        let Some((chip, key)) = member_of(model, cell) else { continue };
        let group = chip.info.group.as_str();
        let seat = Member { cell, chip, key, colony: cell.chip.is_none() };
        match names.iter().position(|n| *n == group) {
            Some(k) => clusters[k].push(seat),
            None => {
                names.push(group);
                clusters.push(vec![seat]);
            }
        }
    }

    for cluster in &clusters {
        let grown: Vec<f32> = cluster.iter().map(|m| ease(comb::reveal(m.cell, t))).collect();
        // Growing the reach rather than the weight: a ball under the level
        // is not a small ball, it is no ball at all, so a weight that ramps
        // would have the body pop into existence halfway through its own
        // arrival (#92).
        // Where the walk has put each of them, which is near its seat and not
        // on it (#142).
        let balls: Vec<Ball> = cluster
            .iter()
            .zip(&grown)
            .map(|(m, g)| Ball::soft(place(sim.at(&m.key, m.cell.center, t, r)), REACH * r * g, &m.key, t))
            .collect();
        if balls.iter().all(|b| b.reach < 1.0) || !balls.iter().any(|b| on_board(b.at)) {
            continue;
        }
        let field = Sampled::of(&balls, step);
        for (b, (seat, grown)) in cluster.iter().zip(&grown).enumerate() {
            let info = &seat.chip.info;
            let fade = |col: Rgba| theme::with_alpha(col, *grown);
            // The letters wait for a body big enough to hold them (#143).
            // They are cut to the size the ball ends up (#94), so a row drawn
            // at full width on a ball half grown hangs out of it on both
            // sides.
            let letters = ease(((grown - 0.6) / 0.4).clamp(0.0, 1.0));
            let (skin, beat) = comb::body(info, pal, t);
            // On a board that prints there is one ink, and what the cell
            // would have said in colour it says in the weave instead (#146,
            // #149). The beat rides in the alpha, where the colour was
            // carrying it.
            let screen = if pal.screened { comb::body_screen(info) } else { Screen::Solid };
            let fill = if pal.screened { theme::with_alpha(pal.ink, beat) } else { skin };
            // What the body reads as — the skin itself where it is painted,
            // and the weave's own weight where it is printed. Everything
            // that has to hold its own against the body is measured against
            // this and not against the fill.
            let on = if pal.screened { theme::lerp(pal.bg, pal.ink, screen.weight()) } else { skin };
            // On a board that prints, the letters do not stand on the body
            // at all: a screen is half sheet, and letters over it are a
            // mess at any tone but solid. They stand on a plate of the bare
            // sheet instead, the way a label is knocked out of a print, so
            // what they have to read on is the paper (#146).
            let rows = comb::slots(info, pal, if pal.screened { pal.bg } else { on });
            let (i0, j0, wx, wy) = field.window(&balls[b]);
            let corner = field.pt(i0, j0);
            // Sampled once: the floor below is the same ground with a line
            // drawn across it, and the pulls are the dear part.
            let ground: Vec<f32> =
                (0..wx * wy).map(|k| field.ground(&balls, b, i0 + k % wx, j0 + k / wx)).collect();
            let at = balls[b].at;
            for ring in trace(&ground, wx, wy, corner, field.step) {
                out.push(Shape::Poly { points: ring.clone(), color: fade(fill), screen });
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

            if seat.colony {
                // The agents, in the room of the body they belong to. Inside
                // a body the board's part is played by the skin: an agent is
                // drawn in the board's own colour, which is what every letter
                // on a body is drawn in (#139), and a finished one gives that
                // colour back to the skin exactly as it gave it back to the
                // board when the colony stood out there on its own (#110).
                let agents: Vec<(String, bool)> = seat
                    .chip
                    .subs
                    .iter()
                    .filter(|s| !s.anim.gone)
                    .map(|s| (s.info.label(), s.info.running))
                    .collect();
                if agents.is_empty() {
                    continue;
                }
                let holes = comb::sub_grid(at, r, agents.len());
                let dots: Vec<Ball> =
                    holes.iter().map(|(p, mr)| Ball::round(*p, REACH * mr * grown)).collect();
                if dots.iter().all(|b| b.reach < 1.0) {
                    continue;
                }
                let small = Sampled::of(&dots, (holes[0].1 * STEP).max(0.8));
                let ink = comb::legible(pal.bg, on, pal);
                for (k, ((p, mr), (label, going))) in holes.iter().zip(&agents).enumerate() {
                    let color = if *going { ink } else { theme::lerp(on, ink, 0.45) };
                    let (i0, j0, wx, wy) = small.window(&dots[k]);
                    let ground: Vec<f32> =
                        (0..wx * wy).map(|n| small.ground(&dots, k, i0 + n % wx, j0 + n / wx)).collect();
                    for ring in trace(&ground, wx, wy, small.pt(i0, j0), small.step) {
                        out.push(Shape::poly(ring.clone(), fade(color)));
                        // And the hairline between two of them is the skin,
                        // for the same reason it is the board outside (#140).
                        out.push(Shape::Stroke {
                            pieces: vec![closed(ring)],
                            width: comb::BORDER,
                            color: fade(on),
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
                            color: theme::with_alpha(comb::legible(on, color, pal), letters),
                        });
                    }
                }
                continue;
            }
            // The rows the honeycomb draws, in the room a circle gives
            // instead of a cell: the body is never smaller than the ball
            // alone, so a line that fits the ball cannot leave the body.
            // Measured at the ball's tightest, since the surface gives by
            // direction now and a line cut to the slack side hangs out of
            // the taut one (#147).
            let reach = lone(REACH * r) * (1.0 - SQUISH * 1.5);
            for (row, slot) in rows.iter().enumerate() {
                let Some(slot) = slot else { continue };
                let extras = Extras { origin: slot.origin.is_some(), check: slot.check, made: slot.made };
                let dy = comb::ROW_DY[row] * r;
                let tall = crate::font::height(if row == 1 { r * 0.021 } else { r * 0.016 }).max(r * 0.17);
                let half = (reach * reach - (dy.abs() + tall / 2.0).powi(2)).max(0.0).sqrt();
                let rb = comb::row_box_in(2.0 * half, r, row, &slot.label, extras);
                // The pull request's state as the floor of the body, which
                // is the same line the press is divided on (#139).
                if let Some(bed) = slot.fill {
                    let top = at.y + (comb::ROW_DY[1] + comb::ROW_DY[2]) / 2.0 * r;
                    let floor: Vec<f32> = ground
                        .iter()
                        .enumerate()
                        .map(|(k, g)| g.min(corner.y + (k / wx) as f32 * field.step - top))
                        .collect();
                    let (bed, weave) = if pal.screened { (pal.ink, slot.screen) } else { (bed, Screen::Solid) };
                    for ring in trace(&floor, wx, wy, corner, field.step) {
                        // A screened floor is laid on bare sheet, not on the
                        // body: a weave with air in it would show the cell's
                        // own ink through its gaps and come out solid (#149).
                        if pal.screened {
                            out.push(Shape::poly(ring.clone(), fade(pal.bg)));
                        }
                        out.push(Shape::Poly { points: ring, color: fade(bed), screen: weave });
                    }
                }
                let slot_beat = if row == 1 {
                    beat
                } else if slot.fill.is_some() {
                    1.0
                } else {
                    comb::row_beat(info, row, t)
                };
                // Rows that say their state by being painted write in the
                // board's own colour (#139), which on a plate of that same
                // colour is nothing at all: on the printing board they take
                // the ink instead (#146).
                let paint =
                    if pal.screened { comb::legible(slot.color, pal.bg, pal) } else { slot.color };
                let ink = theme::with_alpha(paint, slot_beat * letters);
                let line = Pt::new(at.x, at.y + dy);
                if pal.screened {
                    let w = comb::row_width(&rb) + tall * 0.5;
                    out.push(Shape::RoundedRect {
                        center: line,
                        w,
                        h: tall,
                        r: tall / 2.0,
                        angle: 0.0,
                        color: theme::with_alpha(pal.bg, grown * letters),
                        stroke: None,
                    });
                }
                out.extend(comb::row_shapes(line, &rb, slot, ink));
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn circle(at: Pt, reach: f32) -> Vec<Vec<Pt>> {
        let balls = [Ball::round(at, reach)];
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

    /// The orange board prints rather than paints (#146): one sheet, one
    /// ink, and a tone is a screen of squares whose area is the ink the
    /// colour asked for. Nothing on it is filled with a colour of its own.
    #[test]
    fn the_printing_board_lays_a_screen_instead_of_a_colour() {
        use crate::settings::BoardTheme;
        let mut model = board(&[("a", 2)]);
        // One of them carrying a pull request, since the floor is where the
        // two alphabets have the most to say (#139, #149).
        model.chips[0].info.prs = vec![crate::pr::Pr::synthetic(7, crate::pr::Look::Merged)];
        let comb = comb::lay_out(&model, 980.0, 620.0);
        let mut sim = Sim::default();
        sim.settle(&model, &comb, 1.0e3);
        let drawn = |theme| {
            let pal = Palette::for_theme(theme);
            (pal, build_shapes(&model, &comb, pal, 0.0, Pt::new(0.0, 0.0), 1.0e3, &sim))
        };
        let (white, painted) = drawn(BoardTheme::White);
        let (orange, printed) = drawn(BoardTheme::Orange);
        assert!(!white.screened && orange.screened, "the boards do not disagree about printing");

        // Every body the painted board fills with a colour of its own, the
        // printing board fills with the one ink and a weave.
        let bodies = |shapes: &[Shape]| -> Vec<(Rgba, Screen)> {
            shapes
                .iter()
                .filter_map(|s| match s {
                    Shape::Poly { points, color, screen } if points.len() > 8 => Some((*color, *screen)),
                    _ => None,
                })
                .collect()
        };
        assert!(!bodies(&painted).is_empty(), "the painted board did not fill its bodies");
        assert!(bodies(&painted).iter().all(|(_, sc)| *sc == Screen::Solid), "the painted board screened a body");
        let printed_bodies = bodies(&printed);
        assert!(!printed_bodies.is_empty(), "the printing board did not fill its bodies");
        // Its ink, woven, or its bare sheet to knock a floor out with. There
        // is nothing else on this board to put down.
        for (color, screen) in &printed_bodies {
            let rgb = (color.r, color.g, color.b);
            let ink = rgb == (orange.ink.r, orange.ink.g, orange.ink.b);
            let sheet = rgb == (orange.bg.r, orange.bg.g, orange.bg.b) && *screen == Screen::Solid;
            assert!(ink || sheet, "the printing board put down {rgb:?}, which is neither its ink nor its sheet");
        }

        // And it is the weave that tells the states apart, not the tone: a
        // session at work, one waiting on a person and one sitting idle are
        // three different ones.
        use crate::model::{Phase, SessionInfo};
        let weave = |phase| comb::body_screen(&SessionInfo::synthetic(1, "S", phase));
        let (work, idle, waiting) = (weave(Phase::Working), weave(Phase::Idle), weave(Phase::NeedsUser));
        assert_ne!(work, idle);
        assert_ne!(work, waiting);
        assert_ne!(idle, waiting);
        assert_eq!(work, Screen::Solid, "a session at work is the solid one");
        assert!((idle.weight() - 0.5).abs() < 0.01, "half a checkerboard is half the ink");

        // A floor is laid on bare sheet and then woven, or its gaps show
        // the cell's own ink and it comes out solid (#149).
        let under = printed.iter().filter_map(|s| match s {
            Shape::Poly { points, color, screen } if points.len() > 8 => Some((*color, *screen)),
            _ => None,
        });
        assert!(
            under.clone().any(|(c, sc)| sc == Screen::Solid && c.r == orange.bg.r && c.g == orange.bg.g),
            "a woven floor was laid straight onto the body"
        );
        assert!(
            under.clone().any(|(c, sc)| sc != Screen::Solid && c.r == orange.ink.r),
            "the floor was laid down as flat ink rather than woven"
        );

        // And the letters stand on a plate of the bare sheet, since a screen
        // is half sheet and letters over it are a mess.
        assert!(
            printed.iter().any(|s| matches!(s, Shape::RoundedRect { color, .. } if color.r == orange.bg.r && color.g == orange.bg.g)),
            "the rows were left standing on the screen"
        );
    }

    /// A drop of anything holds a shape, and a perfect circle is the one it
    /// never holds (#147). The reach gives by direction and the give turns,
    /// so the surface is a little off round and never twice the same.
    #[test]
    fn a_ball_on_its_own_is_not_quite_a_circle() {
        let at = Pt::new(300.0, 200.0);
        let reach = 190.0;
        let want = lone(reach);
        let mut strayed = 0.0f32;
        for k in 0..40 {
            let t = k as f32 * 0.37;
            let balls = [Ball::soft(at, reach, "S-1", t)];
            let field = Sampled::of(&balls, reach * 0.02);
            let v: Vec<f32> =
                (0..field.nx * field.ny).map(|k| field.body(k % field.nx, k / field.nx)).collect();
            let rings = trace(&v, field.nx, field.ny, field.at, field.step);
            assert_eq!(rings.len(), 1, "t={t}: a ball on its own came out in pieces");
            for p in &rings[0] {
                let d = ((p.x - at.x).powi(2) + (p.y - at.y).powi(2)).sqrt();
                strayed = strayed.max((d - want).abs() / want);
            }
        }
        assert!(strayed > 0.01, "the ball is still a circle: it strayed {strayed} from round");
        // And not so far that the size the rows are cut to (`SQUISH`) is a
        // lie: everything laid on a body is measured against its tightest.
        assert!(strayed < SQUISH * 1.6, "the ball strayed {strayed} from round");
    }

    /// The whole point of the form (#140): the seating puts cluster mates a
    /// cell apart and everybody else two, and the reach is chosen so that
    /// the first pair is one body and the second is two.
    #[test]
    fn mates_run_together_and_strangers_do_not() {
        let r = 90.0;
        // `give` is the squish turned along the line between them (#147):
        // negative is both of them at their shortest reach towards each
        // other, positive is both at their longest.
        let one = |apart: f32, give: f32| {
            let g = [(give, 0.0), (give / 2.0, 0.0)];
            let balls =
                [Ball::with_give(Pt::new(0.0, 0.0), REACH * r, g), Ball::with_give(Pt::new(apart, 0.0), REACH * r, g)];
            let field = Sampled::of(&balls, r * STEP);
            let v: Vec<f32> = (0..field.nx * field.ny).map(|k| field.body(k % field.nx, k / field.nx)).collect();
        trace(&v, field.nx, field.ny, field.at, field.step).len()
        };
        // Neighbours in the honeycomb: a cell's height apart (#132).
        assert_eq!(one(r * 1.7320508, 0.0), 1, "two sessions of one cluster are one body");
        // And still one when both have walked (#142) and breathed (#144) as
        // far apart as either is ever allowed to get, with the surface drawn
        // in as far as it ever gives (#147).
        assert_eq!(
            one(r * (1.7320508 + 2.0 * (DRIFT + WANDER)), -SQUISH),
            1,
            "a cluster came apart at full stretch"
        );
        // The nearest two cells of different clusters can ever be, the rim
        // between them being the thing that says they are different (#121).
        assert_eq!(one(r * 3.0, SQUISH), 2, "two clusters never touch");
    }

    /// Where two bodies have run together the surface pulls itself in
    /// (#141): the waist is drawn in from where the two fields added
    /// together would leave it, which is the groove a film makes between two
    /// bubbles and, where it comes out at the air, the cusp. It is a
    /// narrowing and not a parting — the test above is the one that says so.
    #[test]
    fn the_waist_between_two_balls_is_drawn_in() {
        let r = 90.0;
        let apart = r * 1.7320508;
        let balls = [Ball::round(Pt::new(0.0, 0.0), REACH * r), Ball::round(Pt::new(apart, 0.0), REACH * r)];
        let field = Sampled::of(&balls, r * 0.01);
        let i = ((apart / 2.0 - field.at.x) / field.step).round() as usize;
        let j0 = ((0.0 - field.at.y) / field.step).round() as usize;
        // How far down the waist's own line each surface holds out.
        let half = |v: &dyn Fn(usize) -> f32| {
            let mut j = j0;
            while j + 1 < field.ny && v(j + 1) > 0.0 {
                j += 1;
            }
            (j - j0) as f32 * field.step
        };
        let pulled = half(&|j| field.body(i, j));
        let added = half(&|j| field.sum[j * field.nx + i] - LEVEL);
        assert!(pulled > 0.0, "the waist was pinched through: the body came apart");
        assert!(
            pulled < added * 0.8,
            "a waist of {pulled} against {added} of plain addition: the surface is not pulling"
        );
    }

    /// A body is divided between the balls that made it, with no ground
    /// left over and none claimed twice: that is what lets a run-together
    /// cluster still answer in colour which session a spot belongs to.
    #[test]
    fn a_body_is_divided_between_the_balls_that_made_it() {
        let r = 90.0;
        let balls = [Ball::round(Pt::new(0.0, 0.0), REACH * r), Ball::round(Pt::new(r * 1.7320508, 0.0), REACH * r)];
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

    fn board(groups: &[(&str, usize)]) -> BoardModel {
        use crate::model::{Phase, SessionInfo};
        let mut model = BoardModel::new();
        let mut list = Vec::new();
        let mut seq = 0;
        for (group, n) in groups {
            for _ in 0..*n {
                seq += 1;
                let mut s = SessionInfo::synthetic(seq, &format!("S-{seq}"), Phase::Working);
                s.group = (*group).to_string();
                list.push(s);
            }
        }
        model.apply(list, std::time::Instant::now());
        model.settle();
        model
    }

    /// The walk has to come to a stop, or the board never stops painting
    /// (#126). It stops a little off the seating it started from, which is
    /// the point of walking at all (#142).
    #[test]
    fn the_balls_come_to_rest_a_little_off_their_seats() {
        let model = board(&[("a", 3)]);
        let comb = comb::lay_out(&model, 980.0, 620.0);
        let mut sim = Sim::default();
        sim.settle(&model, &comb, 1.0e3);
        assert!(!sim.moving(), "the bodies never stopped moving");
        let mut off: f32 = 0.0;
        for cell in comb.cells.iter().filter(|c| c.chip.is_some()) {
            let id = &model.chips[cell.chip.unwrap()].info.session_id;
            let moved = (sim.walked(id, cell.center) - cell.center).len();
            assert!(moved <= comb.r * DRIFT + 0.01, "a ball sits {moved} from its seat");
            off = off.max(moved);
        }
        assert!(off > comb.r * 0.01, "nothing moved at all: the balls are not pressing on each other");

        // And however hard a body is shoved — by a neighbour growing into a
        // full cluster, or by a frame the machine sat on — it does not leave
        // the cell that answers for it.
        for bob in sim.bobs.values_mut() {
            bob.v = Pt::new(900.0, -900.0);
        }
        sim.advance(&model, &comb, 1.0e3, 1.0 / 30.0);
        for cell in comb.cells.iter().filter(|c| c.chip.is_some()) {
            let id = &model.chips[cell.chip.unwrap()].info.session_id;
            let moved = (sim.walked(id, cell.center) - cell.center).len();
            assert!(moved <= comb.r * DRIFT + 0.01, "a shoved ball reached {moved} from its seat");
        }
    }

    /// What the cap on that is for: the seat is what answers for a press
    /// (#86) and the nearest cell of another cluster is two away (#121), so
    /// the walk must not be able to carry two bodies into each other.
    #[test]
    fn no_amount_of_walking_runs_two_clusters_together() {
        let r = 90.0;
        // The nearest two cells of different clusters can be, with both of
        // them walked as far towards each other as they are ever allowed and
        // breathing in on top of that (#144).
        let apart = r * 3.0 - 2.0 * r * (DRIFT + WANDER);
        // And both of them let out as far as the squish ever lets them, on
        // the line between them (#147).
        let g = [(SQUISH, 0.0), (SQUISH / 2.0, 0.0)];
        let balls =
            [Ball::with_give(Pt::new(0.0, 0.0), REACH * r, g), Ball::with_give(Pt::new(apart, 0.0), REACH * r, g)];
        let field = Sampled::of(&balls, r * 0.02);
        let v: Vec<f32> = (0..field.nx * field.ny).map(|k| field.body(k % field.nx, k / field.nx)).collect();
        assert_eq!(trace(&v, field.nx, field.ny, field.at, field.step).len(), 2, "two clusters ran together");
    }

    /// A seat that moves takes its body with it (#138, #142). Not by
    /// sliding: the rim between two clusters is a single cell, and a body
    /// crossing it to catch up would run into somebody else's on the way.
    /// What the walk keeps is the lean the body had.
    #[test]
    fn a_body_goes_with_a_seat_that_moved_rather_than_after_it() {
        let model = board(&[("a", 1)]);
        let r = 90.0;
        let seated = |at: Pt| Comb {
            cells: vec![Cell { center: at, q: 0, row: 0, chip: Some(0), subs: None, wave: 0.0 }],
            r,
            height: 600.0,
        };
        let id = model.chips[0].info.session_id.clone();
        let from = Pt::new(300.0, 300.0);
        let next = Pt::new(300.0 + r * 1.7320508, 300.0);

        let mut sim = Sim::default();
        sim.settle(&model, &seated(from), 1.0e3);
        let lean = sim.walked(&id, from) - from;

        sim.advance(&model, &seated(next), 1.0e3, 1.0 / 60.0);
        let moved = sim.walked(&id, next) - next;
        assert!((moved - lean).len() < r * 0.02, "the body did not go with its seat");
        assert!(!sim.moving(), "and it was not sent moving by the move");
    }

    /// A colony is a cell of its session's cluster, so it is a ball of that
    /// cluster's body (#145): run together with it, and walked and breathed
    /// like any other member. On its own it was a clutch of specks in the
    /// gap between two bodies, belonging to whichever the eye picked.
    #[test]
    fn a_colony_is_one_body_with_the_session_it_belongs_to() {
        use crate::model::{Phase, SessionInfo, SubagentInfo};
        let mut model = BoardModel::new();
        let mut s = SessionInfo::synthetic(1, "S-1", Phase::Working);
        s.group = "warp-tab:1-1".into();
        s.subagents = (0..3).map(|i| SubagentInfo::synthetic(i + 1, "Explore", "", true)).collect();
        model.apply(vec![s], std::time::Instant::now());
        model.settle();
        let comb = comb::lay_out(&model, 980.0, 620.0);
        let mut sim = Sim::default();
        sim.settle(&model, &comb, 1.0e3);

        let id = model.chips[0].info.session_id.clone();
        let key = colony_key(&id);
        let session = comb.cells.iter().find(|c| c.chip.is_some()).expect("the session");
        let colony = comb.cells.iter().find(|c| c.subs.is_some()).expect("the colony");
        // The walk knows it as a member of its own: it is pushed off its
        // seat by the session next door, the way two sessions push.
        let walked = sim.walked(&key, colony.center);
        assert!((walked - colony.center).len() > 0.0, "the colony is not walked at all");
        assert!(walked != sim.walked(&id, session.center), "the colony walks as its session");

        let balls = [
            Ball::round(sim.at(&id, session.center, 2.0, comb.r), REACH * comb.r),
            Ball::round(sim.at(&key, colony.center, 2.0, comb.r), REACH * comb.r),
        ];
        let field = Sampled::of(&balls, comb.r * STEP);
        let v: Vec<f32> = (0..field.nx * field.ny).map(|k| field.body(k % field.nx, k / field.nx)).collect();
        assert_eq!(
            trace(&v, field.nx, field.ny, field.at, field.step).len(),
            1,
            "the colony came out a body of its own"
        );
    }

    /// The board is never quite still (#144): every body goes on wandering
    /// after the walk has stopped, each on its own phase, and none of them
    /// ever gets further out than the budget the seat answers for.
    #[test]
    fn the_bodies_never_stop_breathing() {
        let r = 90.0;
        let (mut most, mut apart, mut travelled) = (0.0f32, 0.0f32, 0.0f32);
        let here = wander("S-1", 0.0, r);
        for k in 0..2000 {
            let t = k as f32 * 0.05;
            let a = wander("S-1", t, r);
            let b = wander("S-2", t, r);
            most = most.max(a.len()).max(b.len());
            apart = apart.max((a - b).len());
            travelled = travelled.max((a - here).len());
        }
        assert!(most <= r * WANDER * 1.4143, "a body wandered {most} off, over its {} budget", r * WANDER);
        assert!(travelled > r * WANDER, "the board stands still: nothing wandered anywhere");
        assert!(apart > r * WANDER, "two bodies breathe as one: the phase is not theirs");
        // And the wander is of the clock alone, so two frames of the same
        // board at the same moment are the same picture.
        assert_eq!(wander("S-1", 3.5, r), wander("S-1", 3.5, r));
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
