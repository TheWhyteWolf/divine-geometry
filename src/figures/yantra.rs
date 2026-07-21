//! Sri Yantra — nine interlocking triangles, the bindu, two lotus rings and the
//! bhupura.
//!
//! This is the one figure in the set whose triangle net is **not** solved at
//! run time, and it is worth being straight about why. The nine triangles are
//! required to meet three-at-a-point at every crossing, and Kulaichev (1984)
//! showed that system has no exact solution — every Sri Yantra ever drawn is an
//! approximation to some tolerance. The proportions below come from the
//! classical 48-part division of the diameter, solved offline; treating them as
//! the definition of the figure is more honest than pretending a compass
//! derives them.
//!
//! Everything *around* the triangles is still constructed: the marma points
//! where the edges cross are solved intersections, and the lotus petals are
//! genuine vesica lenses between paired circles.

use std::f64::consts::TAU;

use crate::geom::build::Build;
use crate::geom::construction::Role;
use crate::geom::registry::PointId;
use crate::params::Params;

/// (base height, base half-width, apex height, points up?) — the classical
/// proportions, as fractions of the enclosing circle's radius.
const TRI: [(f64, f64, f64, bool); 9] = [
    // Five downward — Shakti.
    (0.750, 0.5424, -0.500, false),
    (0.500, 0.7101, -0.940, false),
    (0.291_667, 0.6074, -0.750, false),
    (0.166_667, 0.3814, -0.250, false),
    (0.041_667, 0.2411, -0.125, false),
    // Four upward — Shiva.
    (-0.750, 0.5424, 0.500, true),
    (-0.500, 0.7101, 0.940, true),
    (-0.250, 0.5869, 0.750, true),
    (-0.125, 0.4019, 0.291_667, true),
];

pub fn sri_yantra(b: &mut Build, p: &Params) {
    b.pace(1.15);
    b.overlap(0.5);
    let tw = p.twist_rad();
    let rot = |x: f64, y: f64| -> (f64, f64) {
        let (s, c) = tw.sin_cos();
        (x * c - y * s, x * s + y * c)
    };

    b.role(Role::Figure);
    let mut edges: Vec<(PointId, PointId)> = Vec::new();
    for &(base_y, half, apex_y, up) in TRI.iter() {
        let (lx, ly) = rot(-half, base_y);
        let (rx, ry) = rot(half, base_y);
        let (ax, ay) = rot(0.0, apex_y);
        let l = b.point(lx, ly);
        let r = b.point(rx, ry);
        let apex = b.point(ax, ay);

        b.group_next();
        b.width(if up { 2.1 } else { 1.9 });
        b.line(l, r);
        b.line(r, apex);
        b.line(apex, l);
        edges.push((l, r));
        edges.push((r, apex));
        edges.push((apex, l));
    }

    // Marma points: where the triangle edges cross. These are solved, and they
    // are what the whole net is trying to make concurrent.
    b.role(Role::Scaffold);
    b.group("marma points");
    let lines: Vec<_> = edges.iter().map(|&(a, c)| b.line_silent(a, c)).collect();
    for i in 0..lines.len() {
        for j in i + 1..lines.len() {
            let _ = b.meet(lines[i], lines[j]);
        }
    }

    b.role(Role::Figure);
    b.group("bindu");
    b.width(2.6);
    let o = b.point(0.0, 0.0);
    b.circle_r(o, 0.030);

    b.group("enclosure");
    b.width(1.9);
    b.circle_r(o, 1.0);
    b.circle_r(o, 1.035);

    b.role(Role::Scaffold);
    b.group("inner lotus");
    lotus(b, 8, 1.035, 1.30, tw);
    b.group("outer lotus");
    lotus(b, 16, 1.31, 1.50, tw);

    b.role(Role::Figure);
    b.group("bhupura");
    b.width(2.0);
    for k in 0..3 {
        b.circle_r(o, 1.52 + k as f64 * 0.05);
    }
}

/// One ring of `n` petals.
///
/// Each petal is a true vesica lens: two equal circles whose centres sit either
/// side of the petal's axis, chosen so their intersections land exactly on the
/// inner and outer radii. Adjacent petals then share an edge, because the
/// centre offset is exactly half the angular pitch.
fn lotus(b: &mut Build, n: u32, r_in: f64, r_out: f64, twist: f64) {
    b.width(1.5);
    let delta = std::f64::consts::PI / n as f64;
    let mid = (r_in + r_out) * 0.5;
    let h = (r_out - r_in) * 0.5;
    let rm = mid / delta.cos();
    let s = rm * delta.sin();
    let rp = (s * s + h * h).sqrt();

    for k in 0..n {
        let th = twist + k as f64 * TAU / n as f64;
        b.group_next();
        // The two lens-forming circles, straddling the petal axis.
        let mut arcs = Vec::with_capacity(2);
        for sign in [-1.0f64, 1.0] {
            let a = th + sign * delta;
            let c = b.point(rm * a.cos(), rm * a.sin());
            arcs.push(b.circle_silent(c, rp));
        }
        // Their intersections are the petal's base and tip, by construction.
        let m = b.meet(arcs[0], arcs[1]);
        if m.len() < 2 {
            continue;
        }
        let (base, tip) = {
            let (a, c) = (m.upper(), m.lower());
            if b.pos(a).length() < b.pos(c).length() {
                (a, c)
            } else {
                (c, a)
            }
        };
        b.arc_between(arcs[0], base, tip);
        b.arc_between(arcs[1], tip, base);
    }
}
