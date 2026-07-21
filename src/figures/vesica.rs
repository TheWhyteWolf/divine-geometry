//! Vesica Piscis — the first construction, and the one every other starts from:
//! two circles of equal radius, each centred on the other's circumference.

use crate::geom::build::Build;
use crate::geom::construction::Role;
use crate::params::Params;

pub fn vesica(b: &mut Build, p: &Params) {
    // Only four gestures, so give each one room to breathe.
    b.pace(1.4);
    b.overlap(0.0);

    // ratio detunes the separation: 1.0 is the true vesica, where each centre
    // lies exactly on the other circle.
    let half = 0.5 * p.ratio as f64;
    let tw = p.twist_rad();
    let (s, c) = tw.sin_cos();
    let a = b.point(-half * c, -half * s);
    let d = b.point(half * c, half * s);

    b.role(Role::Scaffold);
    b.group("first compass");
    let ca = b.circle_r(a, 1.0);
    b.group("second compass");
    let cb = b.circle_r(d, 1.0);

    let m = b.meet(ca, cb);
    if m.len() < 2 {
        // Detuned past tangency — no lens to draw, just the axis.
        b.role(Role::Figure);
        b.group("axis");
        b.line(a, d);
        return;
    }
    let (top, bot) = (m.upper(), m.lower());

    b.role(Role::Figure);
    b.group("the lens");
    b.arc_between(ca, bot, top);
    b.arc_between(cb, top, bot);
    b.group("axes");
    b.line(top, bot);
    b.line(a, d);
}
