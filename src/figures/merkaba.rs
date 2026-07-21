//! Star Tetrahedron (Merkaba), the 64 Tetrahedron Grid, and the Vector
//! Equilibrium — the polyhedral family, all read as 2D projections down an axis
//! of symmetry.

use crate::geom::build::Build;
use crate::geom::construction::Role;
use crate::geom::registry::PointId;
use crate::params::Params;

use super::hex::{grow, seed_frame};

/// Two interpenetrating tetrahedra. Down a threefold axis each projects to a
/// triangle, and the pair projects to a hexagram with the internal edges shown
/// — which is why the Merkaba and the Star of David look alike on paper.
pub fn merkaba(b: &mut Build, p: &Params) {
    b.overlap(0.35);
    b.role(Role::Scaffold);
    let f = seed_frame(b, p, 1.0);
    let n = f.ring1.len();

    b.role(Role::Figure);
    // Alternate vertices form one tetrahedron's projection, the rest the other.
    // That split needs an even order with at least three vertices a side; below
    // that there is only one polygon to draw.
    let (up, down): (Vec<PointId>, Vec<PointId>) = if n >= 6 && n % 2 == 0 {
        ((0..n).step_by(2).map(|i| f.ring1[i]).collect(), (1..n).step_by(2).map(|i| f.ring1[i]).collect())
    } else {
        (f.ring1.clone(), Vec::new())
    };
    b.group("upward tetrahedron");
    b.polygon(&up);
    if !down.is_empty() {
        b.group("downward tetrahedron");
        b.polygon(&down);
    }

    // The edges running to the apex: in projection they meet at the centre.
    b.group("apex edges");
    b.width(1.5);
    for &v in up.iter().chain(down.iter()) {
        b.line(f.o, v);
    }

    b.group("enclosure");
    b.width(1.8);
    b.trace(f.mother);
}

/// The 64 Tetrahedron Grid — the isotropic vector matrix seen down its axis:
/// a triangular lattice where every nearest-neighbour pair is an edge.
pub fn grid_64(b: &mut Build, p: &Params) {
    b.overlap(0.7);
    b.role(Role::Scaffold);
    let f = seed_frame(b, p, 1.0);

    b.group("growth");
    let mut centers: Vec<PointId> =
        std::iter::once(f.o).chain(f.ring1.iter().copied()).collect();
    let limit = f.chord * p.rings as f64;
    // The strut count is quadratic in the centre count, so this grid needs a
    // tighter budget than the lattice figures.
    let cap = super::hex::MAX_CENTERS.min(120);
    for _ in 0..p.rings.max(2) {
        if centers.len() >= cap {
            break;
        }
        let mut fresh = grow(b, &centers, f.chord, limit);
        if fresh.is_empty() {
            break;
        }
        fresh.truncate(cap - centers.len());
        centers.extend(fresh);
    }

    // Every nearest-neighbour pair is a strut of the matrix.
    let pos: Vec<_> = centers.iter().map(|&c| b.pos(c)).collect();
    let mut edges: Vec<(usize, usize, f64)> = Vec::new();
    for i in 0..centers.len() {
        for j in i + 1..centers.len() {
            let d = (pos[i] - pos[j]).length();
            if (d - f.chord).abs() < 1e-6 {
                edges.push((i, j, pos[i].length().max(pos[j].length())));
            }
        }
    }
    // Inner struts first, so the matrix visibly grows outward from the core.
    edges.sort_by(|a, c| a.2.partial_cmp(&c.2).unwrap());

    b.role(Role::Figure);
    b.width(1.5);
    for (i, j, _) in edges {
        b.group_next();
        b.line(centers[i], centers[j]);
    }
}

/// Vector Equilibrium (cuboctahedron): twelve vertices all one strut-length
/// from the centre *and* from their neighbours — the only arrangement where
/// radial and circumferential distances are equal.
pub fn vector_equilibrium(b: &mut Build, p: &Params) {
    b.overlap(0.4);
    b.role(Role::Scaffold);
    let f = seed_frame(b, p, 1.0);
    let n = f.ring1.len();

    b.role(Role::Figure);
    b.group("radials");
    b.width(1.6);
    for &v in &f.ring1 {
        b.line(f.o, v);
    }
    b.group("girdle");
    b.width(2.2);
    b.polygon(&f.ring1);

    // The square faces read as the long diagonals in projection.
    b.group("faces");
    b.width(1.4);
    for i in 0..n {
        b.line(f.ring1[i], f.ring1[(i + 2) % n]);
    }

    b.group("enclosure");
    b.width(1.8);
    b.trace(f.mother);
}
