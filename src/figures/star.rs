//! Star polygons {N/k} — the family that contains both the Hexagram and the
//! Pentagram.
//!
//! Connect every k-th of N points on a circle. {6/2} is two interlaced
//! triangles (the Star of David); {5/2} is the pentagram in one unbroken
//! stroke. The difference between "one continuous line" and "several separate
//! polygons" is gcd(N, k), and the construction handles both without caring.

use crate::geom::build::Build;
use crate::geom::construction::Role;
use crate::params::Params;

use super::hex::seed_frame;

pub fn star_polygon(b: &mut Build, p: &Params) {
    b.overlap(0.35);
    b.role(Role::Scaffold);
    let f = seed_frame(b, p, 1.0);

    let n = f.ring1.len();
    let k = (p.skip as usize).clamp(1, (n.saturating_sub(1) / 2).max(1));

    b.role(Role::Figure);
    // gcd(n, k) separate closed circuits, each stepping by k.
    let circuits = gcd(n, k);
    for start in 0..circuits {
        b.group("circuit");
        let mut i = start;
        loop {
            let j = (i + k) % n;
            b.line(f.ring1[i], f.ring1[j]);
            i = j;
            if i == start {
                break;
            }
        }
    }

    b.group("enclosure");
    b.width(1.8);
    b.trace(f.mother);
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// The Pentagram as a first-class catalogue entry. It was always reachable by
/// knob-fiddling the Hexagram down to {5/2}, but a figure this canonical
/// shouldn't require setup. The user's ratio/twist still apply; symmetry and
/// skip are pinned to the pentagram's own.
pub fn pentagram(b: &mut Build, p: &Params) {
    let p5 = Params { symmetry: 5, skip: 2, ..*p };
    star_polygon(b, &p5);
}
