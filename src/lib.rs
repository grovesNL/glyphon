use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    error::Error,
    fmt::{self, Display, Formatter},
    iter,
    mem::size_of,
    num::{NonZeroU32, NonZeroU64},
    slice,
};

use etagere::{size2, AllocId, Allocation, BucketedAtlasAllocator};
use fontdue::{
    layout::{GlyphRasterConfig, Layout},
    Font,
};
use wgpu::{
    util::{BufferInitDescriptor, DeviceExt},
    BindGroup, BindGroupEntry, BindGroupLayoutEntry, BindingResource, BindingType, BlendState,
    Buffer, BufferBindingType, BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites,
    Device, Extent3d, FilterMode, FragmentState, ImageCopyTexture, ImageDataLayout, IndexFormat,
    MultisampleState, Origin3d, PipelineLayoutDescriptor, PrimitiveState, Queue, RenderPass,
    RenderPipeline, RenderPipelineDescriptor, SamplerBindingType, SamplerDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, Texture, TextureAspect, TextureDescriptor,
    TextureDimension, TextureFormat, TextureSampleType, TextureUsages, TextureViewDescriptor,
    TextureViewDimension, VertexFormat, VertexState,
};

pub use fontdue;

#[repr(C)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

pub trait HasColor: Copy {
    fn color(&self) -> Color;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrepareError {
    AtlasFull,
}

impl Display for PrepareError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "prepare error")
    }
}

impl Error for PrepareError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderError {}

impl Display for RenderError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "render error")
    }
}

enum GpuCache {
    InAtlas { x: u16, y: u16 },
    SkipRasterization,
}

struct GlyphDetails {
    width: u16,
    height: u16,
    gpu_cache: GpuCache,
    atlas_id: Option<AllocId>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct GlyphToRender {
    pos: [u32; 2],
    dim: [u16; 2],
    uv: [u16; 2],
    color: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Params {
    screen_resolution: Resolution,
}

fn try_allocate(
    atlas_packer: &mut BucketedAtlasAllocator,
    layout: &Layout<impl HasColor>,
    glyph_cache: &mut HashMap<GlyphRasterConfig, GlyphDetails>,
    width: usize,
    height: usize,
) -> Option<Allocation> {
    let size = size2(width as i32, height as i32);
    let allocation = atlas_packer.allocate(size);

    if allocation.is_some() {
        return allocation;
    }

    // Try to free any allocations not used in the current layout
    let used_glyphs = layout
        .glyphs()
        .iter()
        .map(|gp| gp.key)
        .collect::<HashSet<_>>();

    glyph_cache.retain(|key, details| {
        if used_glyphs.contains(&key) {
            true
        } else {
            if let Some(atlas_id) = details.atlas_id {
                atlas_packer.deallocate(atlas_id)
            }
            false
        }
    });

    // Attempt to reallocate
    atlas_packer.allocate(size)
}

pub struct TextRenderer {
    glyph_cache: HashMap<GlyphRasterConfig, GlyphDetails>,
    atlas_texture_pending: Vec<u8>,
    atlas_texture: Texture,
    atlas_packer: BucketedAtlasAllocator,
    atlas_width: u32,
    atlas_height: u32,
    pipeline: RenderPipeline,
    vertex_buffer: Buffer,
    vertex_buffer_size: u64,
    index_buffer: Buffer,
    index_buffer_size: u64,
    params: Params,
    params_buffer: Buffer,
    bind_group: BindGroup,
    vertices_to_render: u32,
}

impl TextRenderer {
    pub fn new(device: &Device, _queue: &Queue, format: TextureFormat) -> Self {
        let glyph_cache = HashMap::new();
        let max_texture_dimension_2d = device.limits().max_texture_dimension_2d;
        let atlas_width = max_texture_dimension_2d;
        let atlas_height = max_texture_dimension_2d;

        let atlas_packer =
            BucketedAtlasAllocator::new(size2(atlas_width as i32, atlas_height as i32));

        // Create a texture to use for our atlas
        let atlas_texture_pending = vec![0; (atlas_width * atlas_height) as usize];
        let atlas_texture = device.create_texture(&TextureDescriptor {
            label: Some("glyphon atlas"),
            size: Extent3d {
                width: atlas_width,
                height: atlas_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::R8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        });
        let atlas_texture_view = atlas_texture.create_view(&TextureViewDescriptor::default());
        let atlas_sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("glyphon sampler"),
            min_filter: FilterMode::Nearest,
            mag_filter: FilterMode::Nearest,
            mipmap_filter: FilterMode::Nearest,
            lod_min_clamp: 0f32,
            lod_max_clamp: 0f32,
            ..Default::default()
        });

        // Create a render pipeline to use for rendering later
        let shader = device.create_shader_module(&ShaderModuleDescriptor {
            label: Some("glyphon shader"),
            source: ShaderSource::Wgsl(Cow::Borrowed(include_str!("shader.wgsl"))),
        });

        let vertex_buffers = [wgpu::VertexBufferLayout {
            array_stride: size_of::<GlyphToRender>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: VertexFormat::Uint32x2,
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
        };

        let params_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("glyphon params"),
            size: size_of::<Params>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&atlas_texture_view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(&atlas_sampler),
                },
            ],
            label: Some("glyphon bind group"),
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
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
                targets: &[ColorTargetState {
                    format,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::default(),
                }],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        let vertex_buffer_size = 4096;
        let vertex_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("glyphon vertices"),
            size: vertex_buffer_size,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let index_buffer_size = 4096;
        let index_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("glyphon indices"),
            size: index_buffer_size,
            usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            glyph_cache,
            atlas_texture_pending,
            atlas_texture,
            atlas_packer,
            atlas_width,
            atlas_height,
            pipeline,
            vertex_buffer,
            vertex_buffer_size,
            index_buffer,
            index_buffer_size,
            params,
            params_buffer,
            bind_group,
            vertices_to_render: 0,
        }
    }

    pub fn prepare(
        &mut self,
        device: &Device,
        queue: &Queue,
        screen_resolution: Resolution,
        fonts: &[Font],
        layouts: &[&Layout<impl HasColor>],
    ) -> Result<(), PrepareError> {
        if screen_resolution != self.params.screen_resolution {
            self.params.screen_resolution = screen_resolution;
            queue.write_buffer(&self.params_buffer, 0, unsafe {
                slice::from_raw_parts(
                    &self.params as *const Params as *const u8,
                    size_of::<Params>(),
                )
            });
        }

        struct UploadBounds {
            x_min: usize,
            x_max: usize,
            y_min: usize,
            y_max: usize,
        }
        let mut upload_bounds = None::<UploadBounds>;

        for layout in layouts.iter() {
            for glyph in layout.glyphs() {
                let already_on_gpu = self.glyph_cache.contains_key(&glyph.key);
                if already_on_gpu {
                    continue;
                }

                let font = &fonts[glyph.font_index];
                let (metrics, bitmap) = font.rasterize_config(glyph.key);

                let (gpu_cache, atlas_id) = if glyph.char_data.rasterize() {
                    // Find a position in the packer
                    let allocation = match try_allocate(
                        &mut self.atlas_packer,
                        layout,
                        &mut self.glyph_cache,
                        metrics.width,
                        metrics.height,
                    ) {
                        Some(a) => a,
                        None => return Err(PrepareError::AtlasFull),
                    };
                    let atlas_min = allocation.rectangle.min;
                    let atlas_max = allocation.rectangle.max;

                    for row in 0..metrics.height {
                        let y_offset = atlas_min.y as usize;
                        let x_offset =
                            (y_offset + row) * self.atlas_width as usize + atlas_min.x as usize;
                        let bitmap_row = &bitmap[row * metrics.width..(row + 1) * metrics.width];
                        self.atlas_texture_pending[x_offset..x_offset + metrics.width]
                            .copy_from_slice(bitmap_row);
                    }

                    match upload_bounds.as_mut() {
                        Some(ub) => {
                            ub.x_min = ub.x_min.min(atlas_min.x as usize);
                            ub.x_max = ub.x_max.max(atlas_max.x as usize);
                            ub.y_min = ub.y_min.min(atlas_min.y as usize);
                            ub.y_max = ub.y_max.max(atlas_max.y as usize);
                        }
                        None => {
                            upload_bounds = Some(UploadBounds {
                                x_min: atlas_min.x as usize,
                                x_max: atlas_max.x as usize,
                                y_min: atlas_min.y as usize,
                                y_max: atlas_max.y as usize,
                            });
                        }
                    }

                    (
                        GpuCache::InAtlas {
                            x: atlas_min.x as u16,
                            y: atlas_min.y as u16,
                        },
                        Some(allocation.id),
                    )
                } else {
                    (GpuCache::SkipRasterization, None)
                };

                self.glyph_cache.insert(
                    glyph.key,
                    GlyphDetails {
                        width: metrics.width as u16,
                        height: metrics.height as u16,
                        gpu_cache,
                        atlas_id,
                    },
                );
            }
        }

        if let Some(ub) = upload_bounds {
            queue.write_texture(
                ImageCopyTexture {
                    texture: &self.atlas_texture,
                    mip_level: 0,
                    origin: Origin3d {
                        x: ub.x_min as u32,
                        y: ub.y_min as u32,
                        z: 0,
                    },
                    aspect: TextureAspect::All,
                },
                &self.atlas_texture_pending[ub.y_min * self.atlas_width as usize + ub.x_min..],
                ImageDataLayout {
                    offset: 0,
                    bytes_per_row: NonZeroU32::new(self.atlas_width as u32),
                    rows_per_image: NonZeroU32::new(self.atlas_height as u32),
                },
                Extent3d {
                    width: (ub.x_max - ub.x_min) as u32,
                    height: (ub.y_max - ub.y_min) as u32,
                    depth_or_array_layers: 1,
                },
            );
        }

        let mut glyph_vertices = Vec::new();
        let mut glyph_indices = Vec::new();
        let mut glyphs_added = 0;

        for layout in layouts.iter() {
            for glyph in layout.glyphs() {
                let details = self.glyph_cache.get_mut(&glyph.key).unwrap();
                let (atlas_x, atlas_y) = match details.gpu_cache {
                    GpuCache::InAtlas { x, y } => (x, y),
                    GpuCache::SkipRasterization => continue,
                };

                let color = glyph.user_data.color();

                glyph_vertices.extend(
                    iter::repeat(GlyphToRender {
                        // Note: subpixel positioning is not currently handled, so we always use
                        // the nearest pixel.
                        pos: [glyph.x.round() as u32, glyph.y.round() as u32],
                        dim: [details.width, details.height],
                        uv: [atlas_x, atlas_y],
                        color: [color.r, color.g, color.b, color.a],
                    })
                    .take(4),
                );

                let start = 4 * glyphs_added as u32;
                glyph_indices.extend([start, start + 1, start + 2, start, start + 2, start + 3]);

                glyphs_added += 1;
            }
        }

        const VERTICES_PER_GLYPH: u32 = 6;
        self.vertices_to_render = glyphs_added as u32 * VERTICES_PER_GLYPH;

        let will_render = glyphs_added > 0;
        if !will_render {
            return Ok(());
        }

        let vertices = glyph_vertices.as_slice();
        let vertices_raw = unsafe {
            slice::from_raw_parts(
                vertices as *const _ as *const u8,
                size_of::<GlyphToRender>() * vertices.len(),
            )
        };

        if self.vertex_buffer_size >= vertices_raw.len() as u64 {
            queue.write_buffer(&self.vertex_buffer, 0, vertices_raw);
        } else {
            self.vertex_buffer.destroy();
            self.vertex_buffer_size = vertices_raw.len().next_power_of_two() as u64;
            self.vertex_buffer = device.create_buffer_init(&BufferInitDescriptor {
                label: Some("glyphon vertices"),
                contents: vertices_raw,
                usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            });
        }

        let indices = glyph_indices.as_slice();
        let indices_raw = unsafe {
            slice::from_raw_parts(
                indices as *const _ as *const u8,
                size_of::<u32>() * indices.len(),
            )
        };

        if self.index_buffer_size >= indices_raw.len() as u64 {
            queue.write_buffer(&self.index_buffer, 0, indices_raw);
        } else {
            self.index_buffer.destroy();
            self.index_buffer_size = indices_raw.len().next_power_of_two() as u64;
            self.index_buffer = device.create_buffer_init(&BufferInitDescriptor {
                label: Some("glyphon indices"),
                contents: indices_raw,
                usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
            });
        }

        Ok(())
    }

    pub fn render<'pass>(&'pass mut self, pass: &mut RenderPass<'pass>) -> Result<(), ()> {
        if self.vertices_to_render == 0 {
            return Ok(());
        }

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(0..self.vertices_to_render, 0, 0..1);

        Ok(())
    }
}
