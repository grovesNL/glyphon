use crate::{text_render::ContentType, CacheKey, GlyphDetails, GlyphToRender, Params, Resolution};
use etagere::{size2, Allocation, BucketedAtlasAllocator};
use lru::LruCache;
use std::{borrow::Cow, mem::size_of, num::NonZeroU64, sync::Arc};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayoutEntry, BindingResource,
    BindingType, BlendState, Buffer, BufferBindingType, BufferDescriptor, BufferUsages,
    ColorTargetState, ColorWrites, Device, Extent3d, FilterMode, FragmentState, MultisampleState,
    PipelineLayoutDescriptor, PrimitiveState, Queue, RenderPipeline, RenderPipelineDescriptor,
    SamplerBindingType, SamplerDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages,
    Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
    TextureView, TextureViewDescriptor, TextureViewDimension, VertexFormat, VertexState,
};

pub(crate) struct InnerAtlas {
    pub texture_pending: Vec<u8>,
    pub texture: Texture,
    pub texture_view: TextureView,
    pub packer: BucketedAtlasAllocator,
    pub width: u32,
    pub height: u32,
    pub glyph_cache: LruCache<CacheKey, GlyphDetails>,
    pub num_atlas_channels: usize,
}

impl InnerAtlas {
    fn new(device: &Device, _queue: &Queue, num_atlas_channels: usize) -> Self {
        let max_texture_dimension_2d = device.limits().max_texture_dimension_2d;
        let width = max_texture_dimension_2d;
        let height = max_texture_dimension_2d;

        let packer = BucketedAtlasAllocator::new(size2(width as i32, height as i32));

        // Create a texture to use for our atlas
        let texture_pending = vec![0; (width * height) as usize * num_atlas_channels];
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
            format: match num_atlas_channels {
                1 => TextureFormat::R8Unorm,
                4 => TextureFormat::Rgba8Unorm,
                _ => panic!("unexpected number of channels"),
            },
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        });

        let texture_view = texture.create_view(&TextureViewDescriptor::default());

        let glyph_cache = LruCache::unbounded();

        Self {
            texture_pending,
            texture,
            texture_view,
            packer,
            width,
            height,
            glyph_cache,
            num_atlas_channels,
        }
    }

    pub(crate) fn try_allocate(&mut self, width: usize, height: usize) -> Option<Allocation> {
        let size = size2(width as i32, height as i32);

        loop {
            let allocation = self.packer.allocate(size);
            if allocation.is_some() {
                return allocation;
            }

            // Try to free least recently used allocation
            let (_, value) = self.glyph_cache.pop_lru()?;
            self.packer
                .deallocate(value.atlas_id.expect("cache corrupt"));
        }
    }
}

/// An atlas containing a cache of rasterized glyphs that can be rendered.
pub struct TextAtlas {
    pub(crate) params: Params,
    pub(crate) params_buffer: Buffer,
    pub(crate) pipeline: Arc<RenderPipeline>,
    pub(crate) bind_group: Arc<BindGroup>,
    pub(crate) color_atlas: InnerAtlas,
    pub(crate) mask_atlas: InnerAtlas,
}

impl TextAtlas {
    /// Creates a new `TextAtlas`.
    pub fn new(device: &Device, queue: &Queue, format: TextureFormat) -> Self {
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("glyphon sampler"),
            min_filter: FilterMode::Nearest,
            mag_filter: FilterMode::Nearest,
            mipmap_filter: FilterMode::Nearest,
            lod_min_clamp: 0f32,
            lod_max_clamp: 0f32,
            ..Default::default()
        });

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
                wgpu::VertexAttribute {
                    format: VertexFormat::Uint32,
                    offset: size_of::<u32>() as u64 * 5,
                    shader_location: 4,
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
                    visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        multisampled: false,
                        view_dimension: TextureViewDimension::D2,
                        sample_type: TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 3,
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

        let color_atlas = InnerAtlas::new(device, queue, 4);
        let mask_atlas = InnerAtlas::new(device, queue, 1);

        let bind_group = Arc::new(device.create_bind_group(&BindGroupDescriptor {
            layout: &bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&color_atlas.texture_view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(&mask_atlas.texture_view),
                },
                BindGroupEntry {
                    binding: 3,
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
                    blend: Some(BlendState::ALPHA_BLENDING),
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
            params,
            params_buffer,
            pipeline,
            bind_group,
            color_atlas,
            mask_atlas,
        }
    }

    pub(crate) fn contains_cached_glyph(&self, glyph: &CacheKey) -> bool {
        self.mask_atlas.glyph_cache.contains(glyph) || self.color_atlas.glyph_cache.contains(glyph)
    }

    pub(crate) fn glyph(&self, glyph: &CacheKey) -> Option<&GlyphDetails> {
        self.mask_atlas
            .glyph_cache
            .peek(glyph)
            .or_else(|| self.color_atlas.glyph_cache.peek(glyph))
    }

    pub(crate) fn inner_for_content(&self, content_type: ContentType) -> &InnerAtlas {
        match content_type {
            ContentType::Color => &self.color_atlas,
            ContentType::Mask => &self.mask_atlas,
        }
    }

    pub(crate) fn inner_for_content_mut(&mut self, content_type: ContentType) -> &mut InnerAtlas {
        match content_type {
            ContentType::Color => &mut self.color_atlas,
            ContentType::Mask => &mut self.mask_atlas,
        }
    }
}
