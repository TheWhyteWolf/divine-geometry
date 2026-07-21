//! The N-fold frame every figure in this family grows from.
//!
//! Mother circle, N rim points walked out with the compass, then a growth rule
//! that derives each new generation from intersections of the previous one.
//!
//! The classical figures are the N = 6 case. Setting the compass to the side of
//! the inscribed N-gon (`2R·sin(π/N)`) makes the walk close after exactly N
//! steps for *any* N — at N = 6 that chord equals R, which is why the hexagonal
//! construction is the one that looks like it was meant to be.

use glam::DVec2;

use crate::geom::build::Build;
use crate::geom::isect::EPS;
use crate::geom::registry::{CurveId, PointId};
use crate::params::Params;

#[allow(dead_code)]
pub struct Frame {
    pub o: PointId,
    pub mother: CurveId,
    /// The N rim points, at radius R.
    pub ring1: Vec<PointId>,
    /// The N compass circles centred on them.
    pub petals: Vec<CurveId>,
    /// The compass span used for the walk — the N-gon side length.
    pub chord: f64,
    pub r: f64,
}

/// Chord subtending one N-th of a circle of radius `r`.
pub fn ngon_side(r: f64, n: u32) -> f64 {
    2.0 * r * (std::f64::consts::PI / n as f64).sin()
}

/// Mother circle plus the N-petal compass walk.
///
/// Each walking circle is centred on a rim point and spans the N-gon side, so
/// its two intersections with the mother are that point's ±(2π/N) neighbours.
/// Stepping to the one we did not come from walks the rim exactly once.
pub fn seed_frame(b: &mut Build, p: &Params, r: f64) -> Frame {
    let n = p.symmetry.max(3);
    let chord = ngon_side(r, n);
    let tw = p.twist_rad();

    let o = b.point(0.0, 0.0);
    let e = b.point(r * tw.cos(), r * tw.sin());

    b.group("mother circle");
    let mother = b.circle(o, e);

    let mut ring1 = Vec::with_capacity(n as usize);
    let mut petals = Vec::with_capacity(n as usize);
    let mut prev: Option<PointId> = None;
    let mut rim = e;
    for _ in 0..n {
        b.group_next();
        let walk = b.circle_r(rim, chord);
        ring1.push(rim);
        petals.push(walk);
        let m = b.meet(walk, mother);
        let next = match prev {
            Some(q) => m.other_than(q),
            None => m.upper(),
        };
        prev = Some(rim);
        rim = next;
    }
    // The walk closes: after N steps `rim` is back at `e`, and interning it
    // reports "not new". A broken dedup would silently produce an N+1-th point
    // and leave a visible gap in the rosette.
    debug_assert_eq!(rim, e, "the {n}-fold compass walk did not close");

    // A detuned petal radius is a separate family of circles on the same rim.
    // At ratio == 1 the walking circles *are* the petals, so drawing them again
    // would only double the ink.
    if (p.ratio - 1.0).abs() > 1e-3 {
        b.group("petals");
        let pr = chord * p.ratio as f64;
        for &c in &ring1 {
            petals.push(b.circle_r(c, pr));
        }
    }

    Frame { o, mother, ring1, petals, chord, r }
}

/// One generation of lattice growth.
///
/// Every pair of existing circles that actually overlap is intersected, and any
/// solved point inside `max_radius` that isn't already a centre becomes one.
/// This is the same rule the classical Flower of Life obeys — and because it's
/// stated as a rule rather than a table, it works for any symmetry order, any
/// number of generations, and (in infinite mode) forever.
pub fn grow(
    b: &mut Build,
    centers: &[PointId],
    r: f64,
    max_radius: f64,
) -> Vec<PointId> {
    let known: std::collections::HashSet<PointId> = centers.iter().copied().collect();
    let pos: Vec<DVec2> = centers.iter().map(|&c| b.pos(c)).collect();

    // Circles are registered silently: growth is a solving step, and drawing
    // every probe circle would bury the figure.
    let circles: Vec<CurveId> = centers.iter().map(|&c| b.circle_silent(c, r)).collect();

    let mut found: Vec<PointId> = Vec::new();
    let mut seen: std::collections::HashSet<PointId> = std::collections::HashSet::new();
    for i in 0..centers.len() {
        for j in i + 1..centers.len() {
            // Only pairs whose circles actually meet.
            let d = (pos[i] - pos[j]).length();
            if d > 2.0 * r + EPS || d < EPS {
                continue;
            }
            for &id in b.meet(circles[i], circles[j]).all() {
                if known.contains(&id) || !seen.insert(id) {
                    continue;
                }
                if b.pos(id).length() <= max_radius + 1e-6 {
                    found.push(id);
                }
            }
        }
    }
    // Deterministic order: inner rings first, then by angle. Growth order is
    // what the drawing order inherits, so it has to be stable across runs.
    found.sort_by(|&a, &c| {
        let (pa, pc) = (b.pos(a), b.pos(c));
        pa.length()
            .partial_cmp(&pc.length())
            .unwrap()
            .then(pa.y.atan2(pa.x).partial_cmp(&pc.y.atan2(pc.x)).unwrap())
    });
    found
}

/// Hard ceiling on lattice size.
///
/// At symmetry 6 growth is self-limiting: new intersections land exactly on
/// existing lattice points, so each generation adds one ring and converges. At
/// every *other* order the lattice does not tile the plane, so intersections
/// miss each other by small amounts, dedup (correctly) keeps them distinct, and
/// the count multiplies each generation — a sevenfold seed at four rings runs
/// to hundreds of thousands of circles. The geometry is not wrong; it is just
/// unbounded, so a user-facing seed knob needs a budget behind it.
#[allow(dead_code)]
pub const MAX_CENTERS: usize = 220;

/// Grow the lattice out to `p.rings` generations, returning every centre.
pub fn lattice(b: &mut Build, p: &Params, f: &Frame) -> Vec<PointId> {
    let mut centers: Vec<PointId> = std::iter::once(f.o).chain(f.ring1.iter().copied()).collect();
    let limit = f.chord * p.rings as f64;
    // Grow until the disc of radius `rings` stops yielding anything new, rather
    // than for a fixed number of passes: one generation does not reach one ring
    // outward, so a fixed count leaves the outermost ring half-built.
    for _ in 0..p.rings + 3 {
        if centers.len() >= MAX_CENTERS {
            break;
        }
        let mut fresh = grow(b, &centers, f.chord, limit);
        if fresh.is_empty() {
            break;
        }
        // `grow` returns inner rings first, so truncating drops the outermost
        // and least visually important circles.
        fresh.truncate(MAX_CENTERS - centers.len());
        centers.extend(fresh);
    }
    centers
}

/// The thirteen Fruit-of-Life centres (at N = 6): origin, inner ring, and one
/// compass step further out along each spoke.
pub fn fruit_centers(b: &mut Build, p: &Params) -> (Vec<PointId>, Frame) {
    let f = seed_frame(b, p, 1.0);
    let o_pos = b.pos(f.o);
    let n = f.ring1.len();

    let mut ring2 = Vec::with_capacity(n);
    for k in 0..n {
        b.group_next();
        let spoke = b.line_silent(f.o, f.ring1[k]);
        let step = b.circle_silent(f.ring1[k], f.chord);
        ring2.push(b.meet(step, spoke).far_from(o_pos));
    }

    let centers: Vec<PointId> = std::iter::once(f.o)
        .chain(f.ring1.iter().copied())
        .chain(ring2)
        .collect();
    (centers, f)
}
