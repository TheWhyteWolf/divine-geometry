//! Offscreen still rendering.
//!
//! The compositor here doesn't support screen capture, but more importantly a
//! still at an exact figure-time is a far better verification instrument than a
//! photo of a moving window.

use anyhow::Result;

use crate::app::State;
use crate::figures;
use crate::render::gpu::Gpu;
use crate::render::pipeline::Renderer;
use crate::render::readback;

pub struct ShotOpts {
    pub size: (u32, u32),
    pub figure: usize,
    /// Figure time, 0..1.
    pub ft: f32,
    pub time: f32,
    pub shimmer: f32,
    pub bloom: f32,
    /// 0 = mid-draw, 1 = fully settled.
    pub settled: f32,
    pub params: crate::params::Params,
    /// Seconds of endless-mode growth to simulate before capturing.
    pub grow: f32,
    pub hud: crate::hud::Mode,
    /// Palette preset index (cycles applied from default).
    pub palette: u32,
    pub snow: crate::snow::SnowMode,
    pub marks: crate::anim::MarkMode,
}

impl Default for ShotOpts {
    fn default() -> Self {
        Self {
            size: (900, 900),
            figure: 0,
            ft: 0.6,
            time: 0.0,
            shimmer: 0.35,
            bloom: 1.0,
            settled: 0.0,
            params: crate::params::Params::default(),
            grow: 0.0,
            hud: crate::hud::Mode::Off,
            palette: 0,
            snow: crate::snow::SnowMode::Off,
            marks: crate::anim::MarkMode::Fade,
        }
    }
}

pub fn render_to_png(path: &std::path::Path, opts: &ShotOpts) -> Result<()> {
    let (device, queue) = Gpu::headless()?;
    let mut renderer = Renderer::new(
        device.clone(),
        queue.clone(),
        opts.size,
        wgpu::TextureFormat::Rgba8Unorm,
    );
    renderer.set_surface_srgb(false);

    let mut state = State::new(opts.size, opts.figure);
    state.params = opts.params;
    state.hud_mode = opts.hud;
    for _ in 0..opts.palette {
        state.colors.cycle();
    }
    state.marks = opts.marks;
    if opts.snow != crate::snow::SnowMode::Off {
        state.snow.set_mode(opts.snow, opts.size);
        // Run the field forward so the still catches motes mid-fall, some of
        // them mid-flash.
        let dt = 1.0 / 60.0;
        for i in 0..600 {
            state.snow.update(dt, i as f32 * dt, opts.size);
        }
        state.t = 10.0;
    }
    state.rebuild();
    state.t = opts.time;
    state.shimmer = opts.shimmer;
    state.bloom = opts.bloom;
    if state.is_infinite() {
        // Run the growth forward at a fixed step so the capture is reproducible.
        let dt = 1.0 / 30.0;
        let n = (opts.grow.max(0.1) / dt) as usize;
        for _ in 0..n {
            state.tick(dt);
        }
        if let Some(inf) = state.inf.as_ref() {
            tracing::info!(
                gen = inf.generation,
                steps = inf.steps.len(),
                visible = inf.visible_count(),
                frontier = inf.frontier,
                focus = ?inf.focus,
                scale = state.view.scale,
                verts = state.scene.tess.verts.len(),
                "endless state"
            );
        }
        state.refresh_hud();
        renderer.upload_hud(&state.hud_tess);
        state.push_hud_step();
        renderer.upload_strokes(&state.scene.tess);
        renderer.upload_steps(&state.scene.steps);
        renderer.upload_points(&state.scene.points);
        upload_snow(&mut renderer, &state);
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("shot") });
        renderer.encode(&mut encoder, &state.frame_inputs());
        queue.submit([encoder.finish()]);
        let (w, h, rgba) = readback::read_texture(&device, &queue, renderer.final_tex())?;
        readback::save_png(path, w, h, &rgba)?;
        tracing::info!(path = %path.display(), grow = opts.grow, "wrote endless still");
        return Ok(());
    }
    state.anim.seek(opts.ft);
    if opts.settled > 0.0 {
        // Drop into Settle and run the phase forward to the requested point.
        state.anim.phase = crate::anim::Phase::Settle;
        state.anim.phase_t = opts.settled * 1.5;
    }
    state.scene.update(
        &state.anim,
        state.scaffold,
        state.marks,
        state.colors.palette(),
        state.t,
    );

    state.refresh_hud();
    renderer.upload_hud(&state.hud_tess);
    state.push_hud_step();
    renderer.upload_strokes(&state.scene.tess);
    renderer.upload_steps(&state.scene.steps);
    renderer.upload_points(&state.scene.points);
    upload_snow(&mut renderer, &state);

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("shot") });
    renderer.encode(&mut encoder, &state.frame_inputs());
    queue.submit([encoder.finish()]);

    let (w, h, rgba) = readback::read_texture(&device, &queue, renderer.final_tex())?;
    readback::save_png(path, w, h, &rgba)?;
    tracing::info!(
        path = %path.display(),
        figure = figures::FIGURES[opts.figure % figures::FIGURES.len()].name,
        ft = opts.ft,
        "wrote still"
    );
    Ok(())
}

fn upload_snow(renderer: &mut Renderer, state: &State) {
    if state.snow.is_active() {
        let mut parts = Vec::new();
        state.snow.instances(state.t, &mut parts);
        renderer.upload_snow(&parts);
    }
}
