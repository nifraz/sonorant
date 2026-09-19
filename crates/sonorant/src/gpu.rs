//! The window's GPU surface, device and queue.

use std::sync::Arc;

use winit::window::Window;

use crate::options::Options;

#[derive(Debug)]
pub struct Gpu {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    /// The format the visuals render through: the swapchain's sRGB view, so shaders work
    /// in linear light and the hardware encodes.
    pub view_format: wgpu::TextureFormat,
    pub present_modes: Vec<wgpu::PresentMode>,
}

/// Preferred backends, best first: Direct3D 12 on Windows and Vulkan elsewhere, with
/// OpenGL as the fallback for GPUs without either.
fn backend_rank(b: wgpu::Backend) -> u8 {
    match b {
        wgpu::Backend::Dx12 if cfg!(windows) => 3,
        wgpu::Backend::Vulkan => 2,
        wgpu::Backend::Metal => 2,
        wgpu::Backend::Gl => 1,
        _ => 0,
    }
}

fn device_rank(t: wgpu::DeviceType) -> u8 {
    match t {
        wgpu::DeviceType::DiscreteGpu => 3,
        wgpu::DeviceType::IntegratedGpu => 2,
        wgpu::DeviceType::VirtualGpu => 1,
        _ => 0,
    }
}

/// The backend tried first: the platform's own. Opening every backend to compare them
/// cost seconds at start-up (OpenGL and Vulkan each load and probe their drivers), so
/// the others are only tried when this one finds nothing that can draw to the window.
const PRIMARY: wgpu::Backends = if cfg!(windows) {
    wgpu::Backends::DX12
} else if cfg!(target_vendor = "apple") {
    wgpu::Backends::METAL
} else {
    wgpu::Backends::VULKAN
};

impl Gpu {
    /// Opens the GPU for `window`: the backend `--backend` asks for, or the platform's
    /// own, falling back to the rest.
    pub fn open(
        event_loop: &winit::event_loop::ActiveEventLoop,
        window: Arc<Window>,
        options: &Options,
    ) -> Result<Gpu, String> {
        let attempt = |backends: wgpu::Backends| {
            let mut desc = wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(
                event_loop.owned_display_handle(),
            ));
            desc.backends = backends;
            let instance = wgpu::Instance::new(desc);
            Gpu::new(&instance, window.clone(), options)
        };
        if let Some(backends) = options.backends {
            return attempt(backends);
        }
        attempt(PRIMARY).or_else(|e| {
            log::warn!("{e} on {PRIMARY:?}; trying the other backends");
            attempt(wgpu::Backends::all() - PRIMARY)
        })
    }

    pub fn new(
        instance: &wgpu::Instance,
        window: Arc<Window>,
        options: &Options,
    ) -> Result<Gpu, String> {
        let size = window.inner_size();
        let surface = instance
            .create_surface(window)
            .map_err(|e| format!("cannot create a surface for the window: {e}"))?;

        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
        for a in &adapters {
            let i = a.get_info();
            log::debug!("adapter: {} ({:?}, {:?})", i.name, i.backend, i.device_type);
        }
        let adapter = adapters
            .into_iter()
            .filter(|a| a.is_surface_supported(&surface))
            .max_by_key(|a| {
                let i = a.get_info();
                (device_rank(i.device_type), backend_rank(i.backend))
            })
            .ok_or("no GPU adapter can draw to this window")?;
        let adapter_info = adapter.get_info();
        log::info!(
            "GPU: {} ({:?}, driver {} {})",
            adapter_info.name,
            adapter_info.backend,
            adapter_info.driver,
            adapter_info.driver_info
        );

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("sonorant"),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| format!("cannot open the GPU: {e}"))?;

        let caps = surface.get_capabilities(&adapter);
        // A plain 8-bit swapchain with an sRGB view: the visuals render through the view,
        // egui draws to the swapchain directly as it expects.
        let plain = caps.formats.iter().copied().find(|f| {
            matches!(
                f,
                wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
            )
        });
        let (format, view_format) = match plain {
            Some(f) => (f, f.add_srgb_suffix()),
            None => {
                let f = caps
                    .formats
                    .iter()
                    .copied()
                    .find(wgpu::TextureFormat::is_srgb)
                    .or_else(|| caps.formats.first().copied())
                    .ok_or("the surface supports no formats")?;
                (f, f)
            }
        };
        let present_mode = match options.present_mode {
            Some(mode) if caps.present_modes.contains(&mode) => mode,
            Some(mode) => {
                log::warn!("{mode:?} isn't available here; using Fifo");
                wgpu::PresentMode::Fifo
            }
            None => wgpu::PresentMode::Fifo,
        };
        // Copying frames out costs nothing unless it's asked for.
        let copy = if options.screenshot.is_some()
            && caps.usages.contains(wgpu::TextureUsages::COPY_SRC)
        {
            wgpu::TextureUsages::COPY_SRC
        } else {
            wgpu::TextureUsages::empty()
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | copy,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            desired_maximum_frame_latency: options.frame_latency,
            alpha_mode: caps
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: if view_format != format {
                vec![view_format]
            } else {
                vec![]
            },
            // Standard dynamic range.
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        log::info!(
            "surface: {format:?} viewed as {view_format:?}, {present_mode:?}, frame latency {}",
            options.frame_latency
        );

        Ok(Gpu {
            surface,
            device,
            queue,
            config,
            view_format,
            present_modes: caps.present_modes,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return; // minimised
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn reconfigure(&self) {
        self.surface.configure(&self.device, &self.config);
    }

    pub fn set_present_mode(&mut self, mode: wgpu::PresentMode) {
        if self.config.present_mode != mode && self.present_modes.contains(&mode) {
            self.config.present_mode = mode;
            self.reconfigure();
        }
    }
}
