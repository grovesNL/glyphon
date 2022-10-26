use cosmic_text::CacheKey;
use etagere::{size2, BucketedAtlasAllocator};
use std::{borrow::Cow, mem::size_of, num::NonZeroU64, sync::Arc};
use wgpu::{
    BindGroup, BindGroupEntry, BindGroupLayoutEntry, BindingResource, BindingType, BlendState,
    Buffer, BufferBindingType, BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites,
    Device, Extent3d, FilterMode, FragmentState, MultisampleState, PipelineLayoutDescriptor,
    PrimitiveState, Queue, RenderPipeline, RenderPipelineDescriptor, SamplerBindingType,
    SamplerDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages, Texture,
    TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
    TextureViewDescriptor, TextureViewDimension, VertexFormat, VertexState,
};

use crate::{GlyphDetails, GlyphToRender, Params, RecentlyUsedMap, Resolution};

pub(crate) const NUM_ATLAS_CHANNELS: usize = 4usize;

/// An atlas containing a cache of rasterized glyphs that can be rendered.
pub struct TextAtlas {
    pub(crate) texture_pending: Vec<u8>,
    pub(crate) texture: Texture,
    pub(crate) packer: BucketedAtlasAllocator,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) glyph_cache: RecentlyUsedMap<CacheKey, GlyphDetails>,
    pub(crate) params: Params,
    pub(crate) params_buffer: Buffer,
    pub(crate) pipeline: Arc<RenderPipeline>,
    pub(crate) bind_group: Arc<BindGroup>,
}

impl TextAtlas {
    /// Creates a new `TextAtlas`.
    pub fn new(device: &Device, _queue: &Queue, format: TextureFormat) -> Self {
        let max_texture_dimension_2d = device.limits().max_texture_dimension_2d;
        let width = max_texture_dimension_2d;
        let height = max_texture_dimension_2d;

        let packer = BucketedAtlasAllocator::new(size2(width as i32, height as i32));

        // Create a texture to use for our atlas
        let texture_pending = vec![0; (width * height) as usize * NUM_ATLAS_CHANNELS];
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("glyphon atlas"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        });
        let texture_view = texture.create_view(&TextureViewDescriptor::default());
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("glyphon sampler"),
            min_filter: FilterMode::Nearest,
            mag_filter: FilterMode::Nearest,
            mipmap_filter: FilterMode::Nearest,
            lod_min_clamp: 0f32,
            lod_max_clamp: 0f32,
            ..Default::default()
        });

        let glyph_cache = RecentlyUsedMap::new();

        // Create a render pipeline to use for rendering later
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("glyphon shader"),
            source: ShaderSource::Wgsl(Cow::Borrowed(include_str!("shader.wgsl"))),
        });

        let vertex_buffers = [wgpu::VertexBufferLayout {
            array_stride: size_of::<GlyphToRender>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: VertexFormat::Sint32x2,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: VertexFormat::Uint32,
                    offset: size_of::<u32>() as u64 * 2,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: VertexFormat::Uint32,
                    offset: size_of::<u32>() as u64 * 3,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: VertexFormat::Uint32,
                    offset: size_of::<u32>() as u64 * 4,
                    shader_location: 3,
                },
            ],
        }];

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(size_of::<Params>() as u64),
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
            label: Some("glyphon bind group layout"),
        });

        let params = Params {
            screen_resolution: Resolution {
                width: 0,
                height: 0,
            },
            _pad: [0, 0],
        };

        let params_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("glyphon params"),
            size: size_of::<Params>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&texture_view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(&sampler),
                },
            ],
            label: Some("glyphon bind group"),
        }));

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = Arc::new(device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("glyphon pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &vertex_buffers,
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: ColorWrites::default(),
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        }));

        Self {
            texture_pending,
            texture,
            packer,
            width,
            height,
            glyph_cache,
            params,
            params_buffer,
            pipeline,
            bind_group,
        }
    }
}
