//! Endlessly growing constructions, with a camera that follows the growth.
//!
//! Two systems, growing in opposite directions:
//!
//! * **Lattice** — the Flower of Life rule applied forever. Each generation
//!   solves the intersections of the last and keeps them as new centres, so the
//!   mesh spreads outward and the camera pulls back through it.
//! * **Gasket** — an Apollonian packing. Every gap between three mutually
//!   tangent circles gets an inscribed fourth, forever, so the detail grows
//!   *inward* and the camera descends into it. The new circle is solved by
//!   Descartes' theorem, which is the tangency analogue of a compass step.
//!
//! Both share the same treatment: geometry is generated in construction space
//! and never renormalized, the camera eases toward whatever the growth frontier
//! currently is, and anything that has become too small or has left the frame is
//! culled from the vertex buffers rather than accumulating forever.

use glam::{DVec2, Vec2};

use crate::color::Palette;
use crate::geom::build::Build;
use crate::geom::construction::{Geom, Role, Step};
use crate::params::Params;
use crate::tess::{StepGpu, Tess, View};

use crate::figures::hex::{grow, seed_frame};

/// Seconds a newly generated step takes to draw itself in.
const DRAW_SECS: f32 = 1.6;
/// Generations wait this long between batches.
const GROW_EVERY: f32 = 2.2;
/// Camera easing time constant, in seconds. Long enough to read as drift.
const EASE_TAU: f32 = 2.6;
/// Cull anything whose on-screen extent falls below this many pixels.
const MIN_PX: f32 = 0.7;
/// Hard ceiling on retained steps; oldest are dropped first.
const MAX_STEPS: usize = 6000;
/// Live branches the gasket keeps. It triples its queue every generation, so
/// without a cap this is 3ⁿ circles.
const MAX_BRANCHES: usize = 140;
/// New circles per lattice generation.
const MAX_LATTICE_BATCH: usize = 90;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Lattice,
    Gasket,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Lattice => "Infinite Lattice",
            Kind::Gasket => "Infinite Gasket",
        }
    }
}

/// A circle in the Apollonian packing, carried as curvature rather than radius
/// so Descartes' theorem stays linear. Curvature is negative for the enclosing
/// circle, which is what makes it "contain" rather than "touch from outside".
#[derive(Clone, Copy, Debug)]
struct Gc {
    z: DVec2,
    k: f64,
}

impl Gc {
    fn r(&self) -> f64 {
        1.0 / self.k.abs()
    }
}

/// Solve the fourth circle tangent to three mutually tangent circles.
///
/// Descartes: k₄ = k₁+k₂+k₃ ± 2√(k₁k₂+k₂k₃+k₃k₁). Given the *other* solution
/// `prev` from the same quadruple, the two roots sum to 2(k₁+k₂+k₃), so the new
/// one follows without a square root — and the same identity holds for the
/// curvature-weighted centres, which is what keeps this exact under recursion.
fn descartes(a: Gc, b: Gc, c: Gc, prev: Gc) -> Gc {
    let k = 2.0 * (a.k + b.k + c.k) - prev.k;
    let zk = (a.z * a.k + b.z * b.k + c.z * c.k) * 2.0 - prev.z * prev.k;
    Gc { z: zk / k, k }
}

pub struct Infinite {
    pub kind: Kind,
    pub steps: Vec<Step>,
    /// Wall-clock second each step began drawing.
    born: Vec<f32>,

    // --- lattice state ---
    b: Build,
    centers: Vec<crate::geom::registry::PointId>,
    chord: f64,

    // --- gasket state ---
    /// Triples still to be subdivided, with the fourth circle of their quadruple.
    queue: Vec<(Gc, Gc, Gc, Gc)>,

    /// Growth frontier the camera is chasing: (centre, radius).
    pub focus: Vec2,
    /// Where the camera is actually looking — eases toward `focus`.
    eye: Vec2,
    pub frontier: f32,
    next_grow: f32,
    pub generation: u32,
    /// The scale the visible set was last culled and tessellated at.
    tess_scale: f32,
    /// Indices into `steps` that are currently in the vertex buffers.
    visible: Vec<usize>,
}

impl Infinite {
    pub fn new(kind: Kind, p: &Params, now: f32) -> Self {
        let mut s = Self {
            kind,
            steps: Vec::new(),
            born: Vec::new(),
            b: Build::new(),
            centers: Vec::new(),
            chord: 1.0,
            queue: Vec::new(),
            focus: Vec2::ZERO,
            eye: Vec2::ZERO,
            frontier: 1.0,
            next_grow: now + GROW_EVERY,
            generation: 0,
            tess_scale: 0.0,
            visible: Vec::new(),
        };
        s.seed(p, now);
        s
    }

    fn seed(&mut self, p: &Params, now: f32) {
        match self.kind {
            Kind::Lattice => {
                self.b.role(Role::Figure);
                let f = seed_frame(&mut self.b, p, 1.0);
                self.chord = f.chord;
                self.centers = std::iter::once(f.o).chain(f.ring1.iter().copied()).collect();
                self.take(now);
                self.frontier = (self.chord * 2.0) as f32;
            }
            Kind::Gasket => {
                // Three equal circles inside a unit circle, mutually tangent and
                // each tangent to the boundary: r = 2√3 − 3.
                let r = 2.0 * 3.0f64.sqrt() - 3.0;
                let d = 1.0 - r;
                let outer = Gc { z: DVec2::ZERO, k: -1.0 };
                let inner: Vec<Gc> = (0..3)
                    .map(|i| {
                        let a = i as f64 * std::f64::consts::TAU / 3.0
                            + p.twist_rad();
                        Gc { z: DVec2::new(a.cos(), a.sin()) * d, k: 1.0 / r }
                    })
                    .collect();

                self.push_circle(outer, now, Role::Figure);
                for c in &inner {
                    self.push_circle(*c, now, Role::Figure);
                }
                // Each face of the starting quadruple seeds a branch.
                self.queue.push((outer, inner[0], inner[1], inner[2]));
                self.queue.push((outer, inner[0], inner[2], inner[1]));
                self.queue.push((outer, inner[1], inner[2], inner[0]));
                self.queue.push((inner[0], inner[1], inner[2], outer));
                self.frontier = 1.0;
                self.focus = Vec2::ZERO;
            }
        }
    }

    /// Pull whatever the builder has accumulated into our own step list.
    fn take(&mut self, now: f32) {
        let base = self.steps.len();
        let fresh = self.b.drain_steps(base);
        self.born.extend(std::iter::repeat_n(now, fresh.len()));
        self.steps.extend(fresh);
    }

    fn push_circle(&mut self, c: Gc, now: f32, role: Role) {
        let r = c.r() as f32;
        self.steps.push(Step {
            geom: Geom::Arc {
                c: Vec2::new(c.z.x as f32, c.z.y as f32),
                r,
                a0: 0.0,
                a1: std::f32::consts::TAU,
            },
            role,
            group: self.generation as u16,
            width: 2.0,
            t0: 0.0,
            t1: 1.0,
            arc_ref: 1.0,
            seed: (self.steps.len() as f32 * 0.618_034).fract() * 16.0,
        });
        self.born.push(now);
    }

    /// Advance. Returns true if the geometry changed and needs re-uploading.
    pub fn tick(&mut self, now: f32, view: &mut View, size: (u32, u32), dt: f32) -> bool {
        let mut changed = false;
        if now >= self.next_grow {
            self.next_grow = now + GROW_EVERY;
            self.generation += 1;
            match self.kind {
                Kind::Lattice => self.grow_lattice(now),
                Kind::Gasket => self.grow_gasket(now),
            }
            changed = true;
        }

        // Ease the camera toward the current frontier. Exponential easing keeps
        // it smooth through the discontinuous jump a new generation causes.
        //
        // Both the look-at point and the zoom are eased in *construction* space,
        // and the pixel centre is then derived exactly from them. Easing
        // `view.center` in pixels instead looks equivalent and is not: the pixel
        // offset of a fixed point is proportional to scale, so while the zoom is
        // changing by 2× per generation the eased centre chases a target that is
        // itself moving, and the look-at point slides off the geometry. That is
        // what produced frames with a thousand circles present and none visible.
        let target = View::fit_radius(size.0, size.1, self.frontier.max(1e-6));
        let a = 1.0 - (-dt / EASE_TAU).exp();
        self.eye += (self.focus - self.eye) * a;
        view.scale += (target.scale - view.scale) * a;
        view.center = Vec2::new(size.0 as f32 * 0.5, size.1 as f32 * 0.5)
            - Vec2::new(self.eye.x, -self.eye.y) * view.scale;

        // Re-cull and re-tessellate once the camera has drifted far enough that
        // the visible set has meaningfully changed.
        if changed
            || self.tess_scale <= 0.0
            || view.scale > self.tess_scale * 1.35
            || view.scale < self.tess_scale * 0.74
        {
            changed = true;
        }
        changed
    }

    fn grow_lattice(&mut self, now: f32) {
        // Grow one ring further out each generation.
        let limit = self.chord * (self.generation as f64 + 2.0);
        let mut fresh = grow(&mut self.b, &self.centers, self.chord, limit);
        // Keep the per-generation batch bounded so a frame never stalls.
        fresh.truncate(MAX_LATTICE_BATCH);
        if fresh.is_empty() {
            return;
        }
        self.b.role(Role::Figure);
        for &c in &fresh {
            self.b.circle_r(c, self.chord);
        }
        self.centers.extend(fresh);
        self.take(now);

        let far = self
            .centers
            .iter()
            .map(|&c| self.b.point_pos(c).length())
            .fold(0.0f64, f64::max);
        self.frontier = (far + self.chord) as f32;
        self.focus = Vec2::ZERO;
    }

    fn grow_gasket(&mut self, now: f32) {
        let focus = DVec2::new(self.focus.x as f64, self.focus.y as f64);
        let batch = std::mem::take(&mut self.queue);
        let mut next = Vec::new();
        // The circle we will descend toward: nearest the current focus, and
        // meaningfully smaller than where we are now.
        let mut best: Option<Gc> = None;
        let want = self.frontier as f64 * 0.30;

        for (a, b, c, prev) in batch {
            let d = descartes(a, b, c, prev);
            // Curvature grows without bound; stop a branch once its circle is
            // far below anything the camera will reach this run.
            if !d.k.is_finite() || d.k <= 0.0 || d.r() < 1e-9 {
                continue;
            }
            self.push_circle(d, now, Role::Figure);

            // Only ever descend into something already on screen. Without this
            // the next target can sit a hundred screen-widths away at the
            // current zoom, and the camera spends a whole generation flying
            // through empty space to reach it.
            let reachable = (d.z - focus).length() + d.r() <= self.frontier as f64;
            if d.r() <= want && reachable {
                let better = match best {
                    None => true,
                    Some(cur) => (d.z - focus).length() < (cur.z - focus).length(),
                };
                if better {
                    best = Some(d);
                }
            }
            // Each new circle forms three fresh quadruples.
            next.push((a, b, d, c));
            next.push((a, c, d, b));
            next.push((b, c, d, a));
        }

        // An Apollonian gasket triples its queue every generation, so without a
        // cap this is 3ⁿ circles. Keep the branches *nearest the focus* rather
        // than the globally smallest: the camera is descending into one region,
        // and detail generated on the far side of the gasket is detail we will
        // never see. Sorting by curvature instead is what left the view empty —
        // growth kept happening somewhere the camera wasn't.
        next.sort_by(|x, y| {
            let dx = (x.2.z - focus).length() - x.2.r();
            let dy = (y.2.z - focus).length() - y.2.r();
            dx.partial_cmp(&dy).unwrap_or(std::cmp::Ordering::Equal)
        });
        next.truncate(MAX_BRANCHES);
        self.queue = next;

        if let Some(d) = best {
            // Descend, but never faster than roughly half a decade per
            // generation, or the easing camera can't keep up with its own target.
            self.focus = Vec2::new(d.z.x as f32, d.z.y as f32);
            // Cap the descent rate so the eased camera can keep up with its own
            // target; anything faster and the zoom is always a generation behind.
            let target = (d.r() * 11.0) as f32;
            self.frontier = target.max(self.frontier * 0.62);
        }
    }

    /// Rebuild the vertex buffers for what is currently worth drawing.
    pub fn retessellate(&mut self, tess: &mut Tess, view: &View, size: (u32, u32)) {
        self.tess_scale = view.scale;
        tess.clear();
        tess.detail_scale = view.scale.max(1.0);
        self.visible.clear();

        let (w, h) = (size.0 as f32, size.1 as f32);
        // Viewport as a disc in construction space — circle visibility is a
        // circle-vs-circle question, so posing it that way keeps it exact.
        let vr = 0.5 * (w * w + h * h).sqrt() * 1.15 / view.scale;
        let vc = Vec2::new(
            (w * 0.5 - view.center.x) / view.scale,
            -(h * 0.5 - view.center.y) / view.scale,
        );

        // (index, optional angular window) for everything worth drawing.
        let mut keep: Vec<(usize, Option<(f32, f32)>)> = Vec::new();
        for (i, s) in self.steps.iter().enumerate() {
            match s.geom {
                Geom::Arc { c, r, .. } => {
                    if r * view.scale < MIN_PX {
                        continue;
                    }
                    let d = (vc - c).length();
                    // Disjoint from the viewport, or the viewport sits entirely
                    // inside it — either way the stroke is off screen.
                    if d > r + vr || d + vr < r {
                        continue;
                    }
                    // Under deep zoom a circle can be thousands of screens
                    // across. Tessellating the whole thing would blow past the
                    // segment cap and turn the visible sliver into a polygon, so
                    // tessellate only the span that can actually be seen.
                    let window = if r > vr * 2.0 && d > 1e-9 {
                        let mid = vc - c;
                        let ca = mid.y.atan2(mid.x);
                        let half = (vr / d).min(1.0).asin() * 1.2 + 0.02;
                        Some((ca - half, ca + half))
                    } else {
                        None
                    };
                    keep.push((i, window));
                }
                Geom::Seg { a, b } => {
                    let mid = (a + b) * 0.5;
                    if (vc - mid).length() > vr + a.distance(b) * 0.5 {
                        continue;
                    }
                    keep.push((i, None));
                }
            }
        }

        // Retire ancient steps once they can never come back.
        if self.steps.len() > MAX_STEPS {
            let drop = self.steps.len() - MAX_STEPS;
            self.steps.drain(0..drop);
            self.born.drain(0..drop);
            keep.retain(|&(i, _)| i >= drop);
            for (i, _) in keep.iter_mut() {
                *i -= drop;
            }
        }

        for (slot, &(i, window)) in keep.iter().enumerate() {
            let s = &self.steps[i];
            match s.geom {
                Geom::Arc { c, r, a0, a1 } => {
                    let (s0, s1) = window.unwrap_or((a0, a1));
                    tess.arc(c, r, s0, s1, s.width, slot as u32)
                }
                Geom::Seg { a, b } => tess.seg(a, b, s.width, slot as u32),
            }
            self.visible.push(i);
        }
    }

    /// Per-frame state for whatever is currently visible.
    pub fn step_states(&self, now: f32, pal: &Palette, out: &mut Vec<StepGpu>) {
        out.clear();
        out.reserve(self.visible.len());
        for &i in &self.visible {
            let age = now - self.born[i];
            let prog = (age / DRAW_SECS).clamp(0.0, 1.0);
            // Same endpoint overshoot as the finite path: the reveal is exactly
            // 0.5 where u == head, so landing on 0 or 1 leaves a half-lit notch.
            let head = if prog <= 0.0 {
                -0.05
            } else if prog >= 1.0 {
                1.05
            } else {
                prog
            };
            let pen = if prog > 0.0 && prog < 1.0 { 1.0 } else { 0.0 };
            // Everything settles to the same level — in an endless figure there
            // is no scaffold to recede, because nothing is ever finished.
            let glow = 0.30 + 0.34 * prog;
            let st = &self.steps[i];
            // Gradient position: the step's age in the retained window, so a
            // gradient sweeps oldest→newest as the system grows.
            let t = i as f32 / self.steps.len().max(1) as f32;
            out.push(StepGpu {
                head,
                glow,
                pen,
                seed: st.seed,
                color: pal.stroke(t, (st.seed / 16.0).fract()),
                _pad: 0.0,
            });
        }
    }

    pub fn visible_count(&self) -> usize {
        self.visible.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Descartes' theorem must produce circles that are actually tangent to all
    /// three parents — that tangency is the entire construction.
    #[test]
    fn descartes_solves_a_tangent_circle() {
        let r = 2.0 * 3.0f64.sqrt() - 3.0;
        let d = 1.0 - r;
        let outer = Gc { z: DVec2::ZERO, k: -1.0 };
        let inner: Vec<Gc> = (0..3)
            .map(|i| {
                let a = i as f64 * std::f64::consts::TAU / 3.0;
                Gc { z: DVec2::new(a.cos(), a.sin()) * d, k: 1.0 / r }
            })
            .collect();

        let got = descartes(inner[0], inner[1], inner[2], outer);
        assert!(got.k > 0.0, "inner Soddy circle should have positive curvature");

        // Externally tangent to each of the three: centre distance = sum of radii.
        for c in &inner {
            let dist = (got.z - c.z).length();
            assert!(
                (dist - (got.r() + c.r())).abs() < 1e-9,
                "not tangent: d={dist}, r1+r2={}",
                got.r() + c.r()
            );
        }
        // And it sits inside the unit circle.
        assert!(got.z.length() + got.r() <= 1.0 + 1e-9);
    }

    #[test]
    fn gasket_growth_stays_bounded_and_shrinking() {
        let mut inf = Infinite::new(Kind::Gasket, &Params::default(), 0.0);
        let mut t = 0.0;
        for _ in 0..14 {
            t += GROW_EVERY;
            inf.generation += 1;
            inf.grow_gasket(t);
        }
        assert!(inf.queue.len() <= MAX_BRANCHES, "queue ran away to {}", inf.queue.len());
        assert!(inf.steps.len() > 40, "gasket barely grew: {}", inf.steps.len());
        // Every circle must lie inside the unit disc it was packed into.
        for s in &inf.steps {
            if let Geom::Arc { c, r, .. } = s.geom {
                assert!(
                    c.length() + r <= 1.0 + 1e-4,
                    "circle escaped the packing: |c|={} r={r}",
                    c.length()
                );
            }
        }
        // The frontier must be descending — that is what the camera follows.
        assert!(inf.frontier < 0.5, "gasket frontier never shrank: {}", inf.frontier);
    }

    #[test]
    fn lattice_grows_outward_forever() {
        let mut inf = Infinite::new(Kind::Lattice, &Params::default(), 0.0);
        let start = inf.frontier;
        let mut t = 0.0;
        for _ in 0..6 {
            t += GROW_EVERY;
            inf.generation += 1;
            inf.grow_lattice(t);
        }
        assert!(inf.frontier > start * 1.5, "lattice frontier did not expand");
        assert!(inf.steps.len() > 20, "lattice barely grew");
    }
}
