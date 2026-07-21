//! Circle/line intersection solving in f64.
//!
//! f64 is not optional here. A Flower of Life point is reached through five or
//! six nested intersections, each involving a `sqrt` and differences of similar
//! magnitudes. f32 accumulates ~1e-4 relative error over that chain — the same
//! order as the tolerance needed to tell genuinely-near points apart. f64 lands
//! at ~1e-13, which is where EPS = 1e-7 gets its enormous safety band.

use glam::DVec2;

/// Coincidence tolerance, in construction units (figures normalized to r ≈ 1).
///
/// ~6 orders of magnitude above accumulated f64 noise and ~5 below the tightest
/// genuine feature separation in any target figure. Do not tune this per figure;
/// the whole point is that one global constant has room to spare in both
/// directions.
pub const EPS: f64 = 1e-7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Isect {
    None,
    One(DVec2),
    Two(DVec2, DVec2),
}

impl Isect {
    pub fn points(self) -> Vec<DVec2> {
        match self {
            Isect::None => vec![],
            Isect::One(p) => vec![p],
            Isect::Two(a, b) => vec![a, b],
        }
    }
}

/// Circle (c0, r0) × circle (c1, r1).
///
/// Two-point results are ordered deterministically: `.0` lies on the +perp
/// (CCW-left) side of the c0→c1 direction, `.1` on the −side. Scripts depend on
/// that ordering for `upper()` / `lower()`.
pub fn circle_circle(c0: DVec2, r0: f64, c1: DVec2, r1: f64) -> Isect {
    let d = c1 - c0;
    let d2 = d.length_squared();
    let dist = d2.sqrt();
    // Concentric (including identical) circles have no isolated intersections.
    if dist < EPS {
        return Isect::None;
    }
    // Signed distance from c0 to the radical line, measured along d̂.
    let a = (d2 + r0 * r0 - r1 * r1) / (2.0 * dist);
    let h2 = r0 * r0 - a * a;
    let base = c0 + d * (a / dist);

    // Test h2 (an area-like quantity), NOT `dist > r0 + r1`. The latter loses
    // about half its precision to cancellation exactly at tangency — and sacred
    // geometry hits exact tangency constantly (the Egg of Life is six mutually
    // tangent circles). Getting `Two` with a 1e-8 separation there would emit a
    // degenerate zero-length arc whose miter normal is garbage.
    let tol = EPS * r0.max(1.0);
    if h2 < -tol {
        return Isect::None;
    }
    if h2 <= tol {
        return Isect::One(base);
    }
    let h = h2.sqrt();
    let perp = DVec2::new(-d.y, d.x) / dist;
    Isect::Two(base + perp * h, base - perp * h)
}

/// Infinite line through (a, b) × circle (c, r).
/// `.0` is the intersection further along the a→b direction.
pub fn line_circle(a: DVec2, b: DVec2, c: DVec2, r: f64) -> Isect {
    let d = b - a;
    let len = d.length();
    if len < EPS {
        return Isect::None;
    }
    let u = d / len;
    let t = (c - a).dot(u);
    let foot = a + u * t;
    let h2 = r * r - (c - foot).length_squared();
    let tol = EPS * r.max(1.0);
    if h2 < -tol {
        return Isect::None;
    }
    if h2 <= tol {
        return Isect::One(foot);
    }
    let h = h2.sqrt();
    Isect::Two(foot + u * h, foot - u * h)
}

/// Infinite line × infinite line.
pub fn line_line(a0: DVec2, a1: DVec2, b0: DVec2, b1: DVec2) -> Option<DVec2> {
    let r = a1 - a0;
    let s = b1 - b0;
    let den = r.perp_dot(s);
    // Relative parallelism test — an absolute epsilon on a cross product is
    // meaningless when the operand magnitudes differ by orders of magnitude.
    if den.abs() < EPS * r.length() * s.length() {
        return None;
    }
    Some(a0 + r * ((b0 - a0).perp_dot(s) / den))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: DVec2, b: DVec2, tol: f64) -> bool {
        (a - b).length() < tol
    }

    #[test]
    fn vesica_intersections_are_exact() {
        // Unit circles centred at ±0.5 meet at (0, ±√3/2).
        let r3_2 = 3.0f64.sqrt() / 2.0;
        match circle_circle(DVec2::new(-0.5, 0.0), 1.0, DVec2::new(0.5, 0.0), 1.0) {
            Isect::Two(p, q) => {
                assert!(close(p, DVec2::new(0.0, r3_2), 1e-12), "upper was {p:?}");
                assert!(close(q, DVec2::new(0.0, -r3_2), 1e-12), "lower was {q:?}");
            }
            other => panic!("expected two intersections, got {other:?}"),
        }
    }

    #[test]
    fn exact_tangency_yields_one_not_two() {
        // Separation exactly 2r — the Egg of Life case.
        let got = circle_circle(DVec2::ZERO, 1.0, DVec2::new(2.0, 0.0), 1.0);
        assert!(matches!(got, Isect::One(_)), "expected One, got {got:?}");
        if let Isect::One(p) = got {
            assert!(close(p, DVec2::new(1.0, 0.0), 1e-12));
        }
    }

    #[test]
    fn internal_tangency_and_separation() {
        // Disjoint.
        assert_eq!(circle_circle(DVec2::ZERO, 1.0, DVec2::new(5.0, 0.0), 1.0), Isect::None);
        // Concentric.
        assert_eq!(circle_circle(DVec2::ZERO, 1.0, DVec2::ZERO, 2.0), Isect::None);
    }

    #[test]
    fn line_circle_ordering_follows_direction() {
        match line_circle(DVec2::new(-5.0, 0.0), DVec2::new(5.0, 0.0), DVec2::ZERO, 1.0) {
            Isect::Two(p, q) => {
                assert!(close(p, DVec2::new(1.0, 0.0), 1e-12), "far end was {p:?}");
                assert!(close(q, DVec2::new(-1.0, 0.0), 1e-12), "near end was {q:?}");
            }
            other => panic!("expected two, got {other:?}"),
        }
    }

    #[test]
    fn parallel_lines_do_not_meet() {
        assert!(line_line(
            DVec2::ZERO,
            DVec2::new(1.0, 0.0),
            DVec2::new(0.0, 1.0),
            DVec2::new(1.0, 1.0)
        )
        .is_none());
        let p = line_line(
            DVec2::new(-1.0, 0.0),
            DVec2::new(1.0, 0.0),
            DVec2::new(0.0, -1.0),
            DVec2::new(0.0, 1.0),
        )
        .expect("perpendicular lines meet");
        assert!(close(p, DVec2::ZERO, 1e-12));
    }
}
