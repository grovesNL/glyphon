use crate::{
    GlyphDetails, GlyphToRender, GpuCacheStatus, Params, PrepareError, RenderError, Resolution,
    TextArea, TextAtlas,
};
use cosmic_text::{CacheKey, Color, SwashCache, SwashContent};
use std::{collections::HashSet, iter, mem::size_of, num::NonZeroU32, slice};
use wgpu::{
    Buffer, BufferDescriptor, BufferUsages, Device, Extent3d, ImageCopyTexture, ImageDataLayout,
    IndexFormat, Origin3d, Queue, RenderPass, TextureAspect, COPY_BUFFER_ALIGNMENT,
};

/// A text renderer that uses cached glyphs to render text into an existing render pass.
pub struct TextRenderer {
    vertex_buffer: Buffer,
    vertex_buffer_size: u64,
    index_buffer: Buffer,
    index_buffer_size: u64,
    vertices_to_render: u32,
    glyphs_in_use: HashSet<CacheKey>,
    screen_resolution: Resolution,
}

impl TextRenderer {
    /// Creates a new `TextRenderer`.
    pub fn new(device: &Device, _queue: &Queue) -> Self {
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
            glyphs_in_use: HashSet::new(),
            screen_resolution: Resolution {
                width: 0,
                height: 0,
            },
        }
    }

    /// Prepares all of the provided text areas for rendering.
    pub fn prepare<'a>(
        &mut self,
        device: &Device,
        queue: &Queue,
        atlas: &mut TextAtlas,
        screen_resolution: Resolution,
        text_areas: &[TextArea<'a>],
        default_color: Color,
        cache: &mut SwashCache,
    ) -> Result<(), PrepareError> {
        self.screen_resolution = screen_resolution;

        let atlas_current_resolution = { atlas.params.screen_resolution };

        if screen_resolution != atlas_current_resolution {
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

        struct BoundsPerAtlas {
            color: Option<UploadBounds>,
            mask: Option<UploadBounds>,
        }

        let mut upload_bounds_per_atlas = BoundsPerAtlas {
            color: None,
            mask: None,
        };

        self.glyphs_in_use.clear();

        for text_area in text_areas.iter() {
            for run in text_area.buffer.layout_runs() {
                for glyph in run.glyphs.iter() {
                    self.glyphs_in_use.insert(glyph.cache_key);

                    let already_on_gpu = atlas.contains_cached_glyph(&glyph.cache_key);

                    if already_on_gpu {
                        continue;
                    }

                    let image = cache.get_image_uncached(glyph.cache_key).unwrap();

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
                        let inner = atlas.inner_for_content_mut(content_type);

                        // Find a position in the packer
                        let allocation = match inner.try_allocate(width, height) {
                            Some(a) => a,
                            None => return Err(PrepareError::AtlasFull),
                        };
                        let atlas_min = allocation.rectangle.min;
                        let atlas_max = allocation.rectangle.max;

                        for row in 0..height {
                            let y_offset = atlas_min.y as usize;
                            let x_offset =
                                (y_offset + row) * inner.width as usize + atlas_min.x as usize;
                            let num_atlas_channels = inner.num_atlas_channels;
                            let bitmap_row = &image.data[row * width * num_atlas_channels
                                ..(row + 1) * width * num_atlas_channels];
                            inner.texture_pending[x_offset * num_atlas_channels
                                ..(x_offset + width) * num_atlas_channels]
                                .copy_from_slice(bitmap_row);
                        }

                        let upload_bounds = match content_type {
                            ContentType::Color => &mut upload_bounds_per_atlas.color,
                            ContentType::Mask => &mut upload_bounds_per_atlas.mask,
                        };

                        match upload_bounds.as_mut() {
                            Some(ub) => {
                                ub.x_min = ub.x_min.min(atlas_min.x as usize);
                                ub.x_max = ub.x_max.max(atlas_max.x as usize);
                                ub.y_min = ub.y_min.min(atlas_min.y as usize);
                                ub.y_max = ub.y_max.max(atlas_max.y as usize);
                            }
                            None => {
                                *upload_bounds = Some(UploadBounds {
                                    x_min: atlas_min.x as usize,
                                    x_max: atlas_max.x as usize,
                                    y_min: atlas_min.y as usize,
                                    y_max: atlas_max.y as usize,
                                });
                            }
                        }

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

                    if !inner.glyph_cache.contains_key(&glyph.cache_key) {
                        inner.glyph_cache.insert(
                            glyph.cache_key,
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
                }
            }
        }

        for (content_type, bounds) in [
            (ContentType::Color, upload_bounds_per_atlas.color),
            (ContentType::Mask, upload_bounds_per_atlas.mask),
        ] {
            if let Some(ub) = bounds {
                let inner = atlas.inner_for_content(content_type);
                let num_atlas_channels = inner.num_atlas_channels;
                queue.write_texture(
                    ImageCopyTexture {
                        texture: &inner.texture,
                        mip_level: 0,
                        origin: Origin3d {
                            x: ub.x_min as u32,
                            y: ub.y_min as u32,
                            z: 0,
                        },
                        aspect: TextureAspect::All,
                    },
                    &inner.texture_pending
                        [ub.y_min * inner.width as usize + ub.x_min * num_atlas_channels..],
                    ImageDataLayout {
                        offset: 0,
                        bytes_per_row: NonZeroU32::new(inner.width * num_atlas_channels as u32),
                        rows_per_image: NonZeroU32::new(inner.height),
                    },
                    Extent3d {
                        width: (ub.x_max - ub.x_min) as u32,
                        height: (ub.y_max - ub.y_min) as u32,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }

        let mut glyph_vertices: Vec<GlyphToRender> = Vec::new();
        let mut glyph_indices: Vec<u32> = Vec::new();
        let mut glyphs_added = 0;

        for text_area in text_areas.iter() {
            // Note: subpixel positioning is not currently handled, so we always truncate down to
            // the nearest pixel whenever necessary.
            for run in text_area.buffer.layout_runs() {
                let line_y = run.line_y;

                for glyph in run.glyphs.iter() {
                    let color = match glyph.color_opt {
                        Some(some) => some,
                        None => default_color,
                    };

                    let details = atlas.glyph(&glyph.cache_key).unwrap();

                    let mut x = glyph.x_int + details.left as i32;
                    let mut y = line_y + glyph.y_int - details.top as i32;

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

                    glyph_vertices.extend(
                        iter::repeat(GlyphToRender {
                            pos: [x as i32, y as i32],
                            dim: [width as u16, height as u16],
                            uv: [atlas_x, atlas_y],
                            color: color.0,
                            content_type: content_type as u32,
                        })
                        .take(4),
                    );

                    let start = 4 * glyphs_added as u32;
                    glyph_indices.extend([
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

    /// Renders all layouts that were previously provided to `prepare`.
    pub fn render<'pass>(
        &'pass mut self,
        atlas: &'pass TextAtlas,
        pass: &mut RenderPass<'pass>,
    ) -> Result<(), RenderError> {
        if self.vertices_to_render == 0 {
            return Ok(());
        }

        {
            // Validate that glyphs haven't been evicted from cache since `prepare`
            for glyph in self.glyphs_in_use.iter() {
                if !atlas.contains_cached_glyph(glyph) {
                    return Err(RenderError::RemovedFromAtlas);
                }
            }

            // Validate that screen resolution hasn't changed since `prepare`
            if self.screen_resolution != atlas.params.screen_resolution {
                return Err(RenderError::ScreenResolutionChanged);
            }
        }

        pass.set_pipeline(&atlas.pipeline);
        pass.set_bind_group(0, &atlas.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(0..self.vertices_to_render, 0, 0..1);

        Ok(())
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum ContentType {
    Color = 0,
    Mask = 1,
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
