//! Point and curve registries — where construction identity is decided.
//!
//! Every derived point funnels through `PointRegistry::intern`. Exact dedup is
//! what makes `Meet::other_than` work at all: "step the compass to the
//! intersection that isn't where I already am" is a `PointId` comparison, and it
//! only means anything if the same geometric point always gets the same id.

use std::collections::HashMap;

use glam::DVec2;

use super::isect::EPS;

/// Spatial-hash cell size. Must be ≥ EPS so an EPS-ball spans at most one cell
/// index per axis; 2·EPS makes that strictly safe.
const CELL: f64 = 2.0 * EPS;

pub type PointId = u32;
pub type CurveId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointKind {
    Seed,
    CircleCircle,
    LineCircle,
    LineLine,
}

#[derive(Clone, Copy, Debug)]
pub struct Pt {
    pub p: DVec2,
    pub kind: PointKind,
    /// Group ordinal at which this point came into existence — drives the
    /// node birth flash.
    pub born: u16,
}

#[derive(Default)]
pub struct PointRegistry {
    pts: Vec<Pt>,
    grid: HashMap<(i64, i64), Vec<PointId>>,
}

impl PointRegistry {
    #[inline]
    fn cell(p: DVec2) -> (i64, i64) {
        ((p.x / CELL).floor() as i64, (p.y / CELL).floor() as i64)
    }

    /// Insert-or-find. Returns `(id, is_new)`.
    ///
    /// The 3×3 neighbour probe is load-bearing, not defensive. Sacred geometry
    /// places points on *exact* lattice coordinates, so two computations of the
    /// same point landing 1e-12 apart across a cell boundary is the common case,
    /// not a rare one. Probing only the home cell lets both survive — and the
    /// Flower of Life then reports 38 circles instead of 19.
    pub fn intern(&mut self, p: DVec2, kind: PointKind, born: u16) -> (PointId, bool) {
        let (cx, cy) = Self::cell(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(bucket) = self.grid.get(&(cx + dx, cy + dy)) {
                    for &id in bucket {
                        if (self.pts[id as usize].p - p).length_squared() <= EPS * EPS {
                            return (id, false);
                        }
                    }
                }
            }
        }
        let id = self.pts.len() as PointId;
        self.pts.push(Pt { p, kind, born });
        self.grid.entry((cx, cy)).or_default().push(id);
        (id, true)
    }

    #[inline]
    pub fn pos(&self, id: PointId) -> DVec2 {
        self.pts[id as usize].p
    }

    pub fn get(&self, id: PointId) -> Pt {
        self.pts[id as usize]
    }

    pub fn len(&self) -> usize {
        self.pts.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (PointId, &Pt)> {
        self.pts.iter().enumerate().map(|(i, p)| (i as PointId, p))
    }

    /// Rescale every point about the origin — used by the finish-time
    /// normalization so EPS means the same thing across figures.
    pub fn rescale(&mut self, center: DVec2, k: f64) {
        for p in &mut self.pts {
            p.p = (p.p - center) * k;
        }
        self.grid.clear();
        for (i, p) in self.pts.iter().enumerate() {
            self.grid.entry(Self::cell(p.p)).or_default().push(i as PointId);
        }
    }
}

/// The infinite geometric object a curve lies on.
#[derive(Clone, Copy, Debug)]
pub enum Curve {
    Circle { c: DVec2, r: f64 },
    /// The infinite line through a and b.
    Line { a: DVec2, b: DVec2 },
}

#[derive(PartialEq, Eq, Hash, Debug)]
enum CurveKey {
    Circle(i64, i64, i64),
    Line(i64, i64, i64),
}

fn q(v: f64) -> i64 {
    (v / EPS).round() as i64
}

fn key_of(c: &Curve) -> CurveKey {
    match *c {
        Curve::Circle { c, r } => CurveKey::Circle(q(c.x), q(c.y), q(r)),
        Curve::Line { a, b } => {
            let d = (b - a).normalize();
            let mut n = DVec2::new(-d.y, d.x);
            // Canonical orientation so (a,b) and (b,a) collapse to one key.
            if n.x < -EPS || (n.x.abs() <= EPS && n.y < 0.0) {
                n = -n;
            }
            CurveKey::Line(q(n.x), q(n.y), q(n.dot(a)))
        }
    }
}

#[derive(Default)]
pub struct CurveRegistry {
    curves: Vec<Curve>,
    /// Dedups the *infinite* object. Used only for `meet` — intersecting the
    /// same line twice must give the same points.
    keys: HashMap<CurveKey, CurveId>,
}

impl CurveRegistry {
    /// Register a curve, returning the existing id if this exact infinite object
    /// is already known. The bool is false when it was already present.
    pub fn intern(&mut self, c: Curve) -> (CurveId, bool) {
        let k = key_of(&c);
        if let Some(&id) = self.keys.get(&k) {
            return (id, false);
        }
        let id = self.curves.len() as CurveId;
        self.curves.push(c);
        self.keys.insert(k, id);
        (id, true)
    }

    pub fn get(&self, id: CurveId) -> Curve {
        self.curves[id as usize]
    }

    pub fn len(&self) -> usize {
        self.curves.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (CurveId, &Curve)> {
        self.curves.iter().enumerate().map(|(i, c)| (i as CurveId, c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coincident_points_merge_across_cell_boundaries() {
        let mut r = PointRegistry::default();
        // Straddle a cell boundary deliberately: exactly on it, and a hair below.
        let on = DVec2::new(CELL * 4.0, CELL * 4.0);
        let (a, new_a) = r.intern(on, PointKind::Seed, 0);
        let (b, new_b) = r.intern(on - DVec2::splat(1e-12), PointKind::CircleCircle, 1);
        assert!(new_a);
        assert!(!new_b, "boundary-straddling duplicate was not merged");
        assert_eq!(a, b);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn genuinely_distinct_points_stay_distinct() {
        let mut r = PointRegistry::default();
        let (a, _) = r.intern(DVec2::ZERO, PointKind::Seed, 0);
        // 1e-4 apart is five orders above EPS — must not merge.
        let (b, new) = r.intern(DVec2::new(1e-4, 0.0), PointKind::Seed, 0);
        assert!(new);
        assert_ne!(a, b);
    }

    #[test]
    fn line_key_is_orientation_independent() {
        let mut r = CurveRegistry::default();
        let a = DVec2::new(-1.0, 0.5);
        let b = DVec2::new(3.0, 0.5);
        let (id0, new0) = r.intern(Curve::Line { a, b });
        // Same infinite line, opposite direction and different sample points.
        let (id1, new1) = r.intern(Curve::Line { a: b, b: a });
        let (id2, new2) = r.intern(Curve::Line {
            a: DVec2::new(10.0, 0.5),
            b: DVec2::new(-7.0, 0.5),
        });
        assert!(new0);
        assert!(!new1 && !new2, "the same infinite line was registered twice");
        assert_eq!(id0, id1);
        assert_eq!(id0, id2);
    }
}
