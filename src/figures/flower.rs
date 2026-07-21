//! Flower of Life and Fruit of Life.

use crate::geom::build::Build;
use crate::geom::construction::Role;
use crate::params::Params;

use super::hex::{fruit_centers, lattice, seed_frame};

/// Every centre derived by the growth rule — no lattice table anywhere.
/// At symmetry 6 and rings 2 this is exactly the classical 19 circles.
pub fn flower_of_life(b: &mut Build, p: &Params) {
    b.overlap(0.5);
    b.role(Role::Scaffold);
    let f = seed_frame(b, p, 1.0);
    b.group("growth");
    let centers = lattice(b, p, &f);

    b.role(Role::Figure);
    b.group("the flower");
    b.trace(f.mother);
    for &c in &f.petals {
        b.trace(c);
    }
    // Skip the origin and first ring — already drawn as the seed frame.
    for &c in centers.iter().skip(1 + f.ring1.len()) {
        b.group_next();
        b.circle_r(c, f.chord);
    }
}

/// Thirteen circles, mutually tangent: radius is half the lattice spacing.
pub fn fruit_of_life(b: &mut Build, p: &Params) {
    b.overlap(0.4);
    b.role(Role::Scaffold);
    let (centers, f) = fruit_centers(b, p);

    b.role(Role::Figure);
    b.group("the fruit");
    for c in centers {
        b.group_next();
        b.circle_r(c, f.chord * 0.5);
    }
}
