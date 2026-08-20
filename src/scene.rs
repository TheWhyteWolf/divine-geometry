//! Binds a `Construction` to the renderer: static tessellation once per figure,
//! a small per-frame state table after that.

use crate::anim::{node_level, node_times, step_state, Anim, FrameMood, MarkMode, ScaffoldVis};
use crate::color::Palette;
use crate::geom::construction::{Construction, Geom};
use crate::tess::{PointInstance, StepGpu, Tess};

pub struct Scene {
    pub cons: Construction,
    pub tess: Tess,
    /// Per-step state, rewritten every frame. 16 bytes each — Metatron's whole
    /// animation is a 1.7 KB buffer write.
    pub steps: Vec<StepGpu>,
    pub points: Vec<PointInstance>,
    node_t: Vec<f32>,
}

impl Scene {
    pub fn new(cons: Construction, detail_scale: f32) -> Self {
        let node_t = node_times(&cons);
        let steps = vec![StepGpu::default(); cons.steps.len()];
        let points = cons
            .nodes
            .iter()
            .map(|n| PointInstance {
                center: [n.p.x, n.p.y],
                radius: 2.5,
                level: 0.0,
                seed: n.seed,
                _pad: 0.0,
            })
            .collect();
        let mut s = Self { cons, tess: Tess::default(), steps, points, node_t };
        s.retessellate(detail_scale);
        s
    }

    /// Rebuild the static vertex data. Only on figure change, or when the camera
    /// has drifted far enough that arc faceting would show — never on resize,
    /// and never merely because the view moved.
    pub fn retessellate(&mut self, detail_scale: f32) {
        self.tess.clear();
        self.tess.detail_scale = detail_scale.max(1.0);
        for (i, s) in self.cons.steps.iter().enumerate() {
            match s.geom {
                Geom::Arc { c, r, a0, a1 } => self.tess.arc(c, r, a0, a1, s.width, i as u32),
                Geom::Seg { a, b } => self.tess.seg(a, b, s.width, i as u32),
            }
        }
    }

    pub fn update(
        &mut self,
        anim: &Anim,
        scaffold: ScaffoldVis,
        marks: MarkMode,
        pal: Palette,
        t: f32,
    ) {
        use crate::anim::Phase;
        // Drop anything appended past the figure's own steps — the app pushes
        // the HUD's two borrowed slots onto this vec every frame, and the loop
        // below zips, so without this the vec would grow by two per frame
        // forever. It also keeps the HUD's slot index fixed, which is what stops
        // `refresh_hud` from rebuilding the text geometry on every single frame.
        self.steps.truncate(self.cons.steps.len());
        let mood = FrameMood {
            ft: anim.ft,
            phase_k: anim.phase_k(),
            global: anim.global(),
            scaffold: scaffold.gain(),
            pulse: anim.settle_pulse(),
            // Hold breathing only — while drawing, the pen is the life.
            breathe: if anim.phase == Phase::Hold { 0.04 * (t * 0.6).sin() } else { 0.0 },
            pal,
        };
        for (out, s) in self.steps.iter_mut().zip(&self.cons.steps) {
            *out = step_state(s, &mood);
        }
        let secs = self.cons.draw_seconds();
        // Kept marks ignore scaffold dimming: the pricks are part of the page,
        // not the working-out.
        let sc = if marks == MarkMode::Keep { 1.0 } else { scaffold.gain().max(0.35) };
        for (p, &t0) in self.points.iter_mut().zip(&self.node_t) {
            p.level = node_level(t0, anim.ft, secs, mood.global, marks) * sc;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::Anim;
    use crate::color::Colors;
    use crate::figures;

    /// The app's per-frame order is update → push_hud_step → upload. Nothing in
    /// that loop may accumulate.
    #[test]
    fn the_step_table_does_not_grow_frame_over_frame() {
        let cons = figures::build(3);
        let n = cons.steps.len();
        let mut scene = Scene::new(cons, 1.0);
        let anim = Anim::default();
        let pal = Colors::default().palette();

        for frame in 0..120 {
            scene.update(&anim, ScaffoldVis::Full, MarkMode::Fade, pal, frame as f32 / 60.0);
            assert_eq!(
                scene.steps.len(),
                n,
                "step table drifted to {} by frame {frame}",
                scene.steps.len()
            );
            // What `State::push_hud_step` appends.
            scene.steps.push(StepGpu::default());
            scene.steps.push(StepGpu::default());
        }
    }
}
