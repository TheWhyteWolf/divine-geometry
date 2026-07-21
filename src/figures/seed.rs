//! Seed of Life and Egg of Life — the N-petal rosette.

use crate::geom::build::Build;
use crate::geom::construction::Role;
use crate::params::Params;

use super::hex::seed_frame;

pub fn seed_of_life(b: &mut Build, p: &Params) {
    b.overlap(0.3);
    b.role(Role::Scaffold);
    let f = seed_frame(b, p, 1.0);

    b.role(Role::Figure);
    b.group("rosette");
    for &c in &f.petals {
        b.trace(c);
    }
    b.trace(f.mother);
}

/// Egg of Life — half-span circles on the rim points, so they meet at exact
/// tangency rather than overlapping. This is the figure that exercises the
/// intersection solver's tangency branch hardest.
pub fn egg_of_life(b: &mut Build, p: &Params) {
    b.overlap(0.25);
    b.role(Role::Scaffold);
    let f = seed_frame(b, p, 1.0);

    b.role(Role::Figure);
    b.group("the egg");
    for &c in &f.ring1 {
        b.circle_r(c, f.chord * 0.5);
    }
}
