use crate::{
    ColorMode, FontSystem, GlyphDetails, GlyphToRender, GpuCacheStatus, Params, PrepareError,
    RenderError, Resolution, SwashCache, SwashContent, TextArea, TextAtlas,
};
use std::{iter, mem::size_of, slice, sync::Arc};
use wgpu::{
    BindGroupDescriptor, BindGroupEntry, Buffer, BufferDescriptor, BufferUsages, DepthStencilState,
    Device, Extent3d, ImageCopyTexture, ImageDataLayout, IndexFormat, MultisampleState, Origin3d,
    Queue, RenderPass, RenderPipeline, TextureAspect, COPY_BUFFER_ALIGNMENT,
};

/// A text renderer that uses cached glyphs to render text into an existing render pass.
pub struct TextRenderer {
    params: Params,
    params_buffer: Buffer,
    vertex_buffer: Buffer,
    vertex_buffer_size: u64,
    index_buffer: Buffer,
    index_buffer_size: u64,
    vertices_to_render: u32,
    pipeline: Arc<RenderPipeline>,
    bind_group: wgpu::BindGroup,
    glyph_vertices: Vec<GlyphToRender>,
    glyph_indices: Vec<u32>,
}

impl TextRenderer {
    /// Creates a new `TextRenderer`.
    pub fn new(
        atlas: &mut TextAtlas,
        device: &Device,
        multisample: MultisampleState,
        depth_stencil: Option<DepthStencilState>,
    ) -> Self {
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

        let pipeline = atlas.get_or_create_pipeline(device, multisample, depth_stencil);

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

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            layout: &atlas.text_render_bind_group_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: params_buffer.as_entire_binding(),
            }],
            label: Some("glyphon text render bind group"),
        });

        Self {
            params,
            params_buffer,
            vertex_buffer,
            vertex_buffer_size,
            index_buffer,
            index_buffer_size,
            vertices_to_render: 0,
            pipeline,
            bind_group,
            glyph_vertices: Vec::new(),
            glyph_indices: Vec::new(),
        }
    }

    /// Prepares all of the provided text areas for rendering.
    pub fn prepare_with_depth<'a>(
        &mut self,
        device: &Device,
        queue: &Queue,
        font_system: &mut FontSystem,
        atlas: &mut TextAtlas,
        screen_resolution: Resolution,
        text_areas: impl IntoIterator<Item = TextArea<'a>>,
        cache: &mut SwashCache,
        mut metadata_to_depth: impl FnMut(usize) -> f32,
    ) -> Result<(), PrepareError> {
        if self.params.screen_resolution != screen_resolution {
            self.params.screen_resolution = screen_resolution;
            queue.write_buffer(&self.params_buffer, 0, unsafe {
                slice::from_raw_parts(
                    &self.params as *const Params as *const u8,
                    size_of::<Params>(),
                )
            });
        }

        self.glyph_vertices.clear();
        self.glyph_indices.clear();
        let mut glyphs_added = 0;

        for text_area in text_areas {
            for run in text_area.buffer.layout_runs() {
                for glyph in run.glyphs.iter() {
                    let physical_glyph =
                        glyph.physical((text_area.left, text_area.top), text_area.scale);

                    if atlas
                        .mask_atlas
                        .glyph_cache
                        .contains(&physical_glyph.cache_key)
                    {
                        atlas.mask_atlas.promote(physical_glyph.cache_key);
                    } else if atlas
                        .color_atlas
                        .glyph_cache
                        .contains(&physical_glyph.cache_key)
                    {
                        atlas.color_atlas.promote(physical_glyph.cache_key);
                    } else {
                        let Some(image) =
                            cache.get_image_uncached(font_system, physical_glyph.cache_key)
                        else {
                            continue;
                        };

                        let content_type = match image.content {
                            SwashContent::Color => ContentType::Color,
                            SwashContent::Mask => ContentType::Mask,
                            SwashContent::SubpixelMask => {
                                // Not implemented yet, but don't panic if this happens.
                                ContentType::Mask
                            }
                        };

                        let width = image.placement.width as usize;
                        let height = image.placement.height as usize;

                        let should_rasterize = width > 0 && height > 0;

                        let (gpu_cache, atlas_id, inner) = if should_rasterize {
                            let mut inner = atlas.inner_for_content_mut(content_type);

                            // Find a position in the packer
                            let allocation = loop {
                                match inner.try_allocate(width, height) {
                                    Some(a) => break a,
                                    None => {
                                        if !atlas.grow(
                                            device,
                                            queue,
                                            font_system,
                                            cache,
                                            content_type,
                                        ) {
                                            return Err(PrepareError::AtlasFull);
                                        }

                                        inner = atlas.inner_for_content_mut(content_type);
                                    }
                                }
                            };
                            let atlas_min = allocation.rectangle.min;

                            queue.write_texture(
                                ImageCopyTexture {
                                    texture: &inner.texture,
                                    mip_level: 0,
                                    origin: Origin3d {
                                        x: atlas_min.x as u32,
                                        y: atlas_min.y as u32,
                                        z: 0,
                                    },
                                    aspect: TextureAspect::All,
                                },
                                &image.data,
                                ImageDataLayout {
                                    offset: 0,
                                    bytes_per_row: Some(width as u32 * inner.num_channels() as u32),
                                    rows_per_image: None,
                                },
                                Extent3d {
                                    width: width as u32,
                                    height: height as u32,
                                    depth_or_array_layers: 1,
                                },
                            );

                            (
                                GpuCacheStatus::InAtlas {
                                    x: atlas_min.x as u16,
                                    y: atlas_min.y as u16,
                                    content_type,
                                },
                                Some(allocation.id),
                                inner,
                            )
                        } else {
                            let inner = &mut atlas.color_atlas;
                            (GpuCacheStatus::SkipRasterization, None, inner)
                        };

                        inner.put(
                            physical_glyph.cache_key,
                            GlyphDetails {
                                width: width as u16,
                                height: height as u16,
                                gpu_cache,
                                atlas_id,
                                top: image.placement.top as i16,
                                left: image.placement.left as i16,
                            },
                        );
                    }

                    let details = atlas.glyph(&physical_glyph.cache_key).unwrap();

                    let mut x = physical_glyph.x + details.left as i32;
                    let mut y = (run.line_y * text_area.scale).round() as i32 + physical_glyph.y
                        - details.top as i32;

                    let (mut atlas_x, mut atlas_y, content_type) = match details.gpu_cache {
                        GpuCacheStatus::InAtlas { x, y, content_type } => (x, y, content_type),
                        GpuCacheStatus::SkipRasterization => continue,
                    };

                    let mut width = details.width as i32;
                    let mut height = details.height as i32;

                    let bounds_min_x = text_area.bounds.left.max(0);
                    let bounds_min_y = text_area.bounds.top.max(0);
                    let bounds_max_x = text_area.bounds.right.min(screen_resolution.width as i32);
                    let bounds_max_y = text_area.bounds.bottom.min(screen_resolution.height as i32);

                    // Starts beyond right edge or ends beyond left edge
                    let max_x = x + width;
                    if x > bounds_max_x || max_x < bounds_min_x {
                        continue;
                    }

                    // Starts beyond bottom edge or ends beyond top edge
                    let max_y = y + height;
                    if y > bounds_max_y || max_y < bounds_min_y {
                        continue;
                    }

                    // Clip left ege
                    if x < bounds_min_x {
                        let right_shift = bounds_min_x - x;

                        x = bounds_min_x;
                        width = max_x - bounds_min_x;
                        atlas_x += right_shift as u16;
                    }

                    // Clip right edge
                    if x + width > bounds_max_x {
                        width = bounds_max_x - x;
                    }

                    // Clip top edge
                    if y < bounds_min_y {
                        let bottom_shift = bounds_min_y - y;

                        y = bounds_min_y;
                        height = max_y - bounds_min_y;
                        atlas_y += bottom_shift as u16;
                    }

                    // Clip bottom edge
                    if y + height > bounds_max_y {
                        height = bounds_max_y - y;
                    }

                    let color = match glyph.color_opt {
                        Some(some) => some,
                        None => text_area.default_color,
                    };

                    let depth = metadata_to_depth(glyph.metadata);

                    self.glyph_vertices.extend(
                        iter::repeat(GlyphToRender {
                            pos: [x, y],
                            dim: [width as u16, height as u16],
                            uv: [atlas_x, atlas_y],
                            color: color.0,
                            content_type_with_srgb: [
                                content_type as u16,
                                match atlas.color_mode {
                                    ColorMode::Accurate => TextColorConversion::ConvertToLinear,
                                    ColorMode::Web => TextColorConversion::None,
                                } as u16,
                            ],
                            depth,
                        })
                        .take(4),
                    );

                    let start = 4 * glyphs_added as u32;
                    self.glyph_indices.extend([
                        start,
                        start + 1,
                        start + 2,
                        start,
                        start + 2,
                        start + 3,
                    ]);

                    glyphs_added += 1;
                }
            }
        }

        const VERTICES_PER_GLYPH: u32 = 6;
        self.vertices_to_render = glyphs_added as u32 * VERTICES_PER_GLYPH;

        let will_render = glyphs_added > 0;
        if !will_render {
            return Ok(());
        }

        let vertices = self.glyph_vertices.as_slice();
        let vertices_raw = unsafe {
            slice::from_raw_parts(
                vertices as *const _ as *const u8,
                std::mem::size_of_val(vertices),
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

        let indices = self.glyph_indices.as_slice();
        let indices_raw = unsafe {
            slice::from_raw_parts(
                indices as *const _ as *const u8,
                std::mem::size_of_val(indices),
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

    pub fn prepare<'a>(
        &mut self,
        device: &Device,
        queue: &Queue,
        font_system: &mut FontSystem,
        atlas: &mut TextAtlas,
        screen_resolution: Resolution,
        text_areas: impl IntoIterator<Item = TextArea<'a>>,
        cache: &mut SwashCache,
    ) -> Result<(), PrepareError> {
        self.prepare_with_depth(
            device,
            queue,
            font_system,
            atlas,
            screen_resolution,
            text_areas,
            cache,
            zero_depth,
        )
    }

    /// Renders all layouts that were previously provided to `prepare`.
    pub fn render<'pass>(
        &'pass self,
        atlas: &'pass TextAtlas,
        pass: &mut RenderPass<'pass>,
    ) -> Result<(), RenderError> {
        if self.vertices_to_render == 0 {
            return Ok(());
        }

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_bind_group(1, &atlas.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(0..self.vertices_to_render, 0, 0..1);

        Ok(())
    }
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ContentType {
    Color = 0,
    Mask = 1,
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum TextColorConversion {
    None = 0,
    ConvertToLinear = 1,
}

fn next_copy_buffer_size(size: u64) -> u64 {
    let align_mask = COPY_BUFFER_ALIGNMENT - 1;
    ((size.next_power_of_two() + align_mask) & !align_mask).max(COPY_BUFFER_ALIGNMENT)
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

fn zero_depth(_: usize) -> f32 {
    0f32
}
