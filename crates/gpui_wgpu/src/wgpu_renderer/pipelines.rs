//! The shader modules and render pipelines of a renderer.
//!
//! Each module and each pipeline is created on a thread of its own.
//! Creating a module parses, validates, and translates its WGSL; creating
//! a pipeline compiles its two stages in the driver. Neither reads another
//! module or pipeline, and wgpu creates them from several threads at once.
//! The web has no threads and creates them in turn.
//!
//! A validation error of a creation reaches the device's uncaptured error
//! handler, as it does on the caller's thread.

use super::{STORAGE_BUFFER_SHADERS, SUBPIXEL_SHADERS, WEBGL_SHADERS, WgpuBindGroupLayouts};
use std::thread::{Scope, ScopedJoinHandle};

/// The shader modules pipelines are created from.
pub(super) struct WgpuShaders {
    main: wgpu::ShaderModule,
    /// The dual-source blending shaders of subpixel text, if the device
    /// blends with two sources.
    subpixel: Option<wgpu::ShaderModule>,
}

impl WgpuShaders {
    pub(super) fn new(
        device: &wgpu::Device,
        dual_source_blending: bool,
        uses_webgl_instance_data: bool,
    ) -> Self {
        // Diagnostic guard: verify the device actually has
        // DUAL_SOURCE_BLENDING. We have a crash report (ZED-5G1) where a
        // feature mismatch caused a wgpu-hal abort, but we haven't
        // identified the code path that produces the mismatch. This
        // guard prevents the crash and logs more evidence.
        // Remove this check once:
        // a) We find and fix the root cause, or
        // b) There are no reports of this warning appearing for some time.
        let device_has_feature = device
            .features()
            .contains(wgpu::Features::DUAL_SOURCE_BLENDING);
        if dual_source_blending && !device_has_feature {
            log::error!(
                "BUG: dual_source_blending flag is true but device does not \
                 have DUAL_SOURCE_BLENDING enabled (device features: {:?}). \
                 Falling back to mono text rendering. Please report this at \
                 https://github.com/zed-industries/zed/issues",
                device.features(),
            );
        }
        let dual_source_blending =
            dual_source_blending && device_has_feature && !uses_webgl_instance_data;

        let source = if uses_webgl_instance_data {
            WEBGL_SHADERS
        } else {
            STORAGE_BUFFER_SHADERS
        };
        std::thread::scope(|scope| {
            let module = |label: &'static str, source: &'static str| {
                Job::spawn(scope, "gpu-shaders", move || {
                    device.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some(label),
                        source: wgpu::ShaderSource::Wgsl(source.into()),
                    })
                })
            };
            let main = module("gpui_shaders", source);
            let subpixel =
                dual_source_blending.then(|| module("gpui_subpixel_shaders", SUBPIXEL_SHADERS));
            Self {
                main: main.join(),
                subpixel: subpixel.map(Job::join),
            }
        })
    }
}

/// The render pipelines of a renderer.
pub(super) struct WgpuPipelines {
    pub(super) quads: wgpu::RenderPipeline,
    pub(super) shadows: wgpu::RenderPipeline,
    pub(super) path_rasterization: wgpu::RenderPipeline,
    pub(super) paths: wgpu::RenderPipeline,
    pub(super) underlines: wgpu::RenderPipeline,
    pub(super) mono_sprites: wgpu::RenderPipeline,
    pub(super) subpixel_sprites: Option<wgpu::RenderPipeline>,
    pub(super) poly_sprites: wgpu::RenderPipeline,
    #[allow(dead_code)]
    pub(super) surfaces: wgpu::RenderPipeline,
    pub(super) backdrop_blur: wgpu::RenderPipeline,
    pub(super) path_mask_composite: wgpu::RenderPipeline,
    /// Writes transparent black inside the scissor rect. A render pass load
    /// op clears the whole attachment, so a partial frame clears its damaged
    /// region with this instead.
    pub(super) clear: wgpu::RenderPipeline,
    pub(super) present_retained: Option<wgpu::RenderPipeline>,
}

impl WgpuPipelines {
    /// Creates the pipelines that draw to a `format` target composited with
    /// `alpha_mode`. A target that is not `copyable` is presented by the
    /// `present_retained` pipeline.
    pub(super) fn new(
        device: &wgpu::Device,
        layouts: &WgpuBindGroupLayouts,
        shaders: &WgpuShaders,
        format: wgpu::TextureFormat,
        alpha_mode: wgpu::CompositeAlphaMode,
        path_sample_count: u32,
        copyable: bool,
    ) -> Self {
        let layout = |label: &'static str,
                      bind_group_layouts: &[Option<&wgpu::BindGroupLayout>]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts,
                immediate_size: 0,
            })
        };
        let globals = Some(&layouts.globals);
        let instances = Some(&layouts.instances);
        let texture = Some(&layouts.texture);
        let instance_layout = layout("instances_layout", &[globals, instances]);
        let textured_layout = layout("textured_instances_layout", &[globals, instances, texture]);
        let surfaces_layout = layout("surfaces_layout", &[globals, Some(&layouts.surfaces)]);
        let path_mask_composite_layout = layout(
            "path_mask_composite_layout",
            &[globals, instances, texture, texture],
        );
        let clear_layout = layout("clear_layout", &[]);
        let present_shader = (!copyable).then(|| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("retained_present_shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("../present.wgsl").into()),
            })
        });

        let target = |blend, write_mask| wgpu::ColorTargetState {
            format,
            blend,
            write_mask,
        };
        let blended = target(
            Some(match alpha_mode {
                wgpu::CompositeAlphaMode::PreMultiplied => {
                    wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING
                }
                _ => wgpu::BlendState::ALPHA_BLENDING,
            }),
            wgpu::ColorWrites::ALL,
        );
        let paths_blend = target(
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
            wgpu::ColorWrites::ALL,
        );
        let main = &shaders.main;
        let strip = |name, layout, entry: [&'static str; 2], target: &wgpu::ColorTargetState| {
            PipelineSpec {
                name,
                layout: Some(layout),
                vertex: (main, entry[0]),
                fragment: (main, entry[1]),
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                target: target.clone(),
                sample_count: 1,
            }
        };
        let fullscreen = |name, layout, fragment| PipelineSpec {
            name,
            layout,
            vertex: (main, "vs_clear"),
            fragment,
            topology: wgpu::PrimitiveTopology::TriangleList,
            target: target(None, wgpu::ColorWrites::ALL),
            sample_count: 1,
        };

        let quads = strip("quads", &instance_layout, ["vs_quad", "fs_quad"], &blended);
        let shadows = strip(
            "shadows",
            &instance_layout,
            ["vs_shadow", "fs_shadow"],
            &blended,
        );
        let path_rasterization = PipelineSpec {
            name: "path_rasterization",
            layout: Some(&instance_layout),
            vertex: (main, "vs_path_rasterization"),
            fragment: (main, "fs_path_rasterization"),
            topology: wgpu::PrimitiveTopology::TriangleList,
            target: target(
                Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                wgpu::ColorWrites::ALL,
            ),
            sample_count: path_sample_count,
        };
        let paths = strip(
            "paths",
            &textured_layout,
            ["vs_path", "fs_path"],
            &paths_blend,
        );
        let underlines = strip(
            "underlines",
            &instance_layout,
            ["vs_underline", "fs_underline"],
            &blended,
        );
        let mono_sprites = strip(
            "mono_sprites",
            &textured_layout,
            ["vs_mono_sprite", "fs_mono_sprite"],
            &blended,
        );
        let subpixel_sprites = shaders.subpixel.as_ref().map(|subpixel| PipelineSpec {
            name: "subpixel_sprites",
            layout: Some(&textured_layout),
            vertex: (subpixel, "vs_subpixel_sprite"),
            fragment: (subpixel, "fs_subpixel_sprite"),
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            target: target(
                Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Src1,
                        dst_factor: wgpu::BlendFactor::OneMinusSrc1,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                wgpu::ColorWrites::COLOR,
            ),
            sample_count: 1,
        });
        let poly_sprites = strip(
            "poly_sprites",
            &textured_layout,
            ["vs_poly_sprite", "fs_poly_sprite"],
            &blended,
        );
        let surfaces = strip(
            "surfaces",
            &surfaces_layout,
            ["vs_surface", "fs_surface"],
            &blended,
        );
        let backdrop_blur = strip(
            "backdrop_blur",
            &textured_layout,
            ["vs_backdrop_blur", "fs_backdrop_blur"],
            &blended,
        );
        let path_mask_composite = strip(
            "path_mask_composite",
            &path_mask_composite_layout,
            ["vs_path", "fs_path_mask_composite"],
            &paths_blend,
        );
        let clear = fullscreen("clear", Some(&clear_layout), (main, "fs_clear"));
        let present_retained = present_shader
            .as_ref()
            .map(|shader| fullscreen("present_retained", None, (shader, "fs_present")));

        std::thread::scope(|scope| {
            let required = [
                quads,
                shadows,
                path_rasterization,
                paths,
                underlines,
                mono_sprites,
                poly_sprites,
                surfaces,
                backdrop_blur,
                path_mask_composite,
                clear,
            ]
            .map(|spec| spec.spawn(scope, device));
            let subpixel_sprites = subpixel_sprites.map(|spec| spec.spawn(scope, device));
            let present_retained = present_retained.map(|spec| spec.spawn(scope, device));
            let [
                quads,
                shadows,
                path_rasterization,
                paths,
                underlines,
                mono_sprites,
                poly_sprites,
                surfaces,
                backdrop_blur,
                path_mask_composite,
                clear,
            ] = required.map(Job::join);
            Self {
                quads,
                shadows,
                path_rasterization,
                paths,
                underlines,
                mono_sprites,
                subpixel_sprites: subpixel_sprites.map(Job::join),
                poly_sprites,
                surfaces,
                backdrop_blur,
                path_mask_composite,
                clear,
                present_retained: present_retained.map(Job::join),
            }
        })
    }
}

/// One render pipeline: a vertex and a fragment stage, each a module and
/// an entry point, drawing one color target without depth or vertex
/// buffers.
#[derive(Clone)]
struct PipelineSpec<'a> {
    name: &'static str,
    /// The pipeline layout, or `None` to derive it from the shaders.
    layout: Option<&'a wgpu::PipelineLayout>,
    vertex: (&'a wgpu::ShaderModule, &'static str),
    fragment: (&'a wgpu::ShaderModule, &'static str),
    topology: wgpu::PrimitiveTopology,
    target: wgpu::ColorTargetState,
    sample_count: u32,
}

impl<'scope> PipelineSpec<'scope> {
    /// Starts creating the pipeline on a thread of `scope`.
    fn spawn(
        self,
        scope: &'scope Scope<'scope, '_>,
        device: &'scope wgpu::Device,
    ) -> Job<'scope, wgpu::RenderPipeline> {
        Job::spawn(scope, "gpu-pipeline", move || self.create(device))
    }

    fn create(&self, device: &wgpu::Device) -> wgpu::RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(self.name),
            layout: self.layout,
            vertex: wgpu::VertexState {
                module: self.vertex.0,
                entry_point: Some(self.vertex.1),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: self.fragment.0,
                entry_point: Some(self.fragment.1),
                targets: &[Some(self.target.clone())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: self.topology,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: self.sample_count,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        })
    }
}

/// A value a thread of a scope computes, or the value itself when no
/// thread could be started for it.
enum Job<'scope, T> {
    Thread(ScopedJoinHandle<'scope, T>),
    Done(T),
}

impl<'scope, T> Job<'scope, T> {
    /// Computes `f` on a thread of `scope` named `name`, or on the caller's
    /// thread if none can be started.
    #[cfg(not(target_family = "wasm"))]
    fn spawn<F>(scope: &'scope Scope<'scope, '_>, name: &str, f: F) -> Self
    where
        F: Fn() -> T + Send + Sync + 'scope,
        T: Send + 'scope,
    {
        let f = std::sync::Arc::new(f);
        let thread = std::thread::Builder::new()
            .name(name.into())
            .spawn_scoped(scope, {
                let f = std::sync::Arc::clone(&f);
                move || f()
            });
        match thread {
            Ok(thread) => Self::Thread(thread),
            Err(_) => Self::Done(f()),
        }
    }

    /// Computes `f` on the caller's thread: the web has no threads.
    #[cfg(target_family = "wasm")]
    fn spawn<F>(_scope: &'scope Scope<'scope, '_>, _name: &str, f: F) -> Self
    where
        F: Fn() -> T,
    {
        Self::Done(f())
    }

    /// The value. A panic on its thread continues on the caller's.
    fn join(self) -> T {
        match self {
            Self::Thread(thread) => thread
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            Self::Done(value) => value,
        }
    }
}
