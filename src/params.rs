//! The seed settings a figure is constructed from.
//!
//! Because the engine *solves* intersections rather than replaying stored
//! coordinates, changing `symmetry` from 6 to 7 doesn't distort a hexagon — it
//! produces a genuinely correct sevenfold construction, with every centre still
//! derived. That's the payoff for building a real compass.

/// A tiny SplitMix64. A whole RNG crate would be a dependency for six numbers.
pub(crate) struct SplitMix(u64);

impl SplitMix {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ 0xA076_1D64_78BD_642F)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + (self.next() % (hi - lo + 1) as u64) as u32
    }

    pub(crate) fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u32 << 24) as f32
    }

    fn between(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Rotational symmetry of the seed frame. 6 is the classical value; every
    /// other order is an equally valid construction.
    pub symmetry: u32,
    /// How many generations the lattice grows outward.
    pub rings: u32,
    /// Petal radius as a multiple of the compass walk chord. 1.0 is classical —
    /// the walking circle *is* the petal.
    pub ratio: f32,
    /// Star polygon step, for figures that connect every k-th vertex.
    pub skip: u32,
    /// Whole-figure rotation, in turns.
    pub twist: f32,
    /// The seed this was drawn from, or 0 if hand-set.
    pub seed: u64,
}

pub const SYMMETRY: (u32, u32) = (3, 16);
pub const RINGS: (u32, u32) = (1, 5);
pub const RATIO: (f32, f32) = (0.45, 1.7);
pub const SKIP: (u32, u32) = (1, 7);

impl Default for Params {
    fn default() -> Self {
        Self { symmetry: 6, rings: 2, ratio: 1.0, skip: 2, twist: 0.0, seed: 0 }
    }
}

impl Params {
    /// Draw a fresh set of parameters. Ranges are chosen so every seed lands on
    /// something that actually constructs — no combination here can fail to
    /// close or explode the point count.
    pub fn from_seed(seed: u64) -> Self {
        let mut r = SplitMix(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ 0xA076_1D64_78BD_642F);
        let symmetry = r.range(SYMMETRY.0, 12);
        Self {
            symmetry,
            // Growth is superlinear in the symmetry order, so cap the rings for
            // high-order seeds or a single figure can reach thousands of circles.
            rings: r.range(RINGS.0, if symmetry > 8 { 2 } else { 3 }),
            ratio: if r.unit() < 0.55 { 1.0 } else { r.between(RATIO.0, RATIO.1) },
            // {N/k} needs k < N/2 or the star degenerates into a polygon traced
            // backwards; must agree with `clamped` or round-tripping breaks.
            skip: r.range(SKIP.0, ((symmetry - 1) / 2).max(1)),
            twist: if r.unit() < 0.6 { 0.0 } else { r.between(-0.5, 0.5) },
            seed,
        }
    }

    pub fn clamped(mut self) -> Self {
        self.symmetry = self.symmetry.clamp(SYMMETRY.0, SYMMETRY.1);
        self.rings = self.rings.clamp(RINGS.0, RINGS.1);
        self.ratio = self.ratio.clamp(RATIO.0, RATIO.1);
        // A star polygon needs skip < N/2, or {N/k} degenerates to a polygon
        // traced backwards.
        self.skip = self.skip.clamp(SKIP.0, (self.symmetry.saturating_sub(1) / 2).max(1));
        self.twist = self.twist.clamp(-1.0, 1.0);
        self
    }

    /// Rotation of the seed frame, in radians.
    pub fn twist_rad(&self) -> f64 {
        self.twist as f64 * std::f64::consts::TAU
    }

    pub fn summary(&self) -> String {
        format!(
            "sym {}  rings {}  ratio {:.2}  skip {}  twist {:+.2}  seed {}",
            self.symmetry, self.rings, self.ratio, self.skip, self.twist, self.seed
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_stay_inside_their_ranges() {
        for s in 1u64..500 {
            let p = Params::from_seed(s);
            assert_eq!(p, p.clamped(), "seed {s} produced out-of-range params: {p:?}");
            assert!((SYMMETRY.0..=SYMMETRY.1).contains(&p.symmetry));
            assert!((RINGS.0..=RINGS.1).contains(&p.rings));
        }
    }

    #[test]
    fn seeds_are_deterministic_and_varied() {
        assert_eq!(Params::from_seed(42), Params::from_seed(42));
        let a: Vec<u32> = (0..40).map(|s| Params::from_seed(s).symmetry).collect();
        let distinct: std::collections::HashSet<_> = a.iter().collect();
        assert!(distinct.len() > 4, "seed bank barely varies symmetry: {distinct:?}");
    }

    #[test]
    fn clamping_keeps_star_polygons_non_degenerate() {
        let p = Params { symmetry: 5, skip: 7, ..Default::default() }.clamped();
        assert!(p.skip < p.symmetry.div_ceil(2), "skip {} too large for N=5", p.skip);
    }
}
