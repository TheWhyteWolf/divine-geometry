//! The Golden Spiral, built the way it is actually constructed: a square, one
//! compass swing to find φ, then quarter arcs that grow by φ each turn.
//!
//! The swing is the whole trick. From the midpoint of a unit square's base to
//! the opposite corner is √5/2; laying that off along the base gives
//! 1/2 + √5/2 = φ. Nothing here is a stored constant — φ is *solved*, and the
//! test checks the solved value against (1+√5)/2.

use std::f64::consts::FRAC_PI_2;

use glam::DVec2;

use crate::geom::build::Build;
use crate::geom::construction::Role;
use crate::params::Params;

pub fn golden_spiral(b: &mut Build, p: &Params) {
    b.pace(1.15);
    b.overlap(0.25);

    b.role(Role::Scaffold);
    b.group("the square");
    let a = b.point(0.0, 0.0);
    let c = b.point(1.0, 0.0);
    let d = b.point(1.0, 1.0);
    let e = b.point(0.0, 1.0);
    b.polygon(&[a, c, d, e]);

    // Bisect the base the honest way: the two circles through both base corners
    // meet on the perpendicular bisector.
    b.group("bisect the base");
    let ka = b.circle(a, c);
    let kc = b.circle(c, a);
    let bis = b.meet(ka, kc);
    let base_line = b.line_silent(a, c);
    let perp = b.line_silent(bis.upper(), bis.lower());
    let mid = b.meet(perp, base_line).upper();

    // The swing that finds φ: from the base midpoint through the far corner.
    b.group("swing to phi");
    let swing = b.circle(mid, d);
    let a_pos = b.pos(a);
    let far = b.meet(swing, base_line).far_from(a_pos);
    let phi = b.pos(far).x;

    b.role(Role::Figure);
    b.group("golden rectangle");
    let top = b.point(phi, 1.0);
    b.polygon(&[a, far, top, e]);

    // Quarter arcs, each tangent to the last and φ times larger.
    //
    // Arc k runs from θ to θ+90° about centre C with radius r, so it ends at
    // P = C + r·û(θ+90°). For the next arc to start there with radius rφ at
    // angle θ+90°, its centre must be P − rφ·û — that is, C + (r − rφ)·û. Both
    // position and tangent then carry over exactly, which is what makes it one
    // continuous curve rather than a chain of arcs that nearly line up.
    // Winding *inward* — each arc is 1/φ of the last. Growing outward instead
    // would work identically, but after nine turns the spiral is φ⁹ ≈ 76 across
    // and normalization shrinks the square that generated it to invisibility.
    // Curling in keeps the construction itself the largest thing on screen.
    b.group("the spiral");
    let turns = (p.rings + 8).min(14);
    let mut centre = DVec2::new(1.0, 1.0);
    let mut r = 1.0f64;
    let mut th = FRAC_PI_2;

    for _ in 0..turns {
        let pivot = b.point(centre.x, centre.y);
        b.group_next();
        b.arc_at(pivot, r, th, th + FRAC_PI_2);

        let nth = th + FRAC_PI_2;
        let nr = r / phi;
        let u = DVec2::new(nth.cos(), nth.sin());
        centre += u * (r - nr);
        r = nr;
        th = nth;
    }
}
