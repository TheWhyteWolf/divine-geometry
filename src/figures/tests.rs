//! Golden tests for the construction engine.
//!
//! These run headless with no GPU. The point counts are frozen constants: a
//! broken dedup roughly *doubles* them, so a single number catches the whole
//! failure class instantly and unambiguously.

use glam::DVec2;

use crate::geom::build::Build;
use crate::geom::construction::{Geom, Role};
use crate::params::{Params, RINGS, SYMMETRY};

use super::hex::{fruit_centers, ngon_side, seed_frame};

fn hex_dir(k: i32) -> DVec2 {
    let a = k as f64 * std::f64::consts::TAU / 6.0;
    DVec2::new(a.cos(), a.sin())
}

fn classic() -> Params {
    Params::default()
}

#[test]
fn seed_of_life_walk_closes_on_seven_points() {
    let mut b = Build::new();
    let f = seed_frame(&mut b, &classic(), 1.0);
    // Origin plus six rim points. A seventh means the walk failed to recognise
    // it had come back around.
    assert_eq!(b.n_points(), 7, "seed frame should intern exactly 7 points");
    assert_eq!(b.n_curves(), 7, "seed frame should register exactly 7 circles");

    let o = b.pos(f.o);
    for k in 0..6 {
        let p = b.pos(f.ring1[k]);
        assert!(((p - o).length() - 1.0).abs() < 1e-12, "rim point {k} off the mother circle");
    }
    // At sixfold the N-gon side equals the radius — which is exactly why the
    // hexagonal construction is the one that looks like it was meant to be.
    assert!((f.chord - 1.0).abs() < 1e-12, "hexagonal chord should be R, was {}", f.chord);
}

/// The generalisation that makes user-defined seeds work: setting the compass
/// to the inscribed N-gon's side closes the walk for *every* order, not just 6.
#[test]
fn the_compass_walk_closes_at_every_symmetry_order() {
    for n in SYMMETRY.0..=SYMMETRY.1 {
        let p = Params { symmetry: n, ..Default::default() };
        let mut b = Build::new();
        let f = seed_frame(&mut b, &p, 1.0);

        assert_eq!(f.ring1.len(), n as usize, "N={n}: wrong rim point count");
        assert_eq!(
            b.n_points(),
            n as usize + 1,
            "N={n}: walk did not close — got {} points, expected {}",
            b.n_points(),
            n + 1
        );

        // Consecutive rim points are exactly one N-th of a turn apart.
        let step = std::f64::consts::TAU / n as f64;
        for k in 0..n as usize {
            let a = b.pos(f.ring1[k]);
            let c = b.pos(f.ring1[(k + 1) % n as usize]);
            let ang = (a.y.atan2(a.x) - c.y.atan2(c.x)).abs();
            let ang = ang.min(std::f64::consts::TAU - ang);
            assert!((ang - step).abs() < 1e-9, "N={n}: rim spacing {ang} != {step}");
            assert!(
                ((a - c).length() - ngon_side(1.0, n)).abs() < 1e-9,
                "N={n}: rim chord is not the N-gon side"
            );
        }
    }
}

/// Checks what actually reaches the screen, not just what the lattice solver
/// believed: the classical figure is nineteen drawn circles.
#[test]
fn flower_of_life_draws_exactly_nineteen_circles() {
    let c = super::build(3);
    assert_eq!(c.name, "Flower of Life");
    let circles = c
        .steps
        .iter()
        .filter(|s| s.role == Role::Figure && matches!(s.geom, Geom::Arc { .. }))
        .count();
    assert_eq!(circles, 19, "the Flower of Life is 19 circles, drew {circles}");
}

/// The growth rule is stated, not tabulated — so one more generation has to
/// produce the next classical ring on its own: 7 → 19 → 37 → 61.
///
/// Counts centres rather than interned points: growth *probes* beyond its
/// radius limit and interns what it finds there, so the registry legitimately
/// holds more points than the lattice keeps.
#[test]
fn growth_rule_reproduces_the_classical_ring_counts() {
    use super::hex::lattice;
    for (rings, expect) in [(1u32, 7usize), (2, 19), (3, 37), (4, 61)] {
        let p = Params { rings, ..Default::default() };
        let mut b = Build::new();
        let f = seed_frame(&mut b, &p, 1.0);
        let centers = lattice(&mut b, &p, &f);
        assert_eq!(centers.len(), expect, "rings={rings} should give {expect} lattice centres");
    }
}

/// The sixfold lattice is the only one that tiles the plane, so it is the only
/// one whose growth converges. Every other order multiplies each generation and
/// has to be caught by the budget rather than by luck.
#[test]
fn every_symmetry_order_stays_within_the_lattice_budget() {
    use super::hex::{lattice, MAX_CENTERS};
    for n in SYMMETRY.0..=SYMMETRY.1 {
        for rings in RINGS.0..=RINGS.1 {
            let p = Params { symmetry: n, rings, ..Default::default() };
            let mut b = Build::new();
            let f = seed_frame(&mut b, &p, 1.0);
            let centers = lattice(&mut b, &p, &f);
            assert!(
                centers.len() <= MAX_CENTERS,
                "N={n} rings={rings} grew to {} centres, past the {MAX_CENTERS} budget",
                centers.len()
            );
        }
    }
}

#[test]
fn fruit_of_life_has_exactly_thirteen_centers() {
    let mut b = Build::new();
    let (centers, _) = fruit_centers(&mut b, &classic());
    assert_eq!(centers.len(), 13);
    assert_eq!(b.n_points(), 13, "no stray points beyond the 13 centres");
}

/// The hardest derivation in the set, checked against an independent oracle.
///
/// Colliderscope hardcodes these same thirteen nodes as `hex_dir(k) * 0.5` and
/// `hex_dir(k) * 1.0` in `src/forms/circles.rs::metatron_nodes()`. Ours are
/// solved by compass; they must agree.
#[test]
fn metatron_centers_match_the_hardcoded_oracle() {
    let mut b = Build::new();
    let (centers, _) = fruit_centers(&mut b, &classic());

    let pos: Vec<DVec2> = centers.iter().map(|&c| b.pos(c)).collect();
    let outer = pos.iter().map(|p| p.length()).fold(0.0f64, f64::max);
    let norm: Vec<DVec2> = pos.iter().map(|p| *p / outer).collect();

    let mut oracle = vec![DVec2::ZERO];
    for k in 0..6 {
        oracle.push(hex_dir(k) * 0.5);
        oracle.push(hex_dir(k));
    }
    assert_eq!(oracle.len(), 13);

    for (i, p) in norm.iter().enumerate() {
        let best = oracle.iter().map(|q| (*q - *p).length()).fold(f64::INFINITY, f64::min);
        assert!(best < 1e-9, "derived centre {i} at {p:?} has no oracle match ({best:.3e})");
    }
    // And it's a bijection — no two derived centres collapsed onto one.
    for q in &oracle {
        let best = norm.iter().map(|p| (*q - *p).length()).fold(f64::INFINITY, f64::min);
        assert!(best < 1e-9, "oracle node {q:?} was never derived");
    }
}

#[test]
fn metatron_draws_all_seventy_eight_chords() {
    let c = super::build(5);
    assert_eq!(c.name, "Metatron's Cube");
    let chords = c
        .steps
        .iter()
        .filter(|s| s.role == Role::Figure && matches!(s.geom, Geom::Seg { .. }))
        .count();
    // C(13,2) = 78. Losing 27 is the signature of conflating the infinite-line
    // dedup with the drawn-segment dedup: three diameters each carry five
    // collinear centres, so ten chords per diameter share one line.
    assert_eq!(chords, 78, "expected all C(13,2) chords");
}

#[test]
fn vesica_lens_meets_at_root_three_over_two() {
    let mut b = Build::new();
    super::vesica::vesica(&mut b, &classic());
    assert_eq!(b.n_points(), 4);
}

/// φ is solved by compass, not typed in.
#[test]
fn golden_spiral_derives_phi_from_the_swing() {
    let mut b = Build::new();
    super::phi::golden_spiral(&mut b, &classic());
    let phi = (1.0 + 5.0f64.sqrt()) / 2.0;
    let hit = (0..b.n_points()).any(|i| {
        let p = b.pos(i as u32);
        (p.x - phi).abs() < 1e-12 && p.y.abs() < 1e-12
    });
    assert!(hit, "the compass swing did not land on phi = {phi}");
}

/// Every figure must build, normalize into the unit box, and bake a sane,
/// monotone timeline — at the classical seed and at a spread of odd ones.
#[test]
fn every_figure_builds_and_bakes_a_valid_timeline() {
    let mut seeds: Vec<Params> = vec![classic()];
    seeds.extend((1u64..24).map(Params::from_seed));

    for (i, def) in super::FIGURES.iter().enumerate() {
        for p in &seeds {
            let c = super::build_with(i, p);
            let who = format!("{} @ {}", def.name, p.summary());
            assert_eq!(c.name, def.name);
            assert!(!c.steps.is_empty(), "{who}: produced no steps");

            for s in &c.steps {
                assert!(s.t0 >= 0.0 && s.t1 <= 1.0 + 1e-5, "{who}: window out of range");
                assert!(s.t1 > s.t0, "{who}: zero-length step window");
                assert!(s.arc_ref > 0.0, "{who}: degenerate step geometry");
                assert!(s.arc_ref.is_finite(), "{who}: non-finite geometry");
            }
            let last = c.steps.iter().map(|s| s.t1).fold(0.0f32, f32::max);
            assert!((last - 1.0).abs() < 1e-5, "{who}: timeline ends at {last}");

            for s in &c.steps {
                let far = match s.geom {
                    Geom::Arc { c, r, .. } => c.length() + r,
                    Geom::Seg { a, b } => a.length().max(b.length()),
                };
                assert!(far <= 1.0 + 1e-4, "{who}: geometry at {far} escapes the unit box");
            }

            let secs = c.draw_seconds();
            assert!((5.0..=33.0).contains(&secs), "{who}: {secs}s out of range");
        }
    }
}

/// A seed must never be able to explode the step count — the renderer uploads
/// this geometry, and an unbounded figure would stall the frame.
#[test]
fn no_seed_produces_a_runaway_figure() {
    for i in 0..super::FIGURES.len() {
        for s in 1u64..80 {
            let c = super::build_with(i, &Params::from_seed(s));
            assert!(
                c.steps.len() < 6000,
                "{} at seed {s} produced {} steps",
                c.name,
                c.steps.len()
            );
        }
    }
}
