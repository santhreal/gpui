//! Offscreen textures and pipelines for backdrop blurs and path-clipped
//! layers.
//!
//! A backdrop blur batch copies the frame drawn so far into the backdrop
//! texture and samples that copy through a clamp-to-edge sampler. A path clip
//! draws its subtree into a layer cleared to transparent black, one layer per
//! nesting depth, and composites the layer onto its parent scaled by the clip
//! path's coverage.
//!
//! Frames without blurs or clips create none of these resources. Textures are
//! released after [`gpui::LAYER_IDLE_RELEASE_FRAMES`] consecutive frames
//! without a use, and pipelines are compiled on first use.

use super::{
    PathSprite, PipelineState, RENDER_TARGET_FORMAT, ShaderModule, create_blend_state,
    create_blend_state_for_path_sprite,
};
use anyhow::{Context as _, Result};
use gpui::{BackdropBlur, LayerIdleCounter};
use std::slice;
use windows::Win32::Graphics::{Direct3D11::*, Dxgi::Common::*};

#[derive(Default)]
pub(super) struct RenderLayers {
    pipelines: Option<LayerPipelines>,
    width: u32,
    height: u32,
    backdrop: Option<Target>,
    clip_layers: Vec<Target>,
    blur_idle: LayerIdleCounter,
    clip_idle: LayerIdleCounter,
}

struct LayerPipelines {
    backdrop_blur: PipelineState<BackdropBlur>,
    path_clip_composite: PipelineState<PathSprite>,
    clamp_sampler: Option<ID3D11SamplerState>,
}

/// An offscreen texture with the views it is drawn and sampled through.
struct Target {
    texture: ID3D11Texture2D,
    render_target_view: Option<ID3D11RenderTargetView>,
    shader_resource_view: Option<ID3D11ShaderResourceView>,
}

impl RenderLayers {
    /// Uploads the frame's backdrop blur instances.
    pub fn upload_backdrop_blurs(
        &mut self,
        device: &ID3D11Device,
        device_context: &ID3D11DeviceContext,
        blurs: &[BackdropBlur],
    ) -> Result<()> {
        layer_pipelines(&mut self.pipelines, device)?
            .backdrop_blur
            .update_buffer(device, device_context, blurs)
    }

    /// Copies `frame` into the backdrop texture, then draws the uploaded
    /// blurs `start..start + len` into the bound render target, sampling
    /// that copy.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_backdrop_blurs(
        &mut self,
        device: &ID3D11Device,
        device_context: &ID3D11DeviceContext,
        frame: &ID3D11Texture2D,
        size: (u32, u32),
        batch_params_buffer: &ID3D11Buffer,
        start: u32,
        len: u32,
    ) -> Result<()> {
        self.match_size(size)?;
        let backdrop = match &mut self.backdrop {
            Some(backdrop) => backdrop,
            empty => empty.insert(new_target(device, size, false)?),
        };
        let pipelines = layer_pipelines(&mut self.pipelines, device)?;
        unsafe { device_context.CopyResource(&backdrop.texture, frame) };
        pipelines.backdrop_blur.draw_range_with_texture(
            device_context,
            slice::from_ref(&backdrop.shader_resource_view),
            batch_params_buffer,
            slice::from_ref(&pipelines.clamp_sampler),
            start,
            len,
        )
    }

    /// Clears the layer for clip nesting `depth`, creating it when missing.
    pub fn begin_clip_layer(
        &mut self,
        device: &ID3D11Device,
        device_context: &ID3D11DeviceContext,
        size: (u32, u32),
        depth: usize,
    ) -> Result<()> {
        self.match_size(size)?;
        while self.clip_layers.len() <= depth {
            self.clip_layers.push(new_target(device, size, true)?);
        }
        let view = self.clip_layers[depth]
            .render_target_view
            .as_ref()
            .context("path clip layer has no render target view")?;
        unsafe { device_context.ClearRenderTargetView(view, &[0.0; 4]) };
        Ok(())
    }

    /// Returns the render target view of the open clip layer at `depth`.
    pub fn clip_layer_view(&self, depth: usize) -> Result<&Option<ID3D11RenderTargetView>> {
        self.clip_layers
            .get(depth)
            .map(|layer| &layer.render_target_view)
            .with_context(|| format!("path clip layer {depth} missing"))
    }

    /// Draws the clip layer at `depth` onto the bound render target inside
    /// `sprite`, scaled by the alpha of `mask`, then unbinds both textures so
    /// the layer can be a render target again.
    pub fn draw_path_clip_composite(
        &mut self,
        device: &ID3D11Device,
        device_context: &ID3D11DeviceContext,
        sprite: PathSprite,
        depth: usize,
        mask: &Option<ID3D11ShaderResourceView>,
    ) -> Result<()> {
        let pipelines = layer_pipelines(&mut self.pipelines, device)?;
        let layer = self
            .clip_layers
            .get(depth)
            .with_context(|| format!("path clip layer {depth} missing"))?;
        pipelines.path_clip_composite.update_buffer(
            device,
            device_context,
            slice::from_ref(&sprite),
        )?;
        unsafe { device_context.PSSetShaderResources(2, Some(slice::from_ref(mask))) };
        let result = pipelines.path_clip_composite.draw_with_texture(
            device_context,
            slice::from_ref(&layer.shader_resource_view),
            slice::from_ref(&pipelines.clamp_sampler),
            1,
        );
        unsafe {
            device_context.VSSetShaderResources(0, Some(&[None]));
            device_context.PSSetShaderResources(0, Some(&[None, None, None]));
        }
        result
    }

    /// Records whether the finished frame used blurs and clips, and releases
    /// the textures of a feature idle for [`gpui::LAYER_IDLE_RELEASE_FRAMES`]
    /// frames.
    pub fn end_frame(&mut self, used_blur: bool, used_clip: bool) {
        if self.blur_idle.tick(used_blur) {
            self.backdrop = None;
        }
        if self.clip_idle.tick(used_clip) {
            self.clip_layers.clear();
        }
    }

    fn match_size(&mut self, size: (u32, u32)) -> Result<()> {
        anyhow::ensure!(
            size.0 > 0 && size.1 > 0,
            "cannot create a {}x{} offscreen layer",
            size.0,
            size.1
        );
        if (self.width, self.height) != size {
            (self.width, self.height) = size;
            self.backdrop = None;
            self.clip_layers.clear();
        }
        Ok(())
    }
}

fn layer_pipelines<'a>(
    pipelines: &'a mut Option<LayerPipelines>,
    device: &ID3D11Device,
) -> Result<&'a mut LayerPipelines> {
    match pipelines {
        Some(pipelines) => Ok(pipelines),
        empty => Ok(empty.insert(LayerPipelines::new(device)?)),
    }
}

impl LayerPipelines {
    fn new(device: &ID3D11Device) -> Result<Self> {
        Ok(Self {
            backdrop_blur: PipelineState::new(
                device,
                "backdrop_blur_pipeline",
                ShaderModule::BackdropBlur,
                4,
                create_blend_state(device)?,
            )?,
            path_clip_composite: PipelineState::new(
                device,
                "path_clip_composite_pipeline",
                ShaderModule::PathClipComposite,
                1,
                create_blend_state_for_path_sprite(device)?,
            )?,
            clamp_sampler: create_clamp_sampler(device)?,
        })
    }
}

fn create_clamp_sampler(device: &ID3D11Device) -> Result<Option<ID3D11SamplerState>> {
    let desc = D3D11_SAMPLER_DESC {
        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        MipLODBias: 0.0,
        MaxAnisotropy: 1,
        ComparisonFunc: D3D11_COMPARISON_ALWAYS,
        BorderColor: [0.0; 4],
        MinLOD: 0.0,
        MaxLOD: D3D11_FLOAT32_MAX,
    };
    let mut sampler = None;
    unsafe { device.CreateSamplerState(&desc, Some(&mut sampler))? };
    Ok(sampler)
}

/// Creates a `size` texture in the render target format with a shader
/// resource view, plus a render target view when `render_target` is set.
fn new_target(device: &ID3D11Device, size: (u32, u32), render_target: bool) -> Result<Target> {
    let bind_flags = if render_target {
        D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0
    } else {
        D3D11_BIND_SHADER_RESOURCE.0
    };
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size.0,
        Height: size.1,
        MipLevels: 1,
        ArraySize: 1,
        Format: RENDER_TARGET_FORMAT,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: bind_flags as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    let texture = texture.context("creating an offscreen layer texture")?;
    let mut render_target_view = None;
    if render_target {
        unsafe { device.CreateRenderTargetView(&texture, None, Some(&mut render_target_view))? };
    }
    let mut shader_resource_view = None;
    unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut shader_resource_view))? };
    Ok(Target {
        texture,
        render_target_view,
        shader_resource_view,
    })
}
