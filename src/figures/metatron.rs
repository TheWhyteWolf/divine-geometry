//! Metatron's Cube — the Fruit-of-Life centres joined by every chord.

use crate::geom::build::Build;
use crate::geom::construction::Role;
use crate::params::Params;

use super::hex::fruit_centers;

pub fn metatron(b: &mut Build, p: &Params) {
    // At symmetry 6 that's 78 chords; they have to rain, not queue.
    b.overlap(0.65);
    b.role(Role::Scaffold);
    let (centers, f) = fruit_centers(b, p);

    b.group("fruit of life");
    for &c in &centers {
        b.circle_r(c, f.chord * 0.5);
    }

    // Longest chord first: the big diameters read as structure, the short ones
    // as detail. Drawing order is a script-level concern — the engine needs to
    // know nothing about it.
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for i in 0..centers.len() {
        for j in i + 1..centers.len() {
            pairs.push((i, j));
        }
    }
    pairs.sort_by(|&(a, c), &(d, e)| {
        let la = b.pos(centers[a]).distance(b.pos(centers[c]));
        let lb = b.pos(centers[d]).distance(b.pos(centers[e]));
        lb.partial_cmp(&la).unwrap()
    });

    b.role(Role::Figure);
    b.width(1.6);
    for (i, j) in pairs {
        b.group_next();
        b.line(centers[i], centers[j]);
    }
}
