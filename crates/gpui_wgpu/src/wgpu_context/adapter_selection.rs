//! The order adapters are tried in, and the backends of the instances they
//! are enumerated from.

use super::{CompositorGpuHint, WgpuContext};
use anyhow::Context as _;
use wgpu::TextureFormat;

/// The backends of an instance a native context is selected from. A
/// context is selected from the Vulkan tier, and from the Vulkan and GL
/// tier only when the Vulkan tier selects no adapter.
///
/// Creating an instance with the GL backend loads every installed EGL
/// vendor library and initializes an EGL display on each, which takes tens
/// of milliseconds; creating the Vulkan tier's instance does not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BackendTier {
    /// Vulkan alone. It selects an adapter only if the adapter ranks above
    /// every adapter the GL backend reports, which the Vulkan and GL tier
    /// would then select as well.
    Vulkan,
    /// Vulkan and GL, selecting among the adapters of both.
    VulkanAndGl,
}

impl BackendTier {
    pub(super) fn backends(self) -> wgpu::Backends {
        match self {
            Self::Vulkan => wgpu::Backends::VULKAN,
            Self::VulkanAndGl => wgpu::Backends::VULKAN | wgpu::Backends::GL,
        }
    }

    /// Whether the tier may select an adapter of `rank`.
    fn selects(self, rank: AdapterRank) -> bool {
        match self {
            Self::Vulkan => rank < AdapterRank::BEST_GL,
            Self::VulkanAndGl => true,
        }
    }
}

/// The position of an adapter in selection order. Adapters are tried from
/// the lowest rank up; the fields compare in declaration order.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct AdapterRank {
    /// 0 for the device `ZED_DEVICE_ID` selects.
    user_override: u8,
    /// 0 for the GPU the display server renders on.
    compositor_match: u8,
    /// Discrete, integrated, other, virtual, CPU. OpenGL reports a GPU it
    /// does not recognize as other, which ranks it above a virtual GPU.
    device_type: u8,
    /// 0 for Vulkan, Metal, and DX12.
    backend: u8,
}

impl AdapterRank {
    /// The lowest rank of an adapter of the GL backend. It reports the
    /// device id as 0, which neither `ZED_DEVICE_ID` nor the compositor
    /// hint match, and the device type as integrated, other, or CPU.
    const BEST_GL: Self = Self {
        user_override: 1,
        compositor_match: 1,
        device_type: device_type_rank(wgpu::DeviceType::IntegratedGpu),
        backend: backend_rank(wgpu::Backend::Gl),
    };

    fn new(
        info: &wgpu::AdapterInfo,
        device_id_filter: Option<u32>,
        compositor_gpu: Option<&CompositorGpuHint>,
    ) -> Self {
        // The GL backend reports device 0 for every adapter, so a device
        // id matches only when it is not 0.
        let device_known = info.device != 0;
        let user_override = match device_id_filter {
            Some(id) if device_known && info.device == id => 0,
            _ => 1,
        };
        let compositor_match = match compositor_gpu {
            Some(hint)
                if device_known
                    && info.vendor == hint.vendor_id
                    && info.device == hint.device_id =>
            {
                0
            }
            _ => 1,
        };
        Self {
            user_override,
            compositor_match,
            device_type: device_type_rank(info.device_type),
            backend: backend_rank(info.backend),
        }
    }
}

const fn device_type_rank(device_type: wgpu::DeviceType) -> u8 {
    match device_type {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::Other => 2,
        wgpu::DeviceType::VirtualGpu => 3,
        wgpu::DeviceType::Cpu => 4,
    }
}

const fn backend_rank(backend: wgpu::Backend) -> u8 {
    match backend {
        wgpu::Backend::Vulkan | wgpu::Backend::Metal | wgpu::Backend::Dx12 => 0,
        _ => 1,
    }
}

impl WgpuContext {
    /// Selects an adapter of `instance` that `tier` may select and creates
    /// its device, testing that the device configures `surface`. That test
    /// is the only reliable one on hybrid GPU systems, where an adapter can
    /// report a surface as compatible and fail to configure it (NVIDIA
    /// reports Vulkan Wayland support and fails where the compositor runs
    /// on the Intel GPU).
    pub(super) async fn select_adapter_and_device(
        instance: &wgpu::Instance,
        tier: BackendTier,
        device_id_filter: Option<u32>,
        surface: Option<&wgpu::Surface<'_>>,
        compositor_gpu: Option<&CompositorGpuHint>,
        reject_software: bool,
    ) -> anyhow::Result<(
        wgpu::Adapter,
        wgpu::Device,
        wgpu::Queue,
        bool,
        TextureFormat,
    )> {
        let mut adapters: Vec<_> = instance.enumerate_adapters(wgpu::Backends::all()).await;

        if adapters.is_empty() {
            anyhow::bail!("No GPU adapters found");
        }

        if let Some(device_id) = device_id_filter {
            log::info!("ZED_DEVICE_ID filter: {:#06x}", device_id);
        }

        // The backend, vendor, device, and name order adapters of one rank
        // deterministically.
        adapters.sort_by_key(|adapter| {
            let info = adapter.get_info();
            (
                AdapterRank::new(&info, device_id_filter, compositor_gpu),
                info.backend as u8,
                info.vendor,
                info.device,
                info.name,
            )
        });

        log::info!("Found {} GPU adapter(s):", adapters.len());
        for adapter in &adapters {
            let info = adapter.get_info();
            log::info!(
                "  - {} (vendor={:#06x}, device={:#06x}, backend={:?}, type={:?})",
                info.name,
                info.vendor,
                info.device,
                info.backend,
                info.device_type,
            );
        }

        for adapter in adapters {
            let info = adapter.get_info();

            if reject_software && info.device_type == wgpu::DeviceType::Cpu {
                log::info!(
                    "Skipping software renderer: {} ({:?})",
                    info.name,
                    info.backend
                );
                continue;
            }

            // Adapters are sorted, so no later adapter ranks higher.
            if !tier.selects(AdapterRank::new(&info, device_id_filter, compositor_gpu)) {
                anyhow::bail!(
                    "{} ({:?}, {:?}) and every adapter after it rank below the GL adapters",
                    info.name,
                    info.backend,
                    info.device_type
                );
            }

            log::info!("Testing adapter: {} ({:?})...", info.name, info.backend);

            let result = if let Some(surface) = surface {
                Self::try_adapter_with_surface(&adapter, surface).await
            } else {
                Self::create_device(&adapter).await
            };

            match result {
                Ok((device, queue, dual_source_blending, color_atlas_texture_format)) => {
                    log::info!(
                        "Selected GPU (passed configuration test): {} ({:?})",
                        info.name,
                        info.backend
                    );
                    return Ok((
                        adapter,
                        device,
                        queue,
                        dual_source_blending,
                        color_atlas_texture_format,
                    ));
                }
                Err(e) => {
                    log::info!(
                        "  Adapter {} ({:?}) failed: {}, trying next...",
                        info.name,
                        info.backend,
                        e
                    );
                }
            }
        }

        anyhow::bail!("No GPU adapter found that can configure the display surface")
    }

    /// Creates the device of `adapter` and configures `surface` with it.
    /// Returns the device and queue if the configuration succeeds, so they
    /// can be reused.
    async fn try_adapter_with_surface(
        adapter: &wgpu::Adapter,
        surface: &wgpu::Surface<'_>,
    ) -> anyhow::Result<(wgpu::Device, wgpu::Queue, bool, TextureFormat)> {
        let caps = surface.get_capabilities(adapter);
        if caps.formats.is_empty() {
            anyhow::bail!("no compatible surface formats");
        }
        if caps.alpha_modes.is_empty() {
            anyhow::bail!("no compatible alpha modes");
        }

        let (device, queue, dual_source_blending, color_atlas_texture_format) =
            Self::create_device(adapter).await?;
        let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

        let test_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: caps.formats[0],
            width: 64,
            height: 64,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };

        surface.configure(&device, &test_config);

        let error = error_scope.pop().await;
        if let Some(e) = error {
            anyhow::bail!("surface configuration failed: {e}");
        }

        Ok((
            device,
            queue,
            dual_source_blending,
            color_atlas_texture_format,
        ))
    }
}

pub(super) fn parse_pci_id(id: &str) -> anyhow::Result<u32> {
    let mut id = id.trim();

    if id.starts_with("0x") || id.starts_with("0X") {
        id = &id[2..];
    }
    let is_hex_string = id.chars().all(|c| c.is_ascii_hexdigit());
    let is_4_chars = id.len() == 4;
    anyhow::ensure!(
        is_4_chars && is_hex_string,
        "Expected a 4 digit PCI ID in hexadecimal format"
    );

    u32::from_str_radix(id, 16).context("parsing PCI ID as hex")
}

#[cfg(test)]
mod tests;
