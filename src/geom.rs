//! Small 2-D helpers: points and arc-length parametrised polylines.

use std::ops::{Add, Mul, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

impl Pt {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn len(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    pub fn norm(self) -> Pt {
        let l = self.len();
        if l <= 1e-6 { Pt::new(1.0, 0.0) } else { Pt::new(self.x / l, self.y / l) }
    }
    /// Rotate 90° clockwise in screen space (y down).
    pub fn perp(self) -> Pt {
        Pt::new(-self.y, self.x)
    }
    pub fn dot(self, o: Pt) -> f32 {
        self.x * o.x + self.y * o.y
    }
    pub fn lerp(self, o: Pt, t: f32) -> Pt {
        Pt::new(self.x + (o.x - self.x) * t, self.y + (o.y - self.y) * t)
    }
    pub fn angle_deg(self) -> f32 {
        self.y.atan2(self.x).to_degrees()
    }
}

impl Add for Pt {
    type Output = Pt;
    fn add(self, o: Pt) -> Pt {
        Pt::new(self.x + o.x, self.y + o.y)
    }
}
impl Sub for Pt {
    type Output = Pt;
    fn sub(self, o: Pt) -> Pt {
        Pt::new(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f32> for Pt {
    type Output = Pt;
    fn mul(self, s: f32) -> Pt {
        Pt::new(self.x * s, self.y * s)
    }
}

/// A polyline with cumulative arc lengths, supporting lookup by distance.
#[derive(Clone, Debug)]
pub struct Polyline {
    pub pts: Vec<Pt>,
    cum: Vec<f32>,
}

impl Polyline {
    pub fn new(pts: Vec<Pt>) -> Self {
        let mut cum = Vec::with_capacity(pts.len());
        let mut acc = 0.0;
        for (i, p) in pts.iter().enumerate() {
            if i > 0 {
                acc += (*p - pts[i - 1]).len();
            }
            cum.push(acc);
        }
        Self { pts, cum }
    }

    /// Builds a polyline through `corners`, rounding every interior corner
    /// with a quadratic curve of roughly `radius`.
    pub fn rounded(corners: &[Pt], radius: f32) -> Self {
        if corners.len() < 3 {
            return Self::new(corners.to_vec());
        }
        let mut pts = vec![corners[0]];
        for i in 1..corners.len() - 1 {
            let a = corners[i - 1];
            let c = corners[i];
            let b = corners[i + 1];
            let da = (a - c).len();
            let db = (b - c).len();
            let d = radius.min(da / 2.0).min(db / 2.0);
            let p1 = c + (a - c).norm() * d;
            let p2 = c + (b - c).norm() * d;
            pts.push(p1);
            const STEPS: usize = 7;
            for k in 1..STEPS {
                let t = k as f32 / STEPS as f32;
                let q = p1.lerp(c, t).lerp(c.lerp(p2, t), t);
                pts.push(q);
            }
            pts.push(p2);
        }
        pts.push(*corners.last().unwrap());
        Self::new(pts)
    }

    pub fn length(&self) -> f32 {
        *self.cum.last().unwrap_or(&0.0)
    }

    /// Position and unit tangent at arc length `s` (clamped).
    pub fn point_at(&self, s: f32) -> (Pt, Pt) {
        let n = self.pts.len();
        if n == 0 {
            return (Pt::default(), Pt::new(1.0, 0.0));
        }
        if n == 1 {
            return (self.pts[0], Pt::new(1.0, 0.0));
        }
        let s = s.clamp(0.0, self.length());
        let idx = match self.cum.binary_search_by(|c| c.partial_cmp(&s).unwrap()) {
            Ok(i) => i.min(n - 2),
            Err(i) => i.saturating_sub(1).min(n - 2),
        };
        let a = self.pts[idx];
        let b = self.pts[idx + 1];
        let seg = self.cum[idx + 1] - self.cum[idx];
        let t = if seg <= 1e-6 { 0.0 } else { (s - self.cum[idx]) / seg };
        (a.lerp(b, t), (b - a).norm())
    }

    /// Points between arc lengths `s0` and `s1` (inclusive endpoints).
    pub fn slice(&self, s0: f32, s1: f32) -> Vec<Pt> {
        let s0 = s0.clamp(0.0, self.length());
        let s1 = s1.clamp(0.0, self.length());
        if s1 - s0 <= 0.01 {
            return Vec::new();
        }
        let mut out = vec![self.point_at(s0).0];
        for (i, c) in self.cum.iter().enumerate() {
            if *c > s0 && *c < s1 {
                out.push(self.pts[i]);
            }
        }
        out.push(self.point_at(s1).0);
        out
    }

    /// Arc length where the polyline first reaches `x`, if it gets there.
    /// Lanes run left to right, so for them the answer is unambiguous (#56).
    pub fn s_at_x(&self, x: f32) -> Option<f32> {
        for i in 0..self.pts.len().saturating_sub(1) {
            let (a, b) = (self.pts[i], self.pts[i + 1]);
            if a.x <= x && x <= b.x {
                let d = b.x - a.x;
                let t = if d.abs() <= 1e-6 { 0.0 } else { (x - a.x) / d };
                return Some(self.cum[i] + (self.cum[i + 1] - self.cum[i]) * t);
            }
        }
        None
    }

    /// Arc length of the point on the polyline nearest to `p`.
    pub fn nearest_s(&self, p: Pt) -> f32 {
        let mut best = (f32::MAX, 0.0);
        for i in 0..self.pts.len().saturating_sub(1) {
            let a = self.pts[i];
            let b = self.pts[i + 1];
            let ab = b - a;
            let l2 = ab.dot(ab);
            let t = if l2 <= 1e-6 { 0.0 } else { ((p - a).dot(ab) / l2).clamp(0.0, 1.0) };
            let q = a + ab * t;
            let d = (p - q).len();
            if d < best.0 {
                best = (d, self.cum[i] + (self.cum[i + 1] - self.cum[i]) * t);
            }
        }
        best.1
    }
}

pub fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t)
}
