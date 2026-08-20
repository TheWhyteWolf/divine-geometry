//! The two psychedelic layers, and the knobs that drive them.
//!
//! * **Background** — a procedural field generated behind the ink: plasma,
//!   tunnel, a kaleidoscopic ground, or one of two escape-time fractals. It
//!   takes its hue from the live palette so it never argues with the figure.
//! * **Foreground** — a screen-space remap of the composed frame: warp,
//!   kaleidoscope, tunnel, droste spiral, vortex. It folds the background and
//!   the geometry together, as one image.
//! * **Trails** — temporal feedback, sitting between the two so that a fold
//!   mirrors coherent trails rather than trailing a mirrored image.
//!
//! Everything defaults to off, and off means *structurally* off: the renderer
//! skips those passes entirely rather than running an identity shader, so the
//! clean build is byte-for-byte the drawing this app was written to make. The
//! argument in `render/pipeline.rs` against feedback still holds — a
//! construction line has to look identical eight seconds after it was drawn —
//! and it still describes what you get until you ask for otherwise.

use crate::color::Colors;
use crate::render::pipeline::{FeedbackUniforms, FieldUniforms, FxUniforms};

/// Cycle helper: these enums exist to be stepped through by a key or a scroll
/// wheel, in both directions, forever.
macro_rules! cycle_enum {
    ($name:ident { $($var:ident => $label:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($var),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$var),+];

            pub fn index(self) -> u32 {
                Self::ALL.iter().position(|&m| m == self).unwrap_or(0) as u32
            }

            pub fn next(self) -> Self {
                Self::ALL[(self.index() as usize + 1) % Self::ALL.len()]
            }

            pub fn prev(self) -> Self {
                Self::ALL[(self.index() as usize + Self::ALL.len() - 1) % Self::ALL.len()]
            }

            pub fn from_index(i: u32) -> Self {
                Self::ALL[i as usize % Self::ALL.len()]
            }

            pub fn name(self) -> &'static str {
                match self { $($name::$var => $label),+ }
            }

            /// `Off` is always the first variant, and always means "skip the pass".
            pub fn is_on(self) -> bool {
                self != Self::ALL[0]
            }
        }
    };
}

cycle_enum!(BgMode {
    Off => "OFF",
    Plasma => "PLASMA",
    Tunnel => "TUNNEL",
    Kaleido => "KALEIDO",
    Kali => "KALI",
    Julia => "JULIA",
});

cycle_enum!(FgMode {
    Off => "OFF",
    Warp => "WARP",
    Kaleido => "KALEIDO",
    Tunnel => "TUNNEL",
    Droste => "DROSTE",
    Vortex => "VORTEX",
});

cycle_enum!(TrailMode {
    Off => "OFF",
    Soft => "SOFT",
    Long => "LONG",
    Flow => "FLOW",
});

impl TrailMode {
    /// (length, flow) the mode loads into the live knobs. Selecting a mode is a
    /// preset load, exactly as selecting a palette is — the knobs stay free
    /// afterwards.
    fn preset(self) -> (f32, f32) {
        match self {
            TrailMode::Off => (0.0, 0.0),
            TrailMode::Soft => (0.35, 0.0),
            TrailMode::Long => (0.75, 0.0),
            TrailMode::Flow => (0.65, 0.60),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fx {
    pub bg: BgMode,
    pub bg_gain: f32,
    pub bg_scale: f32,
    /// Hue spread of the field. 0 = the palette's single hue.
    pub bg_spread: f32,

    pub fg: FgMode,
    /// Master intensity for the foreground; rides swirl, warp and chroma.
    pub fg_amount: f32,
    pub fg_seg: f32,

    pub trails: TrailMode,
    /// 0 = a breath, 1 = a long smear.
    pub trail_len: f32,
    /// Rotate/zoom of the feedback flow — 0 is a plain ghost, up is a tunnel.
    pub trail_flow: f32,
}

pub const BG_GAIN: (f32, f32) = (0.0, 2.0);
pub const BG_SCALE: (f32, f32) = (0.5, 12.0);
pub const BG_SPREAD: (f32, f32) = (0.0, 1.0);
pub const FG_AMOUNT: (f32, f32) = (0.0, 1.5);
pub const FG_SEG: (f32, f32) = (2.0, 16.0);
pub const TRAIL_LEN: (f32, f32) = (0.0, 1.0);
pub const TRAIL_FLOW: (f32, f32) = (0.0, 1.0);

impl Default for Fx {
    fn default() -> Self {
        Self {
            bg: BgMode::Off,
            bg_gain: 0.5,
            bg_scale: 3.0,
            bg_spread: 0.35,
            fg: FgMode::Off,
            fg_amount: 0.6,
            fg_seg: 6.0,
            trails: TrailMode::Off,
            trail_len: 0.35,
            trail_flow: 0.0,
        }
    }
}

impl Fx {
    /// Load a trail mode and the knobs that come with it.
    pub fn set_trails(&mut self, mode: TrailMode) {
        self.trails = mode;
        if mode.is_on() {
            let (len, flow) = mode.preset();
            self.trail_len = len;
            self.trail_flow = flow;
        }
    }

    pub fn clamped(mut self) -> Self {
        self.bg_gain = self.bg_gain.clamp(BG_GAIN.0, BG_GAIN.1);
        self.bg_scale = self.bg_scale.clamp(BG_SCALE.0, BG_SCALE.1);
        self.bg_spread = self.bg_spread.clamp(BG_SPREAD.0, BG_SPREAD.1);
        self.fg_amount = self.fg_amount.clamp(FG_AMOUNT.0, FG_AMOUNT.1);
        self.fg_seg = self.fg_seg.clamp(FG_SEG.0, FG_SEG.1);
        self.trail_len = self.trail_len.clamp(TRAIL_LEN.0, TRAIL_LEN.1);
        self.trail_flow = self.trail_flow.clamp(TRAIL_FLOW.0, TRAIL_FLOW.1);
        self
    }

    pub fn any_on(&self) -> bool {
        self.bg.is_on() || self.fg.is_on() || self.trails.is_on()
    }

    pub fn summary(&self) -> String {
        format!(
            "bg {} {:.2}  fg {} {:.2}  trails {} {:.2}",
            self.bg.name(),
            self.bg_gain,
            self.fg.name(),
            self.fg_amount,
            self.trails.name(),
            self.trail_len
        )
    }

    pub fn field_uniforms(&self, t: f32, size: (u32, u32), colors: &Colors) -> FieldUniforms {
        // Fractal seeds chosen per mode: the Kali constant sits where the fold
        // makes filigree rather than a solid blob, and the Julia one is just
        // outside the Mandelbrot boundary, where the set is still connected but
        // already lacy.
        let (frac_c, frac_iter) = match self.bg {
            BgMode::Kali => ([0.92f32, 0.62], 18.0),
            _ => ([-0.79f32, 0.156], 28.0),
        };
        FieldUniforms {
            res: [size.0 as f32, size.1 as f32],
            time: t,
            gain: self.bg_gain,
            tint: colors.palette().accent,
            scale: self.bg_scale,
            hue: colors.hue(),
            sat: self.bg_spread,
            seg: self.fg_seg,
            warp: 0.6,
            frac_c,
            frac_iter,
            mode: self.bg.index(),
        }
    }

    pub fn fx_uniforms(&self, t: f32, size: (u32, u32)) -> FxUniforms {
        FxUniforms {
            res: [size.0 as f32, size.1 as f32],
            time: t,
            amount: self.fg_amount,
            seg: self.fg_seg,
            // A slow drift, so a kaleidoscope is never a frozen doily.
            rot: t * 0.05,
            zoom: 1.0,
            swirl: 0.35,
            warp: 0.8,
            chroma: 0.6,
            scroll: 0.12,
            mode: self.fg.index(),
        }
    }

    /// Trail constants, framerate-corrected.
    ///
    /// Every one of these is a per-frame factor, so raising it to `60·dt` turns
    /// "this much per frame at 60 Hz" into "this much per 1/60 s of wall clock".
    /// Without it the trails are half as long on a 144 Hz display, which is the
    /// classic way a feedback effect stops being a property of the piece and
    /// starts being a property of the monitor.
    pub fn feedback_uniforms(&self, dt: f32) -> FeedbackUniforms {
        if !self.trails.is_on() {
            // keep = 0 makes the history pass a plain copy of the scene, which is
            // the identity this whole design leans on.
            return FeedbackUniforms {
                keep: 0.0,
                flow_alpha: 0.0,
                rot: 0.0,
                inv_scale: 1.0,
                clamp_max: CLAMP_MAX,
                _pad: [0.0; 3],
            };
        }
        // Clamp the exponent rather than dt: a stalled frame must not be able to
        // wipe the field or blow it up.
        let k60 = (60.0 * dt).clamp(0.1, 4.0);
        let fade = 0.45 + (0.03 - 0.45) * self.trail_len;
        let flow = self.trail_flow;
        FeedbackUniforms {
            keep: (1.0 - fade).powf(k60),
            flow_alpha: 1.0 - (1.0 - 0.34 * flow).powf(k60),
            rot: 0.006 * flow * k60,
            inv_scale: 1.0 / (1.012 + 0.05 * flow).powf(k60),
            clamp_max: CLAMP_MAX,
            _pad: [0.0; 3],
        }
    }
}

/// Ceiling on the trail field. The structural guard is that bloom never runs
/// over the history texture, so the loop has no halo gain; this is the belt to
/// that pair of braces, and it sits well above the tonemap knee so it never
/// clips anything the grade would have shown.
const CLAMP_MAX: f32 = 8.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_cycles_a_complete_loop_in_both_directions() {
        macro_rules! check {
            ($ty:ident) => {{
                let start = $ty::ALL[0];
                let mut m = start;
                for _ in 0..$ty::ALL.len() {
                    m = m.next();
                }
                assert_eq!(m, start, "{} did not close its loop", stringify!($ty));
                for &v in $ty::ALL {
                    assert_eq!(v.next().prev(), v, "{} prev is not next's inverse", v.name());
                    assert_eq!($ty::from_index(v.index()), v, "{} index round trip", v.name());
                }
                assert!(!$ty::ALL[0].is_on(), "{} must start Off", stringify!($ty));
            }};
        }
        check!(BgMode);
        check!(FgMode);
        check!(TrailMode);
    }

    #[test]
    fn every_mode_yields_finite_uniforms() {
        let colors = Colors::default();
        let mut fx = Fx::default();
        for &bg in BgMode::ALL {
            fx.bg = bg;
            let u = fx.field_uniforms(3.5, (900, 600), &colors);
            let vals = [u.time, u.gain, u.scale, u.hue, u.sat, u.seg, u.warp, u.frac_iter];
            for v in vals.iter().chain(&u.res).chain(&u.tint).chain(&u.frac_c) {
                assert!(v.is_finite(), "{}: non-finite field uniform {v}", bg.name());
            }
        }
        for &fg in FgMode::ALL {
            fx.fg = fg;
            let u = fx.fx_uniforms(3.5, (900, 600));
            let vals = [u.time, u.amount, u.seg, u.rot, u.zoom, u.swirl, u.warp, u.chroma, u.scroll];
            for v in vals.iter().chain(&u.res) {
                assert!(v.is_finite(), "{}: non-finite fx uniform {v}", fg.name());
            }
        }
    }

    #[test]
    fn trail_decay_is_framerate_independent() {
        let mut fx = Fx::default();
        fx.set_trails(TrailMode::Long);
        // One second of decay, accumulated at two very different frame rates.
        let after_one_second = |hz: f32| -> f32 {
            let dt = 1.0 / hz;
            fx.feedback_uniforms(dt).keep.powi(hz as i32)
        };
        let slow = after_one_second(30.0);
        let fast = after_one_second(144.0);
        assert!(slow > 0.0 && slow < 1.0, "a second of decay should bite: {slow}");
        assert!(
            (slow - fast).abs() < 1e-3,
            "trail length drifts with frame rate: 30 Hz {slow} vs 144 Hz {fast}"
        );
    }

    #[test]
    fn trails_off_is_an_exact_copy_of_the_scene() {
        let fb = Fx::default().feedback_uniforms(1.0 / 60.0);
        assert_eq!(fb.keep, 0.0, "off must contribute nothing to the history field");
        assert_eq!(fb.flow_alpha, 0.0);
    }

    #[test]
    fn the_default_is_the_clean_build() {
        let fx = Fx::default();
        assert!(!fx.any_on(), "the app must still open as a plain construction");
        assert!(!fx.bg.is_on() && !fx.fg.is_on() && !fx.trails.is_on());
    }

    #[test]
    fn selecting_a_trail_mode_loads_its_preset_and_off_leaves_the_knobs_alone() {
        let mut fx = Fx::default();
        fx.set_trails(TrailMode::Flow);
        assert!(fx.trail_flow > 0.0, "FLOW must actually flow");
        let (len, flow) = (fx.trail_len, fx.trail_flow);
        fx.set_trails(TrailMode::Off);
        assert_eq!((fx.trail_len, fx.trail_flow), (len, flow), "off should not stomp the knobs");
    }

    #[test]
    fn clamping_keeps_every_knob_inside_its_range() {
        let wild = Fx {
            bg_gain: 99.0,
            bg_scale: -4.0,
            bg_spread: 7.0,
            fg_amount: -1.0,
            fg_seg: 900.0,
            trail_len: 3.0,
            trail_flow: -2.0,
            ..Default::default()
        }
        .clamped();
        assert_eq!(wild, wild.clamped(), "clamping must be idempotent");
        assert!((BG_GAIN.0..=BG_GAIN.1).contains(&wild.bg_gain));
        assert!((BG_SCALE.0..=BG_SCALE.1).contains(&wild.bg_scale));
        assert!((FG_SEG.0..=FG_SEG.1).contains(&wild.fg_seg));
        assert!((TRAIL_LEN.0..=TRAIL_LEN.1).contains(&wild.trail_len));
    }
}
