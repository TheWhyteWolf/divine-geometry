//! Glitter — dust motes drifting down through the beam.
//!
//! Not literal snow: small point sprites that fall, wander, and flash. Three
//! layers of motion and light:
//!
//! * **Lifecycle.** Each mote has its own fade period and phase: it eases into
//!   existence, lives a few seconds, dissolves, and returns somewhere else.
//!   The field twinkles into being rather than presenting a permanent column
//!   of particles riding top to bottom.
//! * **Brownian wander.** A damped random-walk velocity per mote (an
//!   Ornstein–Uhlenbeck process in all but name): white-noise kicks
//!   accumulating into a velocity that decays over ~1.5 s. `drift` scales the
//!   kick strength. With gravity at zero this is pure floating dust.
//! * **The flash.** Most of a mote's life is spent as a barely-visible speck
//!   well under the tonemap knee; a narrow `sin^28` pulse briefly spikes it far
//!   over, and the bloom does the glinting — how real glitter behaves as facets
//!   sweep past alignment with a light source.
//!
//! Motes live in *screen space*, drawn through the HUD's identity camera: the
//! figure zooms behind them while they keep falling in the room. That uniform
//! carries `shimmer = 0`, so the CPU is the only brightness animation and
//! `--shot` renders stay reproducible.

use std::f32::consts::TAU;

use crate::params::SplitMix;
use crate::tess::PointInstance;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnowMode {
    Off,
    Light,
    Dense,
}

impl SnowMode {
    pub fn next(self) -> Self {
        match self {
            SnowMode::Off => SnowMode::Light,
            SnowMode::Light => SnowMode::Dense,
            SnowMode::Dense => SnowMode::Off,
        }
    }

    pub fn prev(self) -> Self {
        self.next().next()
    }

    fn count(self) -> usize {
        match self {
            SnowMode::Off => 0,
            SnowMode::Light => 170,
            SnowMode::Dense => 400,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SnowMode::Off => "OFF",
            SnowMode::Light => "LIGHT",
            SnowMode::Dense => "DENSE",
        }
    }
}

struct Mote {
    x: f32,
    y: f32,
    /// 0 = far (small, slow, dim), 1 = near. Drives size, speed and level
    /// together, which is what sells the parallax without a real camera.
    depth: f32,
    sway_phase: f32,
    sway_rate: f32,
    /// Spark timing: phase and rate of the narrow flash pulse.
    spark_phase: f32,
    spark_rate: f32,
    /// Lifecycle: seconds per appear/live/dissolve cycle, and where in it this
    /// mote starts. Randomized per mote so the field never breathes in unison.
    fade_period: f32,
    fade_phase: f32,
    /// Brownian velocity, damped each frame and kicked by white noise.
    bvx: f32,
    bvy: f32,
}

pub struct Snow {
    motes: Vec<Mote>,
    pub mode: SnowMode,
    /// Fall-speed multiplier, 0..2.5. Zero suspends the field entirely.
    pub gravity: f32,
    /// Brownian kick strength, 0..2.5. The "how stirred is the air" knob.
    pub drift: f32,
    /// Noise source for the per-frame Brownian kicks. Seeded once, so a fixed
    /// step sequence (the --shot pre-roll) reproduces exactly.
    rng: SplitMix,
}

impl Snow {
    pub fn new() -> Self {
        Self {
            motes: Vec::new(),
            mode: SnowMode::Off,
            gravity: 1.0,
            drift: 0.5,
            rng: SplitMix::new(0xD1_F7),
        }
    }

    pub fn set_mode(&mut self, mode: SnowMode, size: (u32, u32)) {
        self.mode = mode;
        let want = mode.count();
        let (w, h) = (size.0 as f32, size.1 as f32);
        // Deterministic spawn — seeded by index, not a clock.
        let mut rng = SplitMix::new(0x51_1CE_D);
        self.motes.clear();
        self.motes.reserve(want);
        for _ in 0..want {
            let depth = 0.25 + 0.75 * rng.unit();
            self.motes.push(Mote {
                x: rng.unit() * w,
                y: rng.unit() * h,
                depth,
                sway_phase: rng.unit() * TAU,
                sway_rate: 0.35 + 0.9 * rng.unit(),
                spark_phase: rng.unit() * TAU,
                // Slow enough that flashes are events, fast enough that the
                // field is always alive somewhere.
                spark_rate: 0.25 + 1.1 * rng.unit(),
                fade_period: 3.5 + 5.0 * rng.unit(),
                fade_phase: rng.unit(),
                bvx: 0.0,
                bvy: 0.0,
            });
        }
    }

    pub fn update(&mut self, dt: f32, t: f32, size: (u32, u32)) {
        let (w, h) = (size.0 as f32, size.1 as f32);
        let margin = 8.0;
        // Velocity decay factor for the Brownian walk (τ ≈ 1.5 s).
        let damp = (1.0 - dt / 1.5).max(0.0);
        let cap = 45.0 * self.drift + 1.0;
        for m in &mut self.motes {
            // Brownian: white-noise kicks into a damped velocity. Depth-scaled
            // like everything else — near motes are stirred harder.
            let kick = 300.0 * self.drift * (0.4 + 0.6 * m.depth);
            m.bvx = (m.bvx * damp + (self.rng.unit() - 0.5) * 2.0 * kick * dt).clamp(-cap, cap);
            m.bvy = (m.bvy * damp + (self.rng.unit() - 0.5) * 2.0 * kick * dt).clamp(-cap, cap);

            // Gravity, sway and a whisper of wind — all scaled so `gravity: 0`
            // plus some drift is dust hanging in still air.
            let fall = (14.0 + 40.0 * m.depth) * self.gravity;
            m.y += (fall + m.bvy) * dt;
            m.x += ((t * m.sway_rate + m.sway_phase).sin() * 9.0 * m.depth * self.gravity
                + 3.0 * self.gravity
                + m.bvx)
                * dt;

            if m.y > h + margin {
                m.y = -margin;
                m.x = (m.x + w * 0.381_966).rem_euclid(w);
            } else if m.y < -margin {
                m.y += h + 2.0 * margin;
            }
            if m.x > w + margin {
                m.x -= w + 2.0 * margin;
            } else if m.x < -margin {
                m.x += w + 2.0 * margin;
            }
        }
    }

    /// Append this frame's sprites. Y is negated for the identity camera, the
    /// same convention as the HUD glyphs.
    pub fn instances(&self, t: f32, out: &mut Vec<PointInstance>) {
        for m in &self.motes {
            // Lifecycle envelope: ease in over the first 15% of the cycle,
            // hold, dissolve by 65%, absent for the rest. Random period and
            // phase per mote — the field twinkles, it never marches.
            let cyc = (t / m.fade_period + m.fade_phase).fract();
            let fade = smoothstep(0.0, 0.15, cyc) * (1.0 - smoothstep(0.50, 0.65, cyc));
            if fade <= 0.0 {
                continue;
            }

            let s = (t * m.spark_rate + m.spark_phase).sin().max(0.0);
            // Narrow pulse: dormant speck, rare hot flash. The flash peak (2.4)
            // is far over the 0.8 tonemap knee, so bloom does the glinting.
            let spark = s.powi(28);
            let level = m.depth * (0.085 + 2.4 * spark) * fade;
            out.push(PointInstance {
                center: [m.x, -m.y],
                radius: 0.8 + 1.7 * m.depth,
                level,
                seed: m.sway_phase,
                _pad: 0.0,
            });
        }
    }

    pub fn is_active(&self) -> bool {
        !self.motes.is_empty()
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(snow: &mut Snow, secs: f32, size: (u32, u32)) {
        let dt = 1.0 / 60.0;
        let steps = (secs / dt) as usize;
        for i in 0..steps {
            snow.update(dt, i as f32 * dt, size);
        }
    }

    #[test]
    fn motes_stay_inside_the_wrap_bounds_even_when_stirred() {
        let size = (800, 600);
        let mut snow = Snow::new();
        snow.set_mode(SnowMode::Dense, size);
        snow.drift = 2.5;
        snow.gravity = 2.5;
        run(&mut snow, 30.0, size);
        for m in &snow.motes {
            assert!((-9.0..=809.0).contains(&m.x), "x escaped: {}", m.x);
            assert!((-9.0..=609.0).contains(&m.y), "y escaped: {}", m.y);
            assert!(m.bvx.is_finite() && m.bvy.is_finite());
        }
    }

    #[test]
    fn zero_gravity_zero_drift_means_still_air() {
        let size = (800, 600);
        let mut snow = Snow::new();
        snow.set_mode(SnowMode::Light, size);
        snow.gravity = 0.0;
        snow.drift = 0.0;
        let before: Vec<(f32, f32)> = snow.motes.iter().map(|m| (m.x, m.y)).collect();
        run(&mut snow, 5.0, size);
        for (m, (x, y)) in snow.motes.iter().zip(&before) {
            assert!((m.x - x).abs() < 1e-3 && (m.y - y).abs() < 1e-3, "mote moved in still air");
        }
    }

    #[test]
    fn brownian_actually_wanders_without_gravity() {
        let size = (800, 600);
        let mut snow = Snow::new();
        snow.set_mode(SnowMode::Light, size);
        snow.gravity = 0.0;
        snow.drift = 1.0;
        let before: Vec<(f32, f32)> = snow.motes.iter().map(|m| (m.x, m.y)).collect();
        run(&mut snow, 10.0, size);
        let moved = snow
            .motes
            .iter()
            .zip(&before)
            .filter(|(m, (x, y))| ((m.x - x).powi(2) + (m.y - y).powi(2)).sqrt() > 4.0)
            .count();
        assert!(
            moved > snow.motes.len() / 2,
            "only {moved}/{} motes wandered under pure drift",
            snow.motes.len()
        );
    }

    /// The lifecycle: at any instant a healthy fraction of motes is absent, and
    /// a given mote is visible at some times and gone at others.
    #[test]
    fn motes_fade_in_and_out_rather_than_persisting() {
        let mut snow = Snow::new();
        snow.set_mode(SnowMode::Dense, (800, 800));
        let mut out = Vec::new();

        let mut counts = Vec::new();
        for i in 0..40 {
            out.clear();
            snow.instances(i as f32 * 0.5, &mut out);
            counts.push(out.len());
        }
        let total = snow.motes.len();
        for &c in &counts {
            assert!(c < total, "every mote visible at once — no lifecycle");
            assert!(c > total / 4, "field nearly empty: {c}/{total}");
        }
        // And visibility genuinely turns over: min and max across the sweep
        // must differ, or the envelope is static.
        let (lo, hi) = (counts.iter().min().unwrap(), counts.iter().max().unwrap());
        assert!(hi > lo, "visible population never changed");
    }

    #[test]
    fn sparks_cross_the_tonemap_knee_but_rest_below_it() {
        let mut snow = Snow::new();
        snow.set_mode(SnowMode::Light, (800, 800));
        let mut out = Vec::new();
        let mut peak = 0.0f32;
        let mut sum = 0.0f64;
        let mut n = 0usize;
        for i in 0..600 {
            out.clear();
            snow.instances(i as f32 * 0.05, &mut out);
            for p in &out {
                peak = peak.max(p.level);
                sum += p.level as f64;
                n += 1;
            }
        }
        let mean = (sum / n as f64) as f32;
        assert!(peak > 1.2, "no mote ever flashed over the knee (peak {peak})");
        assert!(mean < 0.25, "field is too bright at rest (mean {mean})");
    }
}
