use std::sync::Arc;

use anyhow::{Context, Result};
use winit::window::Window;

/// Owns the wgpu instance/surface/device and the swapchain configuration.
pub struct Gpu {
    pub surface: wgpu::Surface<'static>,
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub config: wgpu::SurfaceConfiguration,
}

impl Gpu {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(window: Arc<Window>) -> Result<Self> {
        pollster::block_on(Self::new_async(window))
    }

    pub async fn new_async(window: Arc<Window>) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance.create_surface(window.clone())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .context("no suitable GPU adapter")?;
        tracing::info!(
            name = %adapter.get_info().name,
            backend = ?adapter.get_info().backend,
            "adapter"
        );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("divine"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            })
            .await
            .context("device request failed")?;
        tracing::info!("device ready");

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context("surface unsupported by adapter")?;
        // Prefer a non-sRGB surface format so our shader-side sRGB encode passes
        // through 1:1. If only sRGB formats are offered, the blit shader decodes
        // so the hardware re-encode is an identity round trip.
        let caps = surface.get_capabilities(&adapter);
        if let Some(fmt) = caps.formats.iter().copied().find(|f| {
            !f.is_srgb()
                && matches!(
                    f,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
                )
        }) {
            config.format = fmt;
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        let device = Arc::new(device);
        let queue = Arc::new(queue);
        surface.configure(&device, &config);
        Ok(Self { surface, device, queue, config })
    }

    /// Device + queue with no surface — offscreen stills for verification.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn headless() -> Result<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> {
        pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                })
                .await
                .context("no suitable GPU adapter")?;
            tracing::info!(name = %adapter.get_info().name, "headless adapter");
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("divine-headless"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    ..Default::default()
                })
                .await
                .context("device request failed")?;
            Ok((Arc::new(device), Arc::new(queue)))
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }
}
