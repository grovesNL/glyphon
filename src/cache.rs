use crate::{GlyphToRender, Params};

use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutEntry,
    BindingResource, BindingType, BlendState, Buffer, BufferBindingType, ColorTargetState,
    ColorWrites, DepthStencilState, Device, FilterMode, FragmentState, MultisampleState,
    PipelineCompilationOptions, PipelineLayout, PipelineLayoutDescriptor, PrimitiveState,
    RenderPipeline, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderModule, ShaderModuleDescriptor, ShaderSource, ShaderStages, TextureFormat,
    TextureSampleType, TextureView, TextureViewDimension, VertexFormat, VertexState,
};

use std::borrow::Cow;
use std::mem;
use std::num::NonZeroU64;
use std::ops::Deref;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub struct Cache(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    sampler: Sampler,
    shader: ShaderModule,
    vertex_buffers: [wgpu::VertexBufferLayout<'static>; 1],
    atlas_layout: BindGroupLayout,
    uniforms_layout: BindGroupLayout,
    pipeline_layout: PipelineLayout,
    cache: RwLock<
        Vec<(
            TextureFormat,
            MultisampleState,
            Option<DepthStencilState>,
            Arc<RenderPipeline>,
        )>,
    >,
}

impl Cache {
    pub fn new(device: &Device) -> Self {
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("glyphon sampler"),
            min_filter: FilterMode::Nearest,
            mag_filter: FilterMode::Nearest,
            mipmap_filter: FilterMode::Nearest,
            lod_min_clamp: 0f32,
            lod_max_clamp: 0f32,
            ..Default::default()
        });

        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("glyphon shader"),
            source: ShaderSource::Wgsl(Cow::Borrowed(include_str!("shader.wgsl"))),
        });

        let vertex_buffer_layout = wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<GlyphToRender>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: VertexFormat::Sint32x2,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: VertexFormat::Uint32,
                    offset: mem::size_of::<u32>() as u64 * 2,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: VertexFormat::Uint32,
                    offset: mem::size_of::<u32>() as u64 * 3,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: VertexFormat::Uint32,
                    offset: mem::size_of::<u32>() as u64 * 4,
                    shader_location: 3,
                },
                wgpu::VertexAttribute {
                    format: VertexFormat::Uint32,
                    offset: mem::size_of::<u32>() as u64 * 5,
                    shader_location: 4,
                },
                wgpu::VertexAttribute {
                    format: VertexFormat::Float32,
                    offset: mem::size_of::<u32>() as u64 * 6,
                    shader_location: 5,
                },
            ],
        };

        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        multisampled: false,
                        view_dimension: TextureViewDimension::D2,
                        sample_type: TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        multisampled: false,
                        view_dimension: TextureViewDimension::D2,
                        sample_type: TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
            label: Some("glyphon atlas bind group layout"),
        });

        let uniforms_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(mem::size_of::<Params>() as u64),
                },
                count: None,
            }],
            label: Some("glyphon uniforms bind group layout"),
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&atlas_layout, &uniforms_layout],
            push_constant_ranges: &[],
        });

        Self(Arc::new(Inner {
            sampler,
            shader,
            vertex_buffers: [vertex_buffer_layout],
            uniforms_layout,
            atlas_layout,
            pipeline_layout,
            cache: RwLock::new(Vec::new()),
        }))
    }

    pub(crate) fn create_atlas_bind_group(
        &self,
        device: &Device,
        color_atlas: &TextureView,
        mask_atlas: &TextureView,
    ) -> BindGroup {
        device.create_bind_group(&BindGroupDescriptor {
            layout: &self.0.atlas_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(color_atlas),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(mask_atlas),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(&self.0.sampler),
                },
            ],
            label: Some("glyphon atlas bind group"),
        })
    }

    pub(crate) fn create_uniforms_bind_group(&self, device: &Device, buffer: &Buffer) -> BindGroup {
        device.create_bind_group(&BindGroupDescriptor {
            layout: &self.0.uniforms_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
            label: Some("glyphon uniforms bind group"),
        })
    }

    pub(crate) fn get_or_create_pipeline(
        &self,
        device: &Device,
        format: TextureFormat,
        multisample: MultisampleState,
        depth_stencil: Option<DepthStencilState>,
    ) -> Arc<RenderPipeline> {
        let Inner {
            cache,
            pipeline_layout,
            shader,
            vertex_buffers,
            ..
        } = self.0.deref();

        let mut cache = cache.write().expect("Write pipeline cache");

        cache
            .iter()
            .find(|(fmt, ms, ds, _)| fmt == &format && ms == &multisample && ds == &depth_stencil)
            .map(|(_, _, _, p)| Arc::clone(p))
            .unwrap_or_else(|| {
                let pipeline = Arc::new(device.create_render_pipeline(&RenderPipelineDescriptor {
                    label: Some("glyphon pipeline"),
                    layout: Some(pipeline_layout),
                    vertex: VertexState {
                        module: shader,
                        entry_point: "vs_main",
                        buffers: vertex_buffers,
                        compilation_options: PipelineCompilationOptions::default(),
                    },
                    fragment: Some(FragmentState {
                        module: shader,
                        entry_point: "fs_main",
                        targets: &[Some(ColorTargetState {
                            format,
                            blend: Some(BlendState::ALPHA_BLENDING),
                            write_mask: ColorWrites::default(),
                        })],
                        compilation_options: PipelineCompilationOptions::default(),
                    }),
                    primitive: PrimitiveState::default(),
                    depth_stencil: depth_stencil.clone(),
                    multisample,
                    multiview: None,
                }));

                cache.push((format, multisample, depth_stencil, pipeline.clone()));

                pipeline
            })
            .clone()
    }
}
