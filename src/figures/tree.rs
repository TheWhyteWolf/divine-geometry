//! The Tree of Life — ten sephiroth and the twenty-two paths between them.
//!
//! The node positions are seeds rather than solved points: the tree's layout is
//! its definition, the way the Vesica's two centres are. What *is* derived is
//! the vesica lattice it sits on — each sephira gets a circle whose radius is
//! the pillar spacing, and adjacent circles pass through one another's centres,
//! which is the sense in which the tree "lives on" the Flower of Life.

use crate::geom::build::Build;
use crate::geom::construction::Role;
use crate::params::Params;

/// (x, y) in pillar-widths. Three pillars, seven horizontal levels.
const SEPHIROTH: [(f64, f64); 10] = [
    (0.0, 3.0),   // Kether
    (1.0, 2.0),   // Chokmah
    (-1.0, 2.0),  // Binah
    (1.0, 1.0),   // Chesed
    (-1.0, 1.0),  // Geburah
    (0.0, 0.0),   // Tiphareth
    (1.0, -1.0),  // Netzach
    (-1.0, -1.0), // Hod
    (0.0, -2.0),  // Yesod
    (0.0, -3.0),  // Malkuth
];

/// The twenty-two paths, one per letter of the Hebrew alphabet.
const PATHS: [(usize, usize); 22] = [
    (0, 1),
    (0, 2),
    (0, 5),
    (1, 2),
    (1, 3),
    (1, 5),
    (2, 4),
    (2, 5),
    (3, 4),
    (3, 5),
    (3, 6),
    (4, 5),
    (4, 7),
    (5, 6),
    (5, 7),
    (5, 8),
    (6, 7),
    (6, 8),
    (6, 9),
    (7, 8),
    (7, 9),
    (8, 9),
];

pub fn tree_of_life(b: &mut Build, p: &Params) {
    b.pace(1.1);
    b.overlap(0.5);
    let tw = p.twist_rad();
    let (s, c) = tw.sin_cos();
    // Scale so the tree fills the frame at roughly the classical proportion.
    let k = 0.30;

    let nodes: Vec<_> = SEPHIROTH
        .iter()
        .map(|&(x, y)| {
            let (px, py) = (x * k, y * k);
            b.point(px * c - py * s, px * s + py * c)
        })
        .collect();

    // Each sephira carries a circle of the pillar radius; neighbouring circles
    // pass through one another's centres, which is the vesica relation the
    // whole lattice is built on.
    b.role(Role::Scaffold);
    b.group("the vessels");
    for &n in &nodes {
        b.group_next();
        b.circle_r(n, k);
    }

    b.role(Role::Figure);
    b.width(1.9);
    for &(i, j) in PATHS.iter() {
        b.group_next();
        b.line(nodes[i], nodes[j]);
    }

    b.group("the sephiroth");
    b.width(2.6);
    for &n in &nodes {
        b.circle_r(n, k * 0.30);
    }
}
