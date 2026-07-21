//! Palettes: preset and user-defined colour for the line families.
//!
//! Three kinds of colouring for the figure:
//!
//! * **Solid** — one colour, as the classical build.
//! * **Gradient** — hue interpolates along each stroke's *draw time*, so the
//!   figure fades from one colour at its first stroke to another at its last.
//!   The construction order becomes visible as colour: you can read which part
//!   was drawn first long after the pen has gone.
//! * **Prism** — every stroke takes its own hue, stepped by the golden-ratio
//!   sequence already carried in the step seed. Adjacent strokes land far apart
//!   on the wheel, so the figure reads as faceted rather than striped.
//!
//! Scaffold and accent stay single colours derived from the base hue, so the
//! working-out never competes with the figure.
//!
//! The pen glint is deliberately **not** palette-tinted. A specular highlight
//! is the colour of the light, not the body — keeping it white is what makes a
//! gold or violet line still read as *glowing* rather than merely coloured.

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Solid,
    /// Second endpoint's hue and saturation.
    Gradient(f32, f32),
    Prism,
}

/// (name, base hue, base sat, kind)
const PRESETS: &[(&str, f32, f32, Kind)] = &[
    // Matches the original hardcoded cool-white tint, so the default palette
    // is near-identical to the pre-colour build.
    ("MOONLIGHT", 215.0, 0.05, Kind::Solid),
    ("GOLD", 46.0, 0.72, Kind::Solid),
    ("EMBER", 18.0, 0.85, Kind::Solid),
    ("ICE", 196.0, 0.55, Kind::Solid),
    ("VERDANT", 152.0, 0.55, Kind::Solid),
    ("AMETHYST", 278.0, 0.50, Kind::Solid),
    ("ROSE", 338.0, 0.45, Kind::Solid),
    ("AURORA", 150.0, 0.65, Kind::Gradient(275.0, 0.55)),
    ("SUNSET", 15.0, 0.80, Kind::Gradient(320.0, 0.55)),
    ("OCEANIC", 185.0, 0.60, Kind::Gradient(245.0, 0.50)),
    ("PRISM", 0.0, 0.65, Kind::Prism),
];

/// How the figure's strokes take colour, resolved per step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Coloring {
    Solid([f32; 3]),
    Gradient { h0: f32, s0: f32, h1: f32, s1: f32 },
    /// `offset` rotates the whole wheel — the hue keys spin a prism figure.
    Prism { offset: f32, s: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub figure: Coloring,
    pub scaffold: [f32; 3],
    pub accent: [f32; 3],
}

impl Palette {
    /// Colour of one figure stroke. `t` is the stroke's position in the draw
    /// timeline (its baked t0); `u` is its golden-ratio ordinal in [0, 1).
    pub fn stroke(&self, t: f32, u: f32) -> [f32; 3] {
        match self.figure {
            Coloring::Solid(c) => c,
            Coloring::Gradient { h0, s0, h1, s1 } => {
                // Shortest-path hue interpolation, so 340°→20° crosses red
                // rather than dragging through the whole wheel.
                let mut dh = (h1 - h0).rem_euclid(360.0);
                if dh > 180.0 {
                    dh -= 360.0;
                }
                hsv_linear(h0 + dh * t, s0 + (s1 - s0) * t, 1.0)
            }
            Coloring::Prism { offset, s } => hsv_linear(offset + u * 360.0, s, 1.0),
        }
    }
}

/// Current colour state: a preset index plus a hue/sat pair that diverges into
/// a user palette the moment either is touched.
#[derive(Clone, Copy, Debug)]
pub struct Colors {
    preset: usize,
    hue: f32,
    sat: f32,
    kind: Kind,
    user: bool,
}

impl Default for Colors {
    fn default() -> Self {
        let (_, h, s, k) = PRESETS[0];
        Self { preset: 0, hue: h, sat: s, kind: k, user: false }
    }
}

impl Colors {
    fn load(&mut self, idx: usize) {
        self.preset = idx;
        let (_, h, s, k) = PRESETS[idx];
        self.hue = h;
        self.sat = s;
        self.kind = k;
        self.user = false;
    }

    pub fn cycle(&mut self) {
        self.load((self.preset + 1) % PRESETS.len());
    }

    pub fn cycle_back(&mut self) {
        self.load((self.preset + PRESETS.len() - 1) % PRESETS.len());
    }

    /// Rotates the base hue — and a gradient's second hue rides along, so the
    /// span between them is preserved while the whole ramp turns.
    pub fn shift_hue(&mut self, deg: f32) {
        self.hue = (self.hue + deg).rem_euclid(360.0);
        if let Kind::Gradient(h1, s1) = self.kind {
            self.kind = Kind::Gradient((h1 + deg).rem_euclid(360.0), s1);
        }
        self.user = true;
    }

    pub fn step_sat(&mut self) {
        // Cycle through useful saturations rather than a +/- pair — one key.
        self.sat = match self.sat {
            s if s < 0.15 => 0.35,
            s if s < 0.45 => 0.60,
            s if s < 0.70 => 0.85,
            _ => 0.05,
        };
        if let Kind::Gradient(h1, _) = self.kind {
            self.kind = Kind::Gradient(h1, self.sat);
        }
        self.user = true;
    }

    pub fn name(&self) -> String {
        if self.user {
            format!("USER {:.0} {:.0}%", self.hue, self.sat * 100.0)
        } else {
            PRESETS[self.preset].0.to_string()
        }
    }

    pub fn palette(&self) -> Palette {
        let figure = match self.kind {
            Kind::Solid => Coloring::Solid(hsv_linear(self.hue, self.sat, 1.0)),
            Kind::Gradient(h1, s1) => {
                Coloring::Gradient { h0: self.hue, s0: self.sat, h1, s1 }
            }
            Kind::Prism => Coloring::Prism { offset: self.hue, s: self.sat },
        };
        // Scaffold and accent hang off the *midpoint* hue for gradients, so
        // the working-out sits between the ramp's ends rather than under one.
        let base = match self.kind {
            Kind::Gradient(h1, _) => {
                let mut dh = (h1 - self.hue).rem_euclid(360.0);
                if dh > 180.0 {
                    dh -= 360.0;
                }
                (self.hue + dh * 0.5).rem_euclid(360.0)
            }
            _ => self.hue,
        };
        let sat = if self.kind == Kind::Prism { 0.3 } else { self.sat };
        Palette {
            figure,
            scaffold: hsv_linear(base, sat * 0.55, 0.92),
            accent: hsv_linear(base, sat * 0.35, 1.0),
        }
    }
}

/// HSV → linear RGB. Hue in degrees.
pub fn hsv_linear(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [srgb_to_linear(r + m), srgb_to_linear(g + m), srgb_to_linear(b + m)]
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moonlight_matches_the_original_tint() {
        let p = Colors::default().palette();
        let Coloring::Solid(fig) = p.figure else {
            panic!("moonlight must be solid")
        };
        let old = [0.95f32, 0.97, 1.0].map(srgb_to_linear);
        for (a, b) in fig.iter().zip(old.iter()) {
            assert!((a - b).abs() < 0.02, "moonlight drifted: {fig:?} vs {old:?}");
        }
    }

    #[test]
    fn every_preset_yields_finite_in_range_colour() {
        let mut c = Colors::default();
        for _ in 0..PRESETS.len() {
            let p = c.palette();
            for t in [0.0, 0.33, 0.71, 1.0] {
                for ch in p.stroke(t, t).iter().chain(&p.scaffold).chain(&p.accent) {
                    assert!(ch.is_finite() && (0.0..=1.0).contains(ch), "{}: {ch}", c.name());
                }
            }
            c.cycle();
        }
    }

    #[test]
    fn gradient_hits_both_endpoints_and_moves_between() {
        let mut c = Colors::default();
        while c.name() != "AURORA" {
            c.cycle();
        }
        let p = c.palette();
        let start = p.stroke(0.0, 0.0);
        let end = p.stroke(1.0, 0.0);
        let mid = p.stroke(0.5, 0.0);
        assert_eq!(start, hsv_linear(150.0, 0.65, 1.0));
        assert_eq!(end, hsv_linear(275.0, 0.55, 1.0));
        assert_ne!(mid, start);
        assert_ne!(mid, end);
    }

    #[test]
    fn prism_gives_adjacent_strokes_distant_hues() {
        let mut c = Colors::default();
        while c.name() != "PRISM" {
            c.cycle();
        }
        let p = c.palette();
        // Golden-ratio ordinals for consecutive strokes.
        let u: Vec<f32> = (0..5).map(|i| (i as f32 * 0.618_034).fract()).collect();
        let cols: Vec<_> = u.iter().map(|&u| p.stroke(0.5, u)).collect();
        for w in cols.windows(2) {
            let d: f32 = w[0].iter().zip(&w[1]).map(|(a, b)| (a - b).abs()).sum();
            assert!(d > 0.15, "consecutive prism strokes too similar: {w:?}");
        }
    }

    #[test]
    fn cycling_is_a_complete_loop_and_edits_diverge() {
        let mut c = Colors::default();
        for _ in 0..PRESETS.len() {
            c.cycle();
        }
        assert_eq!(c.name(), "MOONLIGHT");
        c.cycle_back();
        assert_eq!(c.name(), "PRISM");
        c.shift_hue(30.0);
        assert!(c.name().starts_with("USER"));
    }
}
