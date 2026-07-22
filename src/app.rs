use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use anyhow::Result;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::anim::{Anim, MarkMode, ScaffoldVis};
use crate::color::Colors;
use crate::figures::{self, Knob};
use crate::params::{Params, RATIO, RINGS, SKIP, SYMMETRY};
use crate::render::gpu::Gpu;
use crate::render::pipeline::{FrameInputs, PostUniforms, Renderer, SceneUniforms};
use crate::hud::{self, Act, Mode, Region, Token};
use crate::infinite::{Infinite, Kind};
use crate::scene::Scene;
use crate::snow::Snow;
use crate::tess::{StepGpu, Tess, View};

/// Slot count: the finite catalogue, then the two endless modes.
pub const SLOTS: usize = figures::FIGURES.len() + 2;

pub fn slot_name(i: usize) -> &'static str {
    match i.checked_sub(figures::FIGURES.len()) {
        None => figures::FIGURES[i].name,
        Some(0) => Kind::Lattice.name(),
        _ => Kind::Gasket.name(),
    }
}

pub struct State {
    pub scene: Scene,
    /// Present only in the endless modes.
    pub inf: Option<Infinite>,
    pub anim: Anim,
    pub fig: usize,
    pub params: Params,
    pub size: (u32, u32),
    pub view: View,
    /// Wall clock. Drives shimmer, and deliberately keeps running when paused —
    /// pausing should stop the construction, not the light.
    pub t: f32,
    pub shimmer: f32,
    pub bloom: f32,
    pub emissive: f32,
    pub scaffold: ScaffoldVis,
    pub marks: MarkMode,
    pub colors: Colors,
    pub snow: Snow,
    pub seed: u64,
    pub hud_mode: Mode,
    pub hud_tess: Tess,
    /// Last rendered readout, so the HUD only re-tessellates when it changes.
    hud_text: Vec<Vec<Token>>,
    /// Step slot the HUD geometry currently points at.
    hud_slot: u32,
    pub hud_dirty: bool,
    pub regions: Vec<Region>,
    pub cursor: (f32, f32),
    pub hover: Option<Act>,
}

impl State {
    pub fn new(size: (u32, u32), fig: usize) -> Self {
        let view = View::fit(size.0, size.1);
        let params = Params::default();
        Self {
            scene: Scene::new(figures::build_with(fig, &params), view.scale),
            inf: None,
            view,
            anim: Anim::default(),
            fig,
            params,
            size,
            t: 0.0,
            shimmer: 0.35,
            bloom: 1.0,
            emissive: 1.0,
            scaffold: ScaffoldVis::Full,
            marks: MarkMode::Fade,
            colors: Colors::default(),
            snow: Snow::new(),
            seed: 0,
            // Opens clean — the figure is the interface. H brings the readout.
            hud_mode: Mode::Off,
            hud_tess: Tess::default(),
            hud_text: Vec::new(),
            hud_slot: u32::MAX,
            hud_dirty: true,
            regions: Vec::new(),
            cursor: (-1.0, -1.0),
            hover: None,
        }
    }

    pub fn load(&mut self, fig: usize) {
        self.fig = fig % SLOTS;
        self.rebuild();
    }

    pub fn is_infinite(&self) -> bool {
        self.fig >= figures::FIGURES.len()
    }

    /// Re-run the construction from the current seed settings. Because every
    /// centre is solved rather than stored, a new symmetry order produces a
    /// genuinely correct figure of that order — not a stretched hexagon.
    pub fn rebuild(&mut self) {
        self.params = self.params.clamped();
        self.hud_dirty = true;
        if let Some(k) = match self.fig.checked_sub(figures::FIGURES.len()) {
            None => None,
            Some(0) => Some(Kind::Lattice),
            _ => Some(Kind::Gasket),
        } {
            let mut inf = Infinite::new(k, &self.params, self.t);
            self.view = View::fit(self.size.0, self.size.1);
            inf.retessellate(&mut self.scene.tess, &self.view, self.size);
            inf.step_states(self.t, &self.colors.palette(), &mut self.scene.steps);
            // Endless figures have no node sparkle: every generation would add
            // hundreds, and they are not the point once nothing ever finishes.
            self.scene.points.clear();
            self.inf = Some(inf);
            tracing::info!(figure = slot_name(self.fig), "endless mode");
            return;
        }
        self.inf = None;
        self.scene = Scene::new(figures::build_with(self.fig, &self.params), self.view.scale);
        self.anim.restart();
        tracing::info!(
            figure = self.scene.cons.name,
            params = %self.params.summary(),
            steps = self.scene.cons.steps.len(),
            "built"
        );
    }

    /// Does this knob actually change the current figure?
    fn has(&self, k: Knob) -> bool {
        if self.is_infinite() {
            return matches!(k, Knob::Symmetry | Knob::Twist);
        }
        figures::FIGURES[self.fig].knobs.contains(&k)
    }

    /// Resize only moves the camera — the vertex buffers are in construction
    /// space, so nothing needs re-tessellating.
    pub fn resize(&mut self, size: (u32, u32)) {
        self.size = size;
        self.hud_dirty = true;
        let snow_mode = self.snow.mode;
        self.snow.set_mode(snow_mode, size);
        self.view = View::fit(size.0, size.1);
        if self.scene.tess.needs_retess(self.view.scale) {
            self.scene.retessellate(self.view.scale);
        }
    }

    /// Advance time. Returns true if the static geometry needs re-uploading.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.t += dt;
        self.snow.update(dt, self.t, self.size);
        if self.inf.is_some() {
            let (mut view, size, t) = (self.view, self.size, self.t);
            let mut inf = self.inf.take().unwrap();
            let changed = inf.tick(t, &mut view, size, dt);
            if changed {
                inf.retessellate(&mut self.scene.tess, &view, size);
            }
            inf.step_states(t, &self.colors.palette(), &mut self.scene.steps);
            self.view = view;
            self.inf = Some(inf);
            return changed;
        }
        let mut reupload = false;
        if self.anim.tick(dt, self.scene.cons.draw_seconds()) {
            // Autoplay cycles the finite catalogue only. The endless modes never
            // finish, so wandering into one would end the rotation for good.
            self.load((self.fig + 1) % figures::FIGURES.len());
            reupload = true;
        }
        self.scene.update(&self.anim, self.scaffold, self.marks, self.colors.palette(), self.t);
        reupload
    }

    /// Append the HUD's two step slots: normal text, then the brighter one the
    /// hovered token's strokes are assigned to.
    pub fn push_hud_step(&mut self) {
        let accent = self.colors.palette().accent;
        for glow in [0.42, 0.95] {
            self.scene.steps.push(StepGpu {
                head: 1.05,
                glow,
                pen: 0.0,
                seed: 0.0,
                color: accent,
                _pad: 0.0,
            });
        }
    }

    pub fn frame_inputs(&self) -> FrameInputs {
        let res = [self.size.0 as f32, self.size.1 as f32];
        // Auto-exposure by ink coverage, so a dense seed stays legible instead
        // of saturating into a solid disc.
        let exposure = match self.inf.as_ref() {
            Some(inf) => {
                let n = inf.visible_count() as f32;
                (90.0 / n.max(90.0)).powf(0.45)
            }
            None => self.scene.cons.exposure(),
        };
        FrameInputs {
            scene: SceneUniforms {
                res,
                emissive: self.emissive * exposure,
                time: self.t,
                tint: self.colors.palette().accent,
                shimmer: self.shimmer,
                view_center: [self.view.center.x, self.view.center.y],
                view_scale: self.view.scale,
                _pad: 0.0,
            },
            hud: SceneUniforms {
                res,
                emissive: 1.0,
                time: self.t,
                tint: self.colors.palette().accent,
                // No shimmer on text: a glyph stroke is ~7 px, so it would take
                // roughly one noise sample and flicker as a whole unit.
                shimmer: 0.0,
                // Identity camera — construction coords are already pixels.
                view_center: [0.0, 0.0],
                view_scale: 1.0,
                _pad: 0.0,
            },
            post: PostUniforms { res, bright: 1.0, vignette: 0.28 },
            bloom_gain: self.bloom,
        }
    }

    /// The readout, as interactive tokens. Only the knobs that actually affect
    /// the current figure are listed — offering `rings` on the Vesica would be
    /// noise — and every value is a hit region the mouse can adjust.
    fn hud_lines(&self) -> Vec<Vec<Token>> {
        if !self.hud_mode.is_on() {
            return Vec::new();
        }
        let mut out = Vec::new();
        out.push(vec![
            Token::hot(slot_name(self.fig).to_uppercase(), Act::Figure),
            Token::plain(format!("{}/{}", self.fig + 1, SLOTS)),
        ]);

        if let Some(inf) = self.inf.as_ref() {
            out.push(vec![Token::plain(format!(
                "GEN {}   CIRCLES {}   VISIBLE {}",
                inf.generation,
                inf.steps.len(),
                inf.visible_count()
            ))]);
            // Quantized: the raw value changes every frame and would force a
            // HUD re-tessellation per frame for no legible difference.
            let z = self.view.scale / 200.0;
            let z = if z < 10.0 { (z * 2.0).round() / 2.0 } else { (z / 10.0).round() * 10.0 };
            out.push(vec![Token::plain(format!("ZOOM {z}X"))]);
        } else {
            let mut knobs: Vec<Token> = Vec::new();
            if self.has(Knob::Symmetry) {
                knobs.push(Token::hot(format!("SYM {}", self.params.symmetry), Act::Sym));
            }
            if self.has(Knob::Rings) {
                knobs.push(Token::hot(format!("RINGS {}", self.params.rings), Act::Rings));
            }
            if self.has(Knob::Ratio) {
                knobs.push(Token::hot(format!("RATIO {:.2}", self.params.ratio), Act::Ratio));
            }
            if self.has(Knob::Skip) {
                knobs.push(Token::hot(format!("SKIP {}", self.params.skip), Act::Skip));
            }
            if self.has(Knob::Twist) {
                knobs.push(Token::hot(format!("TWIST {:+.2}", self.params.twist), Act::Twist));
            }
            knobs.push(Token::hot(format!("SEED {}", self.seed), Act::Seed));
            out.push(knobs);
            out.push(vec![
                Token::plain(format!("DRAW {:>3}%", (self.anim.ft * 100.0).round() as i32)),
                Token::hot(format!("SPEED {:.2}X", self.anim.speed), Act::Speed),
                Token::hot(
                    if self.anim.paused { "PAUSED" } else { "PLAYING" },
                    Act::Pause,
                ),
            ]);
        }

        let mut fx = vec![
            Token::hot(self.colors.name(), Act::Palette),
            Token::hot(format!("MARKS {}", self.marks.name()), Act::Marks),
            Token::hot(format!("SNOW {}", self.snow.mode.name()), Act::SnowMode),
        ];
        if self.snow.is_active() {
            fx.push(Token::hot(format!("GRAV {:.2}", self.snow.gravity), Act::Gravity));
            fx.push(Token::hot(format!("DRIFT {:.2}", self.snow.drift), Act::Drift));
        }
        out.push(fx);

        if self.hud_mode == Mode::Keys {
            out.push(Vec::new());
            out.extend(hud::KEYS.iter().map(|k| vec![Token::plain(*k)]));
        }
        out
    }

    /// Rebuild the HUD geometry if the readout changed.
    pub fn refresh_hud(&mut self) -> bool {
        // The HUD borrows one step slot, appended after the figure's own. In
        // endless mode the visible count changes every generation, so the slot
        // index moves — the geometry has to be rebuilt when it does, or the
        // text would read some figure stroke's animation state instead.
        let slot = self.scene.steps.len() as u32;
        let lines = self.hud_lines();
        if lines == self.hud_text && slot == self.hud_slot && !self.hud_dirty {
            return false;
        }
        self.hud_text = lines;
        self.hud_slot = slot;
        self.hud_dirty = false;
        hud::build(
            &mut self.hud_tess,
            &self.hud_text,
            self.size,
            1.6,
            slot,
            self.hover,
            &mut self.regions,
        );
        true
    }

    /// Mouse motion: retarget the hover highlight.
    pub fn set_cursor(&mut self, x: f32, y: f32) {
        self.cursor = (x, y);
        let hover = if self.hud_mode.is_on() { hud::hit(&self.regions, x, y) } else { None };
        if hover != self.hover {
            self.hover = hover;
            self.hud_dirty = true;
        }
    }

    /// Apply a mouse action. `dir` is +1 for left click / wheel up, -1 for
    /// right click / wheel down. Returns true when geometry must re-upload.
    pub fn adjust(&mut self, act: Act, dir: i32) -> bool {
        let up = dir > 0;
        let mut changed = false;
        match act {
            Act::Figure => {
                let n = if up { self.fig + 1 } else { self.fig + SLOTS - 1 };
                self.load(n % SLOTS);
                return true;
            }
            Act::Sym => {
                self.params.symmetry =
                    if up { self.params.symmetry + 1 } else { self.params.symmetry.saturating_sub(1) };
                changed = true;
            }
            Act::Rings => {
                self.params.rings =
                    if up { self.params.rings + 1 } else { self.params.rings.saturating_sub(1) };
                changed = true;
            }
            Act::Ratio => {
                self.params.ratio += if up { 0.05 } else { -0.05 };
                changed = true;
            }
            Act::Skip => {
                self.params.skip =
                    if up { self.params.skip + 1 } else { self.params.skip.saturating_sub(1) };
                changed = true;
            }
            Act::Twist => {
                self.params.twist += if up { 1.0 / 24.0 } else { -1.0 / 24.0 };
                changed = true;
            }
            Act::Seed => {
                self.seed = if up { self.seed + 1 } else { self.seed.saturating_sub(1) };
                self.params = if self.seed == 0 {
                    crate::params::Params::default()
                } else {
                    Params::from_seed(self.seed)
                };
                changed = true;
            }
            Act::Speed => {
                self.anim.speed = if up {
                    (self.anim.speed * 1.25).min(6.0)
                } else {
                    (self.anim.speed / 1.25).max(0.15)
                };
            }
            Act::Pause => self.anim.paused = !self.anim.paused,
            Act::Palette => {
                if up {
                    self.colors.cycle()
                } else {
                    self.colors.cycle_back()
                }
            }
            Act::Marks => self.marks = self.marks.next(),
            Act::SnowMode => {
                let m = if up { self.snow.mode.next() } else { self.snow.mode.prev() };
                self.snow.set_mode(m, self.size);
            }
            Act::Gravity => {
                self.snow.gravity =
                    (self.snow.gravity + if up { 0.25 } else { -0.25 }).clamp(0.0, 2.5);
            }
            Act::Drift => {
                self.snow.drift =
                    (self.snow.drift + if up { 0.25 } else { -0.25 }).clamp(0.0, 2.5);
            }
        }
        if changed {
            self.rebuild();
        }
        self.hud_dirty = true;
        changed
    }

    /// A click at the current cursor. Returns true when geometry changed.
    pub fn click(&mut self, dir: i32) -> bool {
        match self.hover {
            Some(act) => self.adjust(act, dir),
            None => false,
        }
    }

    /// Step one construction group forward or back.
    fn step_group(&mut self, dir: i32) {
        let ft = self.anim.ft;
        let mut marks: Vec<f32> = self.scene.cons.steps.iter().map(|s| s.t0).collect();
        marks.push(1.0);
        marks.sort_by(|a, b| a.partial_cmp(b).unwrap());
        marks.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
        let target = if dir > 0 {
            marks.iter().copied().find(|&m| m > ft + 1e-4).unwrap_or(1.0)
        } else {
            marks.iter().rev().copied().find(|&m| m < ft - 1e-4).unwrap_or(0.0)
        };
        self.anim.seek(target);
    }

    /// Returns true if the figure changed (geometry needs re-upload).
    pub fn key(&mut self, code: KeyCode) -> bool {
        let mut changed = false;
        match code {
            KeyCode::Space => self.anim.paused = !self.anim.paused,
            KeyCode::ArrowRight => self.step_group(1),
            KeyCode::ArrowLeft => self.step_group(-1),
            KeyCode::ArrowUp => self.anim.speed = (self.anim.speed * 1.25).min(6.0),
            KeyCode::ArrowDown => self.anim.speed = (self.anim.speed / 1.25).max(0.15),
            KeyCode::KeyN => {
                self.load(self.fig + 1);
                return true;
            }
            KeyCode::KeyP => {
                self.load((self.fig + SLOTS - 1) % SLOTS);
                return true;
            }
            KeyCode::KeyR => self.anim.restart(),
            KeyCode::Home => self.anim.seek(0.0),
            KeyCode::End => self.anim.seek(1.0),
            KeyCode::KeyA => {
                self.anim.autoplay = !self.anim.autoplay;
                self.anim.paused = false;
            }
            KeyCode::KeyS => self.scaffold = self.scaffold.next(),
            KeyCode::BracketLeft => self.shimmer = (self.shimmer - 0.1).max(0.0),
            KeyCode::BracketRight => self.shimmer = (self.shimmer + 0.1).min(1.0),
            KeyCode::Minus => self.bloom = (self.bloom - 0.25).max(0.0),
            KeyCode::Equal => self.bloom = (self.bloom + 0.25).min(4.0),

            // ---- seed settings: every one of these re-solves the figure ----
            KeyCode::Comma if self.has(Knob::Symmetry) => {
                self.params.symmetry = self.params.symmetry.saturating_sub(1).max(SYMMETRY.0);
                changed = true;
            }
            KeyCode::Period if self.has(Knob::Symmetry) => {
                self.params.symmetry = (self.params.symmetry + 1).min(SYMMETRY.1);
                changed = true;
            }
            KeyCode::Semicolon if self.has(Knob::Rings) => {
                self.params.rings = self.params.rings.saturating_sub(1).max(RINGS.0);
                changed = true;
            }
            KeyCode::Quote if self.has(Knob::Rings) => {
                self.params.rings = (self.params.rings + 1).min(RINGS.1);
                changed = true;
            }
            KeyCode::Digit9 if self.has(Knob::Ratio) => {
                self.params.ratio = (self.params.ratio - 0.05).max(RATIO.0);
                changed = true;
            }
            KeyCode::Digit0 if self.has(Knob::Ratio) => {
                self.params.ratio = (self.params.ratio + 0.05).min(RATIO.1);
                changed = true;
            }
            KeyCode::KeyK if self.has(Knob::Skip) => {
                self.params.skip = self.params.skip.saturating_sub(1).max(SKIP.0);
                changed = true;
            }
            KeyCode::KeyL if self.has(Knob::Skip) => {
                self.params.skip = (self.params.skip + 1).min(SKIP.1);
                changed = true;
            }
            KeyCode::KeyO if self.has(Knob::Twist) => {
                self.params.twist -= 1.0 / 24.0;
                changed = true;
            }
            KeyCode::KeyI if self.has(Knob::Twist) => {
                self.params.twist += 1.0 / 24.0;
                changed = true;
            }
            KeyCode::KeyG => {
                // Seed bank: a whole parameter set from one number, so a figure
                // you liked can be returned to by seed alone.
                self.seed = self.seed.wrapping_add(1);
                self.params = Params::from_seed(self.seed);
                changed = true;
            }
            KeyCode::KeyC => {
                self.params = Params::default();
                self.seed = 0;
                changed = true;
            }
            KeyCode::KeyX => {
                // Jump straight to the endless modes and cycle between them.
                let base = figures::FIGURES.len();
                self.load(if self.is_infinite() { base + (self.fig - base + 1) % 2 } else { base });
                return true;
            }
            KeyCode::KeyW => {
                let mode = self.snow.mode.next();
                self.snow.set_mode(mode, self.size);
            }
            KeyCode::KeyM => self.marks = self.marks.next(),
            KeyCode::KeyV => self.colors.cycle(),
            KeyCode::KeyT => self.colors.shift_hue(-15.0),
            KeyCode::KeyY => self.colors.shift_hue(15.0),
            KeyCode::KeyU => self.colors.step_sat(),
            KeyCode::KeyH => {
                self.hud_mode = self.hud_mode.next();
                self.hud_dirty = true;
            }
            KeyCode::KeyD if !self.is_infinite() => self.scene.cons.dump(),
            _ => return false,
        }
        if changed {
            self.rebuild();
        }
        tracing::info!(
            figure = slot_name(self.fig),
            ft = self.anim.ft,
            phase = ?self.anim.phase,
            speed = self.anim.speed,
            paused = self.anim.paused,
            params = %self.params.summary(),
            "state"
        );
        changed
    }
}

struct Ctx {
    renderer: Renderer,
    gpu: Gpu,
    window: Arc<Window>,
}

#[derive(Default)]
pub struct App {
    ctx: Option<Ctx>,
    state: Option<State>,
    last: Option<Instant>,
    /// Wasm: the window exists before the GPU does, because adapter/device
    /// request is async in the browser. It parks here until `pending_gpu`
    /// resolves.
    #[cfg(target_arch = "wasm32")]
    boot_window: Option<Arc<Window>>,
    #[cfg(target_arch = "wasm32")]
    pending_gpu: std::rc::Rc<std::cell::RefCell<Option<Result<crate::render::gpu::Gpu>>>>,
}

impl App {
    fn redraw(&mut self) -> Result<()> {
        let (Some(ctx), Some(state)) = (self.ctx.as_mut(), self.state.as_mut()) else {
            return Ok(());
        };

        let now = Instant::now();
        let dt = self.last.map_or(1.0 / 60.0, |l| (now - l).as_secs_f32().min(0.1));
        self.last = Some(now);

        if state.tick(dt) {
            ctx.renderer.upload_strokes(&state.scene.tess);
        }
        if state.refresh_hud() {
            ctx.renderer.upload_hud(&state.hud_tess);
        }
        state.push_hud_step();
        ctx.renderer.upload_steps(&state.scene.steps);
        ctx.renderer.upload_points(&state.scene.points);
        if state.snow.is_active() {
            let mut parts = Vec::new();
            state.snow.instances(state.t, &mut parts);
            ctx.renderer.upload_snow(&parts);
        } else {
            ctx.renderer.upload_snow(&[]);
        }

        use wgpu::CurrentSurfaceTexture as Cst;
        let frame = match ctx.gpu.surface.get_current_texture() {
            Cst::Success(t) | Cst::Suboptimal(t) => t,
            Cst::Outdated | Cst::Lost => {
                let sz = ctx.window.inner_size();
                ctx.gpu.resize(sz.width, sz.height);
                ctx.renderer.resize((sz.width.max(1), sz.height.max(1)));
                return Ok(());
            }
            _ => return Ok(()),
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = ctx
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });

        ctx.renderer.encode(&mut encoder, &state.frame_inputs());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("surface"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            ctx.renderer.blit(&mut pass);
        }
        ctx.gpu.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}

impl App {
    /// Build the renderer and initial state once a surface exists.
    fn install(&mut self, window: Arc<Window>, gpu: Gpu) {
        let size = (gpu.config.width, gpu.config.height);
        let mut renderer =
            Renderer::new(gpu.device.clone(), gpu.queue.clone(), size, gpu.config.format);
        renderer.set_surface_srgb(gpu.config.format.is_srgb());

        let state = State::new(size, 0);
        renderer.upload_strokes(&state.scene.tess);
        tracing::info!(?size, format = ?gpu.config.format, figure = state.scene.cons.name, "ready");

        self.state = Some(state);
        self.ctx = Some(Ctx { renderer, gpu, window });
        #[cfg(target_arch = "wasm32")]
        crate::web::hide_loading();
    }

    /// Wasm: adopt the GPU once its async request resolves, and keep the canvas
    /// backing buffer matched to its CSS box.
    #[cfg(target_arch = "wasm32")]
    fn poll_web(&mut self) {
        if self.ctx.is_none() {
            let ready = self.pending_gpu.borrow_mut().take();
            let (Some(result), Some(window)) = (ready, self.boot_window.take()) else {
                return;
            };
            match result {
                Ok(gpu) => self.install(window, gpu),
                Err(e) => {
                    crate::web::show_fatal(&format!(
                        "WebGPU unavailable — this needs a browser with WebGPU enabled.\n\n{e:#}"
                    ));
                    return;
                }
            }
        }

        // Reconcile the surface with the canvas's true CSS box every frame.
        //
        // winit's `Resized` can't be relied on here. Its `inner_size()` is a
        // value cached from a ResizeObserver (it starts at 0×0), and
        // `request_inner_size` only rewrites the canvas's CSS width/height — for
        // a 100vw/100vh canvas that computes to the *same* box, so the observer
        // never fires and no `Resized` is emitted. Meanwhile the surface was
        // configured from whatever `inner_size()` read at boot: if the async
        // device request beat the observer's first callback, that was 0×0,
        // clamped to a 1×1 surface. Stretched over the viewport, a 1×1 surface
        // reads as a blank page. So drive the reconfigure ourselves, comparing
        // against the surface config rather than winit's cache.
        if let (Some(ctx), Some(state), Some((w, h))) =
            (self.ctx.as_mut(), self.state.as_mut(), crate::web::canvas_size())
        {
            if ctx.gpu.config.width != w || ctx.gpu.config.height != h {
                // Keep winit's canvas backing buffer in step so it doesn't fight
                // wgpu over the canvas dimensions, then reconfigure directly.
                let _ = ctx.window.request_inner_size(winit::dpi::PhysicalSize::new(w, h));
                ctx.gpu.resize(w, h);
                ctx.renderer.resize((w, h));
                state.resize((w, h));
                ctx.renderer.upload_strokes(&state.scene.tess);
            }
        }
    }
}

impl ApplicationHandler for App {
    #[cfg(not(target_arch = "wasm32"))]
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.ctx.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("divine geometry")
            .with_inner_size(winit::dpi::LogicalSize::new(1100.0, 1100.0));
        let window = Arc::new(el.create_window(attrs).expect("create window"));
        let gpu = Gpu::new(window.clone()).expect("gpu init");
        self.install(window, gpu);
    }

    /// Wasm: the window is created synchronously and attached to the page's
    /// canvas, but the GPU arrives later — so nothing else can be built yet.
    #[cfg(target_arch = "wasm32")]
    fn resumed(&mut self, el: &ActiveEventLoop) {
        use winit::platform::web::WindowAttributesExtWebSys;
        if self.ctx.is_some() || self.boot_window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("divine geometry")
            .with_canvas(crate::web::get_canvas());
        let window = Arc::new(el.create_window(attrs).expect("create window"));
        if let Some((w, h)) = crate::web::canvas_size() {
            let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(w, h));
        }
        let pending = self.pending_gpu.clone();
        let w2 = window.clone();
        wasm_bindgen_futures::spawn_local(async move {
            *pending.borrow_mut() = Some(Gpu::new_async(w2).await);
        });
        window.request_redraw();
        self.boot_window = Some(window);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(sz) => {
                if let (Some(ctx), Some(state)) = (self.ctx.as_mut(), self.state.as_mut()) {
                    ctx.gpu.resize(sz.width, sz.height);
                    let size = (ctx.gpu.config.width, ctx.gpu.config.height);
                    ctx.renderer.resize(size);
                    state.resize(size);
                    ctx.renderer.upload_strokes(&state.scene.tess);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(state) = self.state.as_mut() {
                    state.set_cursor(position.x as f32, position.y as f32);
                }
            }
            WindowEvent::MouseInput { state: btn_state, button, .. } => {
                if btn_state != winit::event::ElementState::Pressed {
                    return;
                }
                let dir = match button {
                    winit::event::MouseButton::Left => 1,
                    winit::event::MouseButton::Right => -1,
                    _ => return,
                };
                if let (Some(ctx), Some(state)) = (self.ctx.as_mut(), self.state.as_mut()) {
                    if state.click(dir) {
                        ctx.renderer.upload_strokes(&state.scene.tess);
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                if dy.abs() < f32::EPSILON {
                    return;
                }
                if let (Some(ctx), Some(state)) = (self.ctx.as_mut(), self.state.as_mut()) {
                    if state.click(if dy > 0.0 { 1 } else { -1 }) {
                        ctx.renderer.upload_strokes(&state.scene.tess);
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if !event.state.is_pressed() {
                    return;
                }
                if let PhysicalKey::Code(code) = event.physical_key {
                    if code == KeyCode::Escape {
                        el.exit();
                        return;
                    }
                    if code == KeyCode::KeyF {
                        if let Some(ctx) = self.ctx.as_ref() {
                            let cur = ctx.window.fullscreen();
                            ctx.window.set_fullscreen(match cur {
                                None => Some(winit::window::Fullscreen::Borderless(None)),
                                Some(_) => None,
                            });
                        }
                        return;
                    }
                    if let (Some(ctx), Some(state)) = (self.ctx.as_mut(), self.state.as_mut()) {
                        if state.key(code) {
                            ctx.renderer.upload_strokes(&state.scene.tess);
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.redraw() {
                    tracing::error!(?e, "redraw failed");
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        #[cfg(target_arch = "wasm32")]
        {
            self.poll_web();
            if let Some(w) = self.boot_window.as_ref() {
                w.request_redraw();
            }
        }
        if let Some(ctx) = self.ctx.as_ref() {
            ctx.window.request_redraw();
        }
    }
}

/// Wasm entry: winit's web event loop never returns, so this hands off.
#[cfg(target_arch = "wasm32")]
pub fn run_web() -> Result<()> {
    use winit::platform::web::EventLoopExtWebSys;
    let el = winit::event_loop::EventLoop::new()?;
    el.set_control_flow(winit::event_loop::ControlFlow::Poll);
    el.spawn_app(App::default());
    Ok(())
}
