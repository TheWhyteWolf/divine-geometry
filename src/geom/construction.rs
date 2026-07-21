//! The finished, immutable construction: an ordered list of drawn steps with
//! baked timing, plus the nodes (solved intersections) that produced them.

use glam::Vec2;

/// Nominal pixels per construction unit — the reference scale that step
/// durations are weighted against, so pacing doesn't shift with window size.
pub const REF_SCALE: f32 = 420.0;

/// Ink coverage a figure can reach before exposure starts pulling back. Set
/// just above what the classical figures use, so they are unaffected.
const REF_COVER: f32 = 0.12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Compass arcs and guide lines — the working-out. Bright while being
    /// drawn, then recedes to a faint residue.
    Scaffold,
    /// The figure itself. Brightens as the scaffold recedes.
    Figure,
    /// A solved intersection point.
    Node,
    /// On-screen text.
    Hud,
}

#[derive(Clone, Copy, Debug)]
pub enum Geom {
    /// Signed sweep from a0 to a1. |a1 − a0| = TAU is a full compass circle.
    Arc { c: Vec2, r: f32, a0: f32, a1: f32 },
    Seg { a: Vec2, b: Vec2 },
}

#[derive(Clone, Copy, Debug)]
pub struct Step {
    pub geom: Geom,
    pub role: Role,
    pub group: u16,
    pub width: f32,
    /// Normalized figure-time window, baked by `bake_timeline`.
    pub t0: f32,
    pub t1: f32,
    /// Pixel arc length at REF_SCALE — feeds duration weighting.
    pub arc_ref: f32,
    /// Decorrelates this step's shimmer phase.
    pub seed: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub p: Vec2,
    /// Group ordinal this node was solved in.
    pub born: u16,
    pub seed: f32,
}

pub struct Construction {
    pub name: &'static str,
    pub steps: Vec<Step>,
    pub nodes: Vec<Node>,
    pub n_groups: u16,
    pub pace: f32,
    /// Group index → label, for the HUD.
    pub group_names: Vec<(u16, &'static str)>,
}

impl Construction {
    /// Seconds to draw this figure.
    ///
    /// Sub-linear in the step count, and that is the whole trick. Constant time
    /// per step makes Metatron's 78 chords a five-minute sit; constant total
    /// time makes Vesica's two compass swings flick past in 100 ms. `sqrt` gives
    /// roughly an 8× speed range over a 21× count range — about how a human
    /// draftsman actually accelerates as the individual marks get simpler.
    pub fn draw_seconds(&self) -> f32 {
        (4.0 + 2.0 * (self.n_groups as f32).sqrt()).clamp(5.0, 22.0) * self.pace
    }

    /// Brightness scale for how much ink this figure puts on the page.
    ///
    /// The field is additive, so a figure is as bright as the number of strokes
    /// crossing a given pixel. That is fine at fourteen circles and blows out to
    /// a white disc at two hundred — and a user-facing symmetry knob reaches two
    /// hundred easily. Estimating coverage from total stroke area and pulling
    /// the exposure down to match keeps a dense seed legible instead of solid.
    pub fn exposure(&self) -> f32 {
        let ink: f32 = self.steps.iter().map(|s| s.arc_ref * s.width).sum();
        let area = (2.0 * REF_SCALE) * (2.0 * REF_SCALE);
        let cover = (ink / area).max(REF_COVER);
        (REF_COVER / cover).powf(0.95)
    }

    /// The figure-time at which group `g` starts — used by step-scrubbing.
    pub fn group_start(&self, g: u16) -> f32 {
        self.steps
            .iter()
            .filter(|s| s.group == g)
            .map(|s| s.t0)
            .fold(f32::INFINITY, f32::min)
            .min(1.0)
    }

    pub fn dump(&self) {
        eprintln!("── {} ── {} steps, {} groups, {} nodes, {:.1}s",
            self.name, self.steps.len(), self.n_groups, self.nodes.len(), self.draw_seconds());
        for (i, s) in self.steps.iter().enumerate() {
            eprintln!(
                "  {i:>3} g{:<3} {:?} t[{:.3},{:.3}] {:?}",
                s.group, s.role, s.t0, s.t1, s.geom
            );
        }
    }
}

/// Assign each step a normalized [t0, t1] window.
///
/// Steps in the same group start together; groups advance by (1 − overlap).
/// Length weighting is sub-linear: without it, Metatron's chords (which range
/// from 1 to 4 units long) all take identical time, and the short ones look
/// rushed while the diameters crawl.
pub fn bake_timeline(steps: &mut [Step], overlap: f32) {
    if steps.is_empty() {
        return;
    }
    let mut lens: Vec<f32> = steps.iter().map(|s| s.arc_ref).collect();
    lens.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = lens[lens.len() / 2].max(1e-3);

    let durs: Vec<f32> = steps
        .iter()
        .map(|s| (s.arc_ref / median).sqrt().clamp(0.6, 1.6))
        .collect();

    let mut cursor = 0.0f32;
    let mut i = 0usize;
    while i < steps.len() {
        let g = steps[i].group;
        let mut j = i;
        let mut gdur = 0.0f32;
        while j < steps.len() && steps[j].group == g {
            gdur = gdur.max(durs[j]);
            j += 1;
        }
        for k in i..j {
            steps[k].t0 = cursor;
            steps[k].t1 = cursor + durs[k];
        }
        cursor += gdur * (1.0 - overlap).max(0.05);
        i = j;
    }

    let end = steps.iter().map(|s| s.t1).fold(0.0f32, f32::max).max(1e-6);
    for s in steps.iter_mut() {
        s.t0 /= end;
        s.t1 /= end;
    }
}
