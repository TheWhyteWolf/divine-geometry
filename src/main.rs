use anyhow::Result;
use divine::params::Params;
use divine::{anim, app, figures, hud, shot, snow};
use winit::event_loop::{ControlFlow, EventLoop};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "divine=info,wgpu=warn".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    let num = |name: &str, default: f32| -> f32 {
        args.iter()
            .position(|a| a == name)
            .and_then(|j| args.get(j + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };

    // Build every figure and assert its invariants — runs without a GPU.
    if args.iter().any(|a| a == "--verify") {
        for (i, def) in figures::FIGURES.iter().enumerate() {
            let c = figures::build(i);
            println!(
                "{:<18} {:>4} steps  {:>3} groups  {:>3} nodes  {:>5.1}s",
                def.name,
                c.steps.len(),
                c.n_groups,
                c.nodes.len(),
                c.draw_seconds()
            );
        }
        return Ok(());
    }

    // `--shot <path>` renders one offscreen frame and exits.
    if let Some(i) = args.iter().position(|a| a == "--shot") {
        let path = args.get(i + 1).cloned().unwrap_or_else(|| "shot.png".into());
        let n = num("--size", 900.0) as u32;
        let seed = num("--seed", 0.0) as u64;
        let mut params = if seed > 0 { Params::from_seed(seed) } else { Params::default() };
        params.symmetry = num("--sym", params.symmetry as f32) as u32;
        params.rings = num("--rings", params.rings as f32) as u32;
        params.ratio = num("--ratio", params.ratio);
        params.skip = num("--skip", params.skip as f32) as u32;
        params.twist = num("--twist", params.twist);
        let opts = shot::ShotOpts {
            size: (n, n),
            figure: num("--figure", 0.0) as usize,
            ft: num("--ft", 0.6),
            time: num("--time", 0.0),
            shimmer: num("--shimmer", 0.35),
            bloom: num("--bloom", 1.0),
            settled: num("--settled", 0.0),
            params: params.clamped(),
            grow: num("--grow", 12.0),
            hud: match num("--hud", 0.0) as u32 {
                0 => hud::Mode::Off,
                1 => hud::Mode::Status,
                _ => hud::Mode::Keys,
            },
            palette: num("--palette", 0.0) as u32,
            snow: match num("--snow", 0.0) as u32 {
                0 => snow::SnowMode::Off,
                1 => snow::SnowMode::Light,
                _ => snow::SnowMode::Dense,
            },
            marks: match num("--marks", 0.0) as u32 {
                0 => anim::MarkMode::Fade,
                1 => anim::MarkMode::Keep,
                _ => anim::MarkMode::Off,
            },
        };
        return shot::render_to_png(std::path::Path::new(&path), &opts);
    }

    let el = EventLoop::new()?;
    el.set_control_flow(ControlFlow::Poll);
    let mut app = app::App::default();
    el.run_app(&mut app)?;
    Ok(())
}
