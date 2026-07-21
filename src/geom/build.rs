//! The construction scripting API.
//!
//! Figures are data: a script is a plain function that drives this builder. No
//! string names — every op returns ids the script binds to Rust `let`s, so it's
//! compile-time checked with no lookup-typo class of bugs.

use std::collections::HashSet;
use std::f64::consts::TAU;

use glam::DVec2;

use super::construction::{bake_timeline, Construction, Geom, Node, Role, Step, REF_SCALE};
use super::isect::{circle_circle, line_circle, line_line, Isect, EPS};
use super::registry::{Curve, CurveId, CurveRegistry, PointId, PointKind, PointRegistry};

/// The result of intersecting two curves.
#[derive(Clone, Debug)]
pub struct Meet {
    ids: Vec<PointId>,
    pos: Vec<DVec2>,
}

impl Meet {
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The +perp (CCW-left) intersection of the pair.
    pub fn upper(&self) -> PointId {
        *self.ids.first().expect("meet produced no intersections")
    }

    /// The −perp (CW-right) intersection of the pair.
    pub fn lower(&self) -> PointId {
        *self.ids.last().expect("meet produced no intersections")
    }

    /// The compass-walk primitive: "step to the intersection that isn't where I
    /// already am". Works by `PointId` identity, which is meaningful only
    /// because interning is exact — the dedup and this method justify each other.
    pub fn other_than(&self, p: PointId) -> PointId {
        *self
            .ids
            .iter()
            .find(|&&id| id != p)
            .unwrap_or_else(|| panic!("no intersection distinct from {p}"))
    }

    pub fn near(&self, hint: DVec2) -> PointId {
        self.pick(|d| d, hint)
    }

    pub fn far_from(&self, hint: DVec2) -> PointId {
        self.pick(|d| -d, hint)
    }

    fn pick(&self, key: impl Fn(f64) -> f64, hint: DVec2) -> PointId {
        let mut best = (f64::INFINITY, self.ids[0]);
        for (i, &id) in self.ids.iter().enumerate() {
            let k = key((self.pos[i] - hint).length());
            if k < best.0 {
                best = (k, id);
            }
        }
        best.1
    }

    pub fn all(&self) -> &[PointId] {
        &self.ids
    }
}

/// A step in construction (f64) space, before finish-time normalization.
struct RawStep {
    geom: RawGeom,
    role: Role,
    group: u16,
    width: f32,
}

enum RawGeom {
    Arc { c: DVec2, r: f64, a0: f64, a1: f64 },
    Seg { a: DVec2, b: DVec2 },
}

pub struct Build {
    pts: PointRegistry,
    curves: CurveRegistry,
    steps: Vec<RawStep>,
    /// Drawn-segment dedup, by sorted endpoint ids. Deliberately separate from
    /// the curve registry's infinite-object dedup: Metatron has three diameters
    /// each carrying five collinear centres, giving ten chords per diameter that
    /// share one infinite line but are ten distinct drawn segments. Conflating
    /// the two either loses 27 of the 78 chords or draws them twice.
    seg_seen: HashSet<(PointId, PointId)>,
    role: Role,
    group: u16,
    group_names: Vec<(u16, &'static str)>,
    overlap: f32,
    pace: f32,
    width: f32,
    /// Where the pen last was, so a new compass arc starts from that angle.
    last_touch: Option<DVec2>,
}

impl Build {
    pub fn new() -> Self {
        Self {
            pts: PointRegistry::default(),
            curves: CurveRegistry::default(),
            steps: Vec::new(),
            seg_seen: HashSet::new(),
            role: Role::Scaffold,
            group: 0,
            group_names: Vec::new(),
            overlap: 0.25,
            pace: 1.0,
            width: 2.2,
            last_touch: None,
        }
    }

    // ------------------------------------------------------------- modes

    pub fn role(&mut self, r: Role) {
        self.role = r;
    }

    /// Start a new, named timing group.
    pub fn group(&mut self, name: &'static str) {
        self.group = self.group.saturating_add(1);
        self.group_names.push((self.group, name));
    }

    /// Start a new anonymous timing group.
    pub fn group_next(&mut self) {
        self.group = self.group.saturating_add(1);
    }

    /// How much consecutive groups interleave, 0..0.8.
    pub fn overlap(&mut self, o: f32) {
        self.overlap = o.clamp(0.0, 0.8);
    }

    /// Per-figure duration multiplier.
    pub fn pace(&mut self, p: f32) {
        self.pace = p.max(0.1);
    }

    pub fn width(&mut self, w: f32) {
        self.width = w;
    }

    // ------------------------------------------------------------- points

    pub fn point(&mut self, x: f64, y: f64) -> PointId {
        let (id, _) = self.pts.intern(DVec2::new(x, y), PointKind::Seed, self.group);
        id
    }

    #[inline]
    pub fn pos(&self, id: PointId) -> DVec2 {
        self.pts.pos(id)
    }

    pub fn n_points(&self) -> usize {
        self.pts.len()
    }

    pub fn n_curves(&self) -> usize {
        self.curves.len()
    }

    // ------------------------------------------------------------- compass

    /// Set the compass to span two existing points and swing a full circle.
    pub fn circle(&mut self, center: PointId, through: PointId) -> CurveId {
        let c = self.pos(center);
        let r = (self.pos(through) - c).length();
        self.circle_at(c, r, Some(self.pos(through)))
    }

    /// A circle of an explicit radius — for seeding a figure, where no second
    /// point exists yet.
    pub fn circle_r(&mut self, center: PointId, r: f64) -> CurveId {
        let c = self.pos(center);
        self.circle_at(c, r, None)
    }

    /// Register a circle without drawing it (a construction aid we don't want
    /// on screen).
    pub fn circle_silent(&mut self, center: PointId, r: f64) -> CurveId {
        let (id, _) = self.curves.intern(Curve::Circle { c: self.pos(center), r });
        id
    }

    fn circle_at(&mut self, c: DVec2, r: f64, through: Option<DVec2>) -> CurveId {
        let (id, _) = self.curves.intern(Curve::Circle { c, r });
        // The compass visibly starts where the hand already was.
        let anchor = self.last_touch.or(through);
        let a0 = anchor
            .filter(|p| (*p - c).length() > EPS)
            .map(|p| {
                let d = p - c;
                d.y.atan2(d.x)
            })
            .unwrap_or(0.0);
        self.push(RawGeom::Arc { c, r, a0, a1: a0 + TAU });
        self.last_touch = Some(c + DVec2::new(a0.cos(), a0.sin()) * r);
        id
    }

    // --------------------------------------------------------- straightedge

    /// A drawn segment between two points. A self-loop is silently ignored —
    /// low symmetry orders can collapse a polygon to a single vertex, and a
    /// zero-length segment has no normal and no arc length.
    pub fn line(&mut self, a: PointId, b: PointId) -> CurveId {
        if a == b {
            return u32::MAX;
        }
        let key = (a.min(b), a.max(b));
        let (pa, pb) = (self.pos(a), self.pos(b));
        let (id, _) = self.curves.intern(Curve::Line { a: pa, b: pb });
        if self.seg_seen.insert(key) {
            self.push(RawGeom::Seg { a: pa, b: pb });
            self.last_touch = Some(pb);
        }
        id
    }

    /// The line through a and b, drawn over the parameter range [t0, t1] where
    /// t = 0 is `a` and t = 1 is `b`.
    pub fn line_ext(&mut self, a: PointId, b: PointId, t0: f64, t1: f64) -> CurveId {
        let (pa, pb) = (self.pos(a), self.pos(b));
        let d = pb - pa;
        let (id, _) = self.curves.intern(Curve::Line { a: pa, b: pb });
        self.push(RawGeom::Seg { a: pa + d * t0, b: pa + d * t1 });
        self.last_touch = Some(pa + d * t1);
        id
    }

    /// Register the infinite line through two points without drawing it.
    pub fn line_silent(&mut self, a: PointId, b: PointId) -> CurveId {
        let (id, _) = self.curves.intern(Curve::Line { a: self.pos(a), b: self.pos(b) });
        id
    }

    // ------------------------------------------------------------- solving

    /// Intersect two registered curves, interning every result.
    pub fn meet(&mut self, c0: CurveId, c1: CurveId) -> Meet {
        // `line()` on a self-loop returns u32::MAX rather than registering a
        // degenerate curve; catching it here turns a silent out-of-bounds panic
        // into a message that names the actual mistake.
        debug_assert!(
            c0 != u32::MAX && c1 != u32::MAX,
            "meet() called with the degenerate curve sentinel — a script drew a self-loop line"
        );
        let (a, b) = (self.curves.get(c0), self.curves.get(c1));
        let (isect, kind) = match (a, b) {
            (Curve::Circle { c: ca, r: ra }, Curve::Circle { c: cb, r: rb }) => {
                (circle_circle(ca, ra, cb, rb), PointKind::CircleCircle)
            }
            (Curve::Line { a: la, b: lb }, Curve::Circle { c, r })
            | (Curve::Circle { c, r }, Curve::Line { a: la, b: lb }) => {
                (line_circle(la, lb, c, r), PointKind::LineCircle)
            }
            (Curve::Line { a: a0, b: a1 }, Curve::Line { a: b0, b: b1 }) => (
                line_line(a0, a1, b0, b1).map_or(Isect::None, Isect::One),
                PointKind::LineLine,
            ),
        };
        let pos = isect.points();
        let ids = pos
            .iter()
            .map(|&p| self.pts.intern(p, kind, self.group).0)
            .collect();
        Meet { ids, pos }
    }

    // ------------------------------------------------------------- tracing

    /// Re-draw a registered curve in the current role — how a scaffold circle
    /// becomes part of the bright figure.
    pub fn trace(&mut self, c: CurveId) {
        match self.curves.get(c) {
            Curve::Circle { c, r } => {
                let a0 = self.last_touch.map_or(0.0, |p| {
                    let d = p - c;
                    if d.length() > EPS {
                        d.y.atan2(d.x)
                    } else {
                        0.0
                    }
                });
                self.push(RawGeom::Arc { c, r, a0, a1: a0 + TAU });
            }
            Curve::Line { a, b } => self.push(RawGeom::Seg { a, b }),
        }
    }

    /// Draw an explicit arc about a point — for figures like the golden spiral
    /// whose arcs are quarter turns rather than spans between solved points.
    pub fn arc_at(&mut self, center: PointId, r: f64, a0: f64, a1: f64) {
        let c = self.pos(center);
        self.push(RawGeom::Arc { c, r, a0, a1 });
        self.last_touch = Some(c + DVec2::new(a1.cos(), a1.sin()) * r);
    }

    /// A closed polygon through existing points.
    pub fn polygon(&mut self, pts: &[PointId]) {
        if pts.len() < 2 {
            return;
        }
        for i in 0..pts.len() {
            self.line(pts[i], pts[(i + 1) % pts.len()]);
        }
    }

    /// Draw the minor arc of a circle between two points on it.
    pub fn arc_between(&mut self, c: CurveId, p0: PointId, p1: PointId) {
        self.arc_span(c, p0, p1, false);
    }

    /// Draw the major arc — the long way round.
    pub fn arc_major(&mut self, c: CurveId, p0: PointId, p1: PointId) {
        self.arc_span(c, p0, p1, true);
    }

    fn arc_span(&mut self, cid: CurveId, p0: PointId, p1: PointId, long: bool) {
        let Curve::Circle { c, r } = self.curves.get(cid) else {
            // A "segment between two points on a line" is just a line.
            self.line(p0, p1);
            return;
        };
        let ang = |p: DVec2| {
            let d = p - c;
            d.y.atan2(d.x)
        };
        let a0 = ang(self.pos(p0));
        let a1 = ang(self.pos(p1));
        let mut sweep = (a1 - a0).rem_euclid(TAU);
        if long != (sweep > std::f64::consts::PI) {
            sweep -= TAU;
        }
        self.push(RawGeom::Arc { c, r, a0, a1: a0 + sweep });
        self.last_touch = Some(self.pos(p1));
    }

    fn push(&mut self, geom: RawGeom) {
        self.steps.push(RawStep { geom, role: self.role, group: self.group, width: self.width });
    }

    /// Convert and remove accumulated steps, un-normalized and un-timed.
    ///
    /// For infinite mode, where there is no final extent to normalize against
    /// and no total duration to bake a timeline into — steps are handed out as
    /// they are generated and timed against the wall clock instead.
    pub fn drain_steps(&mut self, seed_offset: usize) -> Vec<Step> {
        let taken = std::mem::take(&mut self.steps);
        taken
            .into_iter()
            .enumerate()
            .map(|(i, s)| convert(&s, DVec2::ZERO, 1.0, seed_offset + i))
            .collect()
    }

    /// Points solved so far — infinite mode reads these back as it grows.
    pub fn point_pos(&self, id: PointId) -> DVec2 {
        self.pts.pos(id)
    }

    // -------------------------------------------------------------- finish

    /// Normalize to a unit figure, bake the timeline, and freeze.
    pub fn finish(mut self, name: &'static str) -> Construction {
        // Bounds over everything drawn.
        let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
        let mut grow = |p: DVec2| {
            lo = lo.min(p);
            hi = hi.max(p);
        };
        for s in &self.steps {
            match s.geom {
                RawGeom::Arc { c, r, .. } => {
                    grow(c - DVec2::splat(r));
                    grow(c + DVec2::splat(r));
                }
                RawGeom::Seg { a, b } => {
                    grow(a);
                    grow(b);
                }
            }
        }
        // Normalize by the furthest *radius* from the centre, not the bounding
        // box half-extent. A box-fit only guarantees the figure fits a square,
        // so a rotated or lopsided figure can still reach √2 at the corners —
        // and `View::fit` frames a unit-radius disc, not a unit square.
        let center = if lo.x.is_finite() { (lo + hi) * 0.5 } else { DVec2::ZERO };
        let mut far: f64 = 0.0;
        for s in &self.steps {
            far = far.max(match s.geom {
                RawGeom::Arc { c, r, .. } => (c - center).length() + r,
                RawGeom::Seg { a, b } => (a - center).length().max((b - center).length()),
            });
        }
        let k = if far > 1e-9 { 1.0 / far } else { 1.0 };

        let mut steps: Vec<Step> =
            self.steps.iter().enumerate().map(|(i, s)| convert(s, center, k, i)).collect();

        bake_timeline(&mut steps, self.overlap);

        self.pts.rescale(center, k);
        // Drop nodes outside the figure. Solving legitimately probes well beyond
        // what gets drawn — extended lines cross far off-frame, and growth looks
        // past its own radius limit — and those points are real, but sparkling
        // them would scatter dots around empty space.
        let nodes: Vec<Node> = self
            .pts
            .iter()
            .filter(|(_, p)| p.p.length() <= 1.02)
            .map(|(id, p)| Node {
                p: p.p.as_vec2(),
                born: p.born,
                seed: (id as f32 * 0.618_034).fract(),
            })
            .collect();

        Construction {
            name,
            steps,
            nodes,
            n_groups: self.group.saturating_add(1),
            pace: self.pace,
            group_names: self.group_names,
        }
    }
}

impl Default for Build {
    fn default() -> Self {
        Self::new()
    }
}

/// Construction-space raw step → normalized drawable step.
fn convert(s: &RawStep, center: DVec2, k: f64, i: usize) -> Step {
    let (geom, arc_ref) = match s.geom {
        RawGeom::Arc { c, r, a0, a1 } => {
            let rr = (r * k) as f32;
            (
                Geom::Arc {
                    c: ((c - center) * k).as_vec2(),
                    r: rr,
                    a0: a0 as f32,
                    a1: a1 as f32,
                },
                rr * (a1 - a0).abs() as f32 * REF_SCALE,
            )
        }
        RawGeom::Seg { a, b } => {
            let (pa, pb) = (((a - center) * k).as_vec2(), ((b - center) * k).as_vec2());
            (Geom::Seg { a: pa, b: pb }, pa.distance(pb) * REF_SCALE)
        }
    };
    Step {
        geom,
        role: s.role,
        group: s.group,
        width: s.width,
        t0: 0.0,
        t1: 1.0,
        arc_ref,
        // Golden-ratio phase offsets — a stateless low-discrepancy sequence,
        // used here for what it is actually good at: decorrelating shimmer,
        // not sequencing causal steps.
        seed: (i as f32 * 0.618_034).fract() * 16.0,
    }
}
