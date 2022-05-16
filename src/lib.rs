use std::{
    borrow::Cow,
    collections::HashSet,
    error::Error,
    fmt::{self, Display, Formatter},
    iter,
    mem::size_of,
    num::{NonZeroU32, NonZeroU64},
    slice,
    sync::{Arc, RwLock},
};

use etagere::{size2, AllocId, Allocation, BucketedAtlasAllocator};
use fontdue::{
    layout::{GlyphRasterConfig, Layout},
    Font,
};
use recently_used::RecentlyUsedMap;
use wgpu::{
    BindGroup, BindGroupEntry, BindGroupLayoutEntry, BindingResource, BindingType, BlendState,
    Buffer, BufferBindingType, BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites,
    Device, Extent3d, FilterMode, FragmentState, ImageCopyTexture, ImageDataLayout, IndexFormat,
    MultisampleState, Origin3d, PipelineLayoutDescriptor, PrimitiveState, Queue, RenderPass,
    RenderPipeline, RenderPipelineDescriptor, SamplerBindingType, SamplerDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, Texture, TextureAspect, TextureDescriptor,
    TextureDimension, TextureFormat, TextureSampleType, TextureUsages, TextureViewDescriptor,
    TextureViewDimension, VertexFormat, VertexState, COPY_BUFFER_ALIGNMENT,
};

pub use fontdue;

mod recently_used;

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
pub enum RenderError {
    RemovedFromAtlas,
    ScreenResolutionChanged,
}

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

fn try_allocate(atlas: &mut InnerAtlas, width: usize, height: usize) -> Option<Allocation> {
    let size = size2(width as i32, height as i32);

    loop {
        let allocation = atlas.packer.allocate(size);
        if allocation.is_some() {
            return allocation;
        }

        // Try to free least recently used allocation
        let (key, value) = atlas.glyph_cache.entries_least_recently_used().next()?;
        atlas
            .packer
            .deallocate(value.atlas_id.expect("cache corrupt"));
        atlas.glyph_cache.remove(&key);
    }
}

struct InnerAtlas {
    texture_pending: Vec<u8>,
    texture: Texture,
    packer: BucketedAtlasAllocator,
    width: u32,
    height: u32,
    glyph_cache: RecentlyUsedMap<GlyphRasterConfig, GlyphDetails>,
    params: Params,
    params_buffer: Buffer,
}

#[derive(Clone)]
pub struct TextAtlas {
    inner: Arc<RwLock<InnerAtlas>>,
    pipeline: Arc<RenderPipeline>,
    bind_group: Arc<BindGroup>,
}

impl TextAtlas {
    pub fn new(device: &Device, _queue: &Queue, format: TextureFormat) -> Self {
        let max_texture_dimension_2d = device.limits().max_texture_dimension_2d;
        let width = max_texture_dimension_2d;
        let height = max_texture_dimension_2d;

        let packer = BucketedAtlasAllocator::new(size2(width as i32, height as i32));
        // Create a texture to use for our atlas
        let texture_pending = vec![0; (width * height) as usize];
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
            format: TextureFormat::R8Unorm,
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
        }));

        Self {
            inner: Arc::new(RwLock::new(InnerAtlas {
                texture_pending,
                texture,
                packer,
                width,
                height,
                glyph_cache,
                params,
                params_buffer,
            })),
            pipeline,
            bind_group,
        }
    }
}

pub struct TextRenderer {
    vertex_buffer: Buffer,
    vertex_buffer_size: u64,
    index_buffer: Buffer,
    index_buffer_size: u64,
    vertices_to_render: u32,
    atlas: TextAtlas,
    glyphs_in_use: HashSet<GlyphRasterConfig>,
    screen_resolution: Resolution,
}

impl TextRenderer {
    pub fn new(device: &Device, _queue: &Queue, atlas: &TextAtlas) -> Self {
        let vertex_buffer_size = next_copy_buffer_size(4096);
        let vertex_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("glyphon vertices"),
            size: vertex_buffer_size,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let index_buffer_size = next_copy_buffer_size(4096);
        let index_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("glyphon indices"),
            size: index_buffer_size,
            usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            vertex_buffer,
            vertex_buffer_size,
            index_buffer,
            index_buffer_size,
            vertices_to_render: 0,
            atlas: atlas.clone(),
            glyphs_in_use: HashSet::new(),
            screen_resolution: Resolution {
                width: 0,
                height: 0,
            },
        }
    }

    pub fn prepare(
        &mut self,
        device: &Device,
        queue: &Queue,
        screen_resolution: Resolution,
        fonts: &[Font],
        layouts: &[Layout<impl HasColor>],
    ) -> Result<(), PrepareError> {
        self.screen_resolution = screen_resolution;

        let atlas_current_resolution = {
            let atlas = self.atlas.inner.read().expect("atlas locked");
            atlas.params.screen_resolution
        };

        if screen_resolution != atlas_current_resolution {
            let mut atlas = self.atlas.inner.write().expect("atlas locked");
            atlas.params.screen_resolution = screen_resolution;
            queue.write_buffer(&atlas.params_buffer, 0, unsafe {
                slice::from_raw_parts(
                    &atlas.params as *const Params as *const u8,
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

        self.glyphs_in_use.clear();

        for layout in layouts.iter() {
            for glyph in layout.glyphs() {
                self.glyphs_in_use.insert(glyph.key);

                let already_on_gpu = self
                    .atlas
                    .inner
                    .read()
                    .expect("atlas locked")
                    .glyph_cache
                    .contains_key(&glyph.key);

                if already_on_gpu {
                    continue;
                }

                let font = &fonts[glyph.font_index];
                let (metrics, bitmap) = font.rasterize_config(glyph.key);

                let mut atlas = self.atlas.inner.write().expect("atlas locked");

                let (gpu_cache, atlas_id) = if glyph.char_data.rasterize() {
                    // Find a position in the packer
                    let allocation = match try_allocate(&mut atlas, metrics.width, metrics.height) {
                        Some(a) => a,
                        None => return Err(PrepareError::AtlasFull),
                    };
                    let atlas_min = allocation.rectangle.min;
                    let atlas_max = allocation.rectangle.max;

                    for row in 0..metrics.height {
                        let y_offset = atlas_min.y as usize;
                        let x_offset =
                            (y_offset + row) * atlas.width as usize + atlas_min.x as usize;
                        let bitmap_row = &bitmap[row * metrics.width..(row + 1) * metrics.width];
                        atlas.texture_pending[x_offset..x_offset + metrics.width]
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

                if !atlas.glyph_cache.contains_key(&glyph.key) {
                    atlas.glyph_cache.insert(
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
        }

        if let Some(ub) = upload_bounds {
            let atlas = self.atlas.inner.read().expect("atlas locked");
            queue.write_texture(
                ImageCopyTexture {
                    texture: &atlas.texture,
                    mip_level: 0,
                    origin: Origin3d {
                        x: ub.x_min as u32,
                        y: ub.y_min as u32,
                        z: 0,
                    },
                    aspect: TextureAspect::All,
                },
                &atlas.texture_pending[ub.y_min * atlas.width as usize + ub.x_min..],
                ImageDataLayout {
                    offset: 0,
                    bytes_per_row: NonZeroU32::new(atlas.width as u32),
                    rows_per_image: NonZeroU32::new(atlas.height as u32),
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
                let atlas = self.atlas.inner.read().expect("atlas locked");
                let details = atlas.glyph_cache.get(&glyph.key).unwrap();
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

            let (buffer, buffer_size) = create_oversized_buffer(
                device,
                Some("glyphon vertices"),
                vertices_raw,
                BufferUsages::VERTEX | BufferUsages::COPY_DST,
            );

            self.vertex_buffer = buffer;
            self.vertex_buffer_size = buffer_size;
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

            let (buffer, buffer_size) = create_oversized_buffer(
                device,
                Some("glyphon indices"),
                indices_raw,
                BufferUsages::INDEX | BufferUsages::COPY_DST,
            );

            self.index_buffer = buffer;
            self.index_buffer_size = buffer_size;
        }

        Ok(())
    }

    pub fn render<'pass>(&'pass mut self, pass: &mut RenderPass<'pass>) -> Result<(), RenderError> {
        if self.vertices_to_render == 0 {
            return Ok(());
        }

        {
            let atlas = self.atlas.inner.read().expect("atlas locked");

            // Validate that glyphs haven't been evicted from cache since `prepare`
            for glyph in self.glyphs_in_use.iter() {
                if !atlas.glyph_cache.contains_key(glyph) {
                    return Err(RenderError::RemovedFromAtlas);
                }
            }

            // Validate that screen resolution hasn't changed since `prepare`
            if self.screen_resolution != atlas.params.screen_resolution {
                return Err(RenderError::ScreenResolutionChanged);
            }
        }

        pass.set_pipeline(&self.atlas.pipeline);
        pass.set_bind_group(0, &self.atlas.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(0..self.vertices_to_render, 0, 0..1);

        Ok(())
    }
}

fn next_copy_buffer_size(size: u64) -> u64 {
    let next_power_of_2 = size.next_power_of_two() as u64;
    let align_mask = COPY_BUFFER_ALIGNMENT - 1;
    let padded_size = ((next_power_of_2 + align_mask) & !align_mask).max(COPY_BUFFER_ALIGNMENT);
    padded_size
}

fn create_oversized_buffer(
    device: &Device,
    label: Option<&str>,
    contents: &[u8],
    usage: BufferUsages,
) -> (Buffer, u64) {
    let size = next_copy_buffer_size(contents.len() as u64);
    let buffer = device.create_buffer(&BufferDescriptor {
        label,
        size,
        usage,
        mapped_at_creation: true,
    });
    buffer.slice(..).get_mapped_range_mut()[..contents.len()].copy_from_slice(contents);
    buffer.unmap();
    (buffer, size)
}
