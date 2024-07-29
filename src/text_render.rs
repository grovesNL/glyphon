use crate::{
    ColorMode, FontSystem, GlyphDetails, GlyphToRender, GpuCacheStatus, PrepareError, RenderError,
    SwashCache, SwashContent, TextArea, TextAtlas, Viewport,
};
use std::{slice, sync::Arc};
use wgpu::{
    Buffer, BufferDescriptor, BufferUsages, DepthStencilState, Device, Extent3d, ImageCopyTexture,
    ImageDataLayout, MultisampleState, Origin3d, Queue, RenderPass, RenderPipeline, TextureAspect,
    COPY_BUFFER_ALIGNMENT,
};

/// A text renderer that uses cached glyphs to render text into an existing render pass.
pub struct TextRenderer {
    vertex_buffer: Buffer,
    vertex_buffer_size: u64,
    pipeline: Arc<RenderPipeline>,
    glyph_vertices: Vec<GlyphToRender>,
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

        let pipeline = atlas.get_or_create_pipeline(device, multisample, depth_stencil);

        Self {
            vertex_buffer,
            vertex_buffer_size,
            pipeline,
            glyph_vertices: Vec::new(),
        }
    }

    /// Prepares all of the provided text areas for rendering.
    pub fn prepare_with_depth<'a>(
        &mut self,
        device: &Device,
        queue: &Queue,
        font_system: &mut FontSystem,
        atlas: &mut TextAtlas,
        viewport: &Viewport,
        text_areas: impl IntoIterator<Item = TextArea<'a>>,
        cache: &mut SwashCache,
        mut metadata_to_depth: impl FnMut(usize) -> f32,
    ) -> Result<(), PrepareError> {
        self.glyph_vertices.clear();

        let resolution = viewport.resolution();

        for text_area in text_areas {
            let bounds_min_x = text_area.bounds.left.max(0);
            let bounds_min_y = text_area.bounds.top.max(0);
            let bounds_max_x = text_area.bounds.right.min(resolution.width as i32);
            let bounds_max_y = text_area.bounds.bottom.min(resolution.height as i32);

            let is_run_visible = |run: &cosmic_text::LayoutRun| {
                let start_y = (text_area.top + run.line_top) as i32;
                let end_y = (text_area.top + run.line_top + run.line_height) as i32;

                start_y <= bounds_max_y && bounds_min_y <= end_y
            };

            let layout_runs = text_area
                .buffer
                .layout_runs()
                .skip_while(|run| !is_run_visible(run))
                .take_while(is_run_visible);

            for run in layout_runs {
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

                    self.glyph_vertices.push(GlyphToRender {
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
                    });
                }
            }
        }

        let will_render = !self.glyph_vertices.is_empty();
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

        Ok(())
    }

    pub fn prepare<'a>(
        &mut self,
        device: &Device,
        queue: &Queue,
        font_system: &mut FontSystem,
        atlas: &mut TextAtlas,
        viewport: &Viewport,
        text_areas: impl IntoIterator<Item = TextArea<'a>>,
        cache: &mut SwashCache,
    ) -> Result<(), PrepareError> {
        self.prepare_with_depth(
            device,
            queue,
            font_system,
            atlas,
            viewport,
            text_areas,
            cache,
            zero_depth,
        )
    }

    /// Renders all layouts that were previously provided to `prepare`.
    pub fn render<'pass>(
        &'pass self,
        atlas: &'pass TextAtlas,
        viewport: &'pass Viewport,
        pass: &mut RenderPass<'pass>,
    ) -> Result<(), RenderError> {
        if self.glyph_vertices.is_empty() {
            return Ok(());
        }

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &atlas.bind_group, &[]);
        pass.set_bind_group(1, &viewport.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..4, 0..self.glyph_vertices.len() as u32);

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
