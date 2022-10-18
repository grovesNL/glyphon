use etagere::{size2, Allocation};
use fontdue::{
    layout::{GlyphRasterConfig, Layout},
    Font,
};
use std::{collections::HashSet, iter, mem::size_of, num::NonZeroU32, slice};
use wgpu::{
    Buffer, BufferDescriptor, BufferUsages, Device, Extent3d, ImageCopyTexture, ImageDataLayout,
    IndexFormat, Origin3d, Queue, RenderPass, TextureAspect, COPY_BUFFER_ALIGNMENT,
};

use crate::{
    GlyphDetails, GlyphToRender, GpuCache, HasColor, Params, PrepareError, RenderError, Resolution,
    TextAtlas, TextOverflow,
};

pub struct TextRenderer {
    vertex_buffer: Buffer,
    vertex_buffer_size: u64,
    index_buffer: Buffer,
    index_buffer_size: u64,
    vertices_to_render: u32,
    glyphs_in_use: HashSet<GlyphRasterConfig>,
    screen_resolution: Resolution,
}

impl TextRenderer {
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

    pub fn prepare(
        &mut self,
        device: &Device,
        queue: &Queue,
        atlas: &mut TextAtlas,
        screen_resolution: Resolution,
        fonts: &[Font],
        layouts: &[(Layout<impl HasColor>, TextOverflow)],
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
        let mut upload_bounds = None::<UploadBounds>;

        self.glyphs_in_use.clear();

        for (layout, _) in layouts.iter() {
            for glyph in layout.glyphs() {
                self.glyphs_in_use.insert(glyph.key);

                let already_on_gpu = atlas.glyph_cache.contains_key(&glyph.key);

                if already_on_gpu {
                    continue;
                }

                let font = &fonts[glyph.font_index];
                let (metrics, bitmap) = font.rasterize_config(glyph.key);

                let (gpu_cache, atlas_id) = if glyph.char_data.rasterize() {
                    // Find a position in the packer
                    let allocation = match try_allocate(atlas, metrics.width, metrics.height) {
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
                    bytes_per_row: NonZeroU32::new(atlas.width),
                    rows_per_image: NonZeroU32::new(atlas.height),
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

        for (layout, overflow) in layouts.iter() {
            let settings = layout.settings();

            // Note: subpixel positioning is not currently handled, so we always truncate down to
            // the nearest pixel.
            let bounds_min_x = settings.x.trunc() as u32;
            let bounds_max_x = settings
                .max_width
                .map(|w| bounds_min_x + w.trunc() as u32)
                .unwrap_or(u32::MAX);
            let bounds_min_y = settings.y.trunc() as u32;
            let bounds_max_y = settings
                .max_height
                .map(|h| bounds_min_y + h.trunc() as u32)
                .unwrap_or(u32::MAX);

            for glyph in layout.glyphs() {
                let mut x = glyph.x.trunc() as u32;
                let mut y = glyph.y.trunc() as u32;

                let details = atlas.glyph_cache.get(&glyph.key).unwrap();
                let (mut atlas_x, mut atlas_y) = match details.gpu_cache {
                    GpuCache::InAtlas { x, y } => (x, y),
                    GpuCache::SkipRasterization => continue,
                };

                let mut width = details.width as u32;
                let mut height = details.height as u32;

                match overflow {
                    TextOverflow::Overflow => {}
                    TextOverflow::Hide => {
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
                    }
                }

                let color = glyph.user_data.color();

                glyph_vertices.extend(
                    iter::repeat(GlyphToRender {
                        pos: [x, y],
                        dim: [width as u16, height as u16],
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
                if !atlas.glyph_cache.contains_key(glyph) {
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

fn try_allocate(atlas: &mut TextAtlas, width: usize, height: usize) -> Option<Allocation> {
    let size = size2(width as i32, height as i32);

    loop {
        let allocation = atlas.packer.allocate(size);
        if allocation.is_some() {
            return allocation;
        }

        // Try to free least recently used allocation
        let (key, value) = atlas.glyph_cache.pop()?;
        atlas
            .packer
            .deallocate(value.atlas_id.expect("cache corrupt"));
        atlas.glyph_cache.remove(&key);
    }
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
