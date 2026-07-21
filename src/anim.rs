//! Playback: where a construction is in its life cycle, and how bright each
//! role should be as a result.

use crate::color::Palette;
use crate::geom::construction::{Construction, Role, Step};
use crate::tess::StepGpu;

const SETTLE: f32 = 1.5;
const HOLD: f32 = 3.0;
const FADE: f32 = 1.2;

/// Figure-time over which a finished scaffold stroke recedes to its residue.
const SCAFFOLD_DECAY: f32 = 0.12;
/// Real seconds a node's birth flash lasts.
const FLASH_SECS: f32 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Draw,
    Settle,
    Hold,
    Fade,
}

/// What happens to the solved compass points — the pricks the compass leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkMode {
    /// Flash at birth, settle to a quiet twinkle, recede with the scaffold.
    Fade,
    /// Flash at birth, then stay — the page keeps its compass pricks.
    Keep,
    Off,
}

impl MarkMode {
    pub fn next(self) -> Self {
        match self {
            MarkMode::Fade => MarkMode::Keep,
            MarkMode::Keep => MarkMode::Off,
            MarkMode::Off => MarkMode::Fade,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            MarkMode::Fade => "FADE",
            MarkMode::Keep => "KEEP",
            MarkMode::Off => "OFF",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaffoldVis {
    Full,
    Dim,
    Hidden,
}

impl ScaffoldVis {
    pub fn gain(self) -> f32 {
        match self {
            ScaffoldVis::Full => 1.0,
            ScaffoldVis::Dim => 0.45,
            ScaffoldVis::Hidden => 0.0,
        }
    }

    pub fn next(self) -> Self {
        match self {
            ScaffoldVis::Full => ScaffoldVis::Dim,
            ScaffoldVis::Dim => ScaffoldVis::Hidden,
            ScaffoldVis::Hidden => ScaffoldVis::Full,
        }
    }
}

pub struct Anim {
    pub phase: Phase,
    /// Normalized figure time, 0..1 across the Draw phase.
    pub ft: f32,
    /// Seconds into the current phase.
    pub phase_t: f32,
    pub autoplay: bool,
    pub paused: bool,
    pub speed: f32,
}

impl Default for Anim {
    fn default() -> Self {
        Self {
            phase: Phase::Draw,
            ft: 0.0,
            phase_t: 0.0,
            autoplay: true,
            paused: false,
            speed: 1.0,
        }
    }
}

impl Anim {
    pub fn restart(&mut self) {
        self.phase = Phase::Draw;
        self.ft = 0.0;
        self.phase_t = 0.0;
    }

    /// Advance. Returns true when the figure has finished and the next one
    /// should be loaded.
    pub fn tick(&mut self, dt: f32, draw_secs: f32) -> bool {
        if self.paused {
            return false;
        }
        let d = dt * self.speed;
        self.phase_t += d;
        match self.phase {
            Phase::Draw => {
                self.ft = (self.ft + d / draw_secs.max(0.1)).min(1.0);
                if self.ft >= 1.0 {
                    self.phase = Phase::Settle;
                    self.phase_t = 0.0;
                }
            }
            Phase::Settle => {
                if self.phase_t >= SETTLE {
                    self.phase = Phase::Hold;
                    self.phase_t = 0.0;
                }
            }
            Phase::Hold => {
                // Without autoplay, hold forever on the completed figure.
                if self.autoplay && self.phase_t >= HOLD {
                    self.phase = Phase::Fade;
                    self.phase_t = 0.0;
                }
            }
            Phase::Fade => {
                if self.phase_t >= FADE {
                    return true;
                }
            }
        }
        false
    }

    /// Completion payoff: a brief lift the moment the last stroke lands,
    /// decaying through Settle. Peaks just enough to push the figure's crests
    /// over the tonemap knee for a moment — the figure "ignites" as it
    /// finishes, then relaxes into its held glow.
    pub fn settle_pulse(&self) -> f32 {
        match self.phase {
            Phase::Settle => 0.30 * (-self.phase_t * 2.4).exp(),
            _ => 0.0,
        }
    }

    /// 0 while drawing, ramping to 1 across Settle and staying there.
    pub fn phase_k(&self) -> f32 {
        match self.phase {
            Phase::Draw => 0.0,
            Phase::Settle => (self.phase_t / SETTLE).clamp(0.0, 1.0),
            Phase::Hold | Phase::Fade => 1.0,
        }
    }

    /// Whole-figure alpha — 1 except during the fade out.
    pub fn global(&self) -> f32 {
        match self.phase {
            Phase::Fade => (1.0 - self.phase_t / FADE).clamp(0.0, 1.0),
            _ => 1.0,
        }
    }

    /// Jump the reveal to a specific figure time, pausing playback.
    pub fn seek(&mut self, ft: f32) {
        self.ft = ft.clamp(0.0, 1.0);
        self.phase = if self.ft >= 1.0 { Phase::Hold } else { Phase::Draw };
        self.phase_t = 0.0;
        self.paused = true;
    }
}

/// Per-step GPU state for this frame.
///
/// The two-scalar envelope (`phase_k`, `global`) is what makes "guide circles
/// dim while the final figure brightens" one line each instead of a pile of
/// special cases.
#[derive(Clone, Copy)]
pub struct FrameMood {
    pub ft: f32,
    pub phase_k: f32,
    pub global: f32,
    pub scaffold: f32,
    /// Post-completion flash, decaying through Settle.
    pub pulse: f32,
    /// Slow whole-figure breathing during Hold — a held figure should feel
    /// alive, not frozen. Amplitude is tiny (±4%) and stays below the knee.
    pub breathe: f32,
    pub pal: Palette,
}

pub fn step_state(s: &Step, m: &FrameMood) -> StepGpu {
    let (ft, phase_k, global) = (m.ft, m.phase_k, m.global);
    let span = (s.t1 - s.t0).max(1e-6);
    let prog = ((ft - s.t0) / span).clamp(0.0, 1.0);
    let age = ((ft - s.t1) / SCAFFOLD_DECAY).clamp(0.0, 1.0);

    let glow = match s.role {
        Role::Scaffold => {
            // Bright while the compass is moving, then a faint residue that
            // recedes further once the figure takes over.
            let residue = lerp(0.16, 0.06, phase_k);
            lerp(0.55, residue, smoothstep01(age)) * m.scaffold
        }
        // Kept just under the tonemap knee so shimmer crests cross it and the
        // halo does the sparkling — see the note in post.wgsl. The settle pulse
        // and hold-breathe ride on top; the pulse deliberately pokes over.
        Role::Figure => (lerp(0.62, 0.84, phase_k) + m.pulse) * (1.0 + m.breathe),
        Role::Node => 0.25,
        Role::Hud => 0.20,
    };

    let color = match s.role {
        // t0 places the stroke in the draw timeline (gradients); seed carries
        // its golden-ratio ordinal (prism) — both already in the step.
        Role::Figure => m.pal.stroke(s.t0, s.seed / 16.0),
        Role::Scaffold => m.pal.scaffold,
        Role::Node | Role::Hud => m.pal.accent,
    };

    // The pen glints only while this step is actively being drawn.
    let pen = if prog > 0.0 && prog < 1.0 { 1.0 } else { 0.0 };

    // Push the frontier just past both ends when the stroke is fully off or
    // fully on. The shader's reveal is `1 - smoothstep(-0.5, 0.5, (u - head)·arc)`,
    // which is exactly 0.5 where u == head — so head = 1 leaves a half-lit notch
    // at a finished stroke's seam, and head = 0 leaves a half-lit dot at the
    // start of one that hasn't begun.
    let head = if prog <= 0.0 {
        -0.05
    } else if prog >= 1.0 {
        1.05
    } else {
        prog
    };

    StepGpu {
        head,
        glow: glow * global,
        pen: pen * global,
        seed: s.seed,
        color,
        _pad: 0.0,
    }
}

/// Brightness of a construction node at figure-time `ft`.
pub fn node_level(born_t: f32, ft: f32, draw_secs: f32, global: f32, marks: MarkMode) -> f32 {
    if marks == MarkMode::Off {
        return 0.0;
    }
    let flash_ft = (FLASH_SECS / draw_secs.max(0.1)).max(1e-4);
    let age = ft - born_t;
    if age < 0.0 {
        return 0.0;
    }
    // A hard flash well over the tonemap knee, settling down after.
    let k = (age / flash_ft).clamp(0.0, 1.0);
    let settled = match marks {
        // The compass prick stays on the page, bright enough to read as a mark.
        MarkMode::Keep => 0.55,
        _ => 0.25,
    };
    lerp(3.0, settled, smoothstep01(k)) * global
}

/// Figure-time at which each node comes into existence.
pub fn node_times(cons: &Construction) -> Vec<f32> {
    cons.nodes.iter().map(|n| cons.group_start(n.born)).collect()
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::construction::Geom;
    use glam::Vec2;

    fn mood(ft: f32, phase_k: f32, global: f32, scaffold: f32) -> FrameMood {
        FrameMood {
            ft,
            phase_k,
            global,
            scaffold,
            pulse: 0.0,
            breathe: 0.0,
            pal: crate::color::Colors::default().palette(),
        }
    }

    fn step(t0: f32, t1: f32, role: Role) -> Step {
        Step {
            geom: Geom::Seg { a: Vec2::ZERO, b: Vec2::X },
            role,
            group: 0,
            width: 2.0,
            t0,
            t1,
            arc_ref: 100.0,
            seed: 0.0,
        }
    }

    /// The shader's reveal is exactly 0.5 where `u == head`. If head ever lands
    /// on 0 or 1, a stroke that should be fully off shows a half-lit dot at its
    /// start, and one that should be fully on shows a half-lit notch at its
    /// seam. Both ends must overshoot.
    #[test]
    fn head_clears_both_endpoints_when_fully_off_or_on() {
        let s = step(0.25, 0.75, Role::Figure);

        let before = step_state(&s, &mood(0.0, 0.0, 1.0, 1.0));
        assert!(before.head < 0.0, "unstarted stroke has head {}", before.head);

        let after = step_state(&s, &mood(1.0, 1.0, 1.0, 1.0));
        assert!(after.head > 1.0, "finished stroke has head {}", after.head);

        // Mid-draw it tracks progress exactly.
        let mid = step_state(&s, &mood(0.5, 0.0, 1.0, 1.0));
        assert!((mid.head - 0.5).abs() < 1e-6, "mid-draw head was {}", mid.head);
    }

    #[test]
    fn pen_glints_only_while_actively_drawing() {
        let s = step(0.25, 0.75, Role::Figure);
        assert_eq!(step_state(&s, &mood(0.1, 0.0, 1.0, 1.0)).pen, 0.0);
        assert!(step_state(&s, &mood(0.5, 0.0, 1.0, 1.0)).pen > 0.0);
        assert_eq!(step_state(&s, &mood(0.9, 0.0, 1.0, 1.0)).pen, 0.0);
    }

    /// The figure must end up brighter than the scaffold it was built on —
    /// that separation is the whole point of the settle phase.
    #[test]
    fn settle_lifts_the_figure_above_the_scaffold() {
        let scaffold = step(0.0, 0.2, Role::Scaffold);
        let figure = step(0.0, 0.2, Role::Figure);

        let sc_draw = step_state(&scaffold, &mood(0.1, 0.0, 1.0, 1.0)).glow;
        let sc_done = step_state(&scaffold, &mood(1.0, 1.0, 1.0, 1.0)).glow;
        let fig_done = step_state(&figure, &mood(1.0, 1.0, 1.0, 1.0)).glow;

        assert!(sc_done < sc_draw, "scaffold should recede after drawing");
        assert!(fig_done > sc_done * 4.0, "figure should dominate the residue");
        // And it must stay under the tonemap knee so shimmer crests can cross it.
        assert!(fig_done < 0.9, "figure glow {fig_done} is past the knee");
    }

    #[test]
    fn hidden_scaffold_really_is_hidden() {
        let s = step(0.0, 0.2, Role::Scaffold);
        assert_eq!(step_state(&s, &mood(0.1, 0.0, 1.0, ScaffoldVis::Hidden.gain())).glow, 0.0);
    }

    #[test]
    fn fade_drives_everything_to_zero() {
        let s = step(0.0, 0.2, Role::Figure);
        assert_eq!(step_state(&s, &mood(1.0, 1.0, 0.0, 1.0)).glow, 0.0);
    }

    #[test]
    fn nodes_flash_at_birth_then_settle() {
        let flash = node_level(0.5, 0.5, 10.0, 1.0, MarkMode::Fade);
        let settled = node_level(0.5, 1.0, 10.0, 1.0, MarkMode::Fade);
        assert_eq!(node_level(0.5, 0.4, 10.0, 1.0, MarkMode::Fade), 0.0, "node lit before it was solved");
        assert!(flash > 2.0, "birth flash was only {flash}");
        assert!(settled < 0.4, "node never settled, still at {settled}");
    }
}

#[inline]
fn smoothstep01(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
