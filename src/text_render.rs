use crate::{
    custom_glyph::CustomGlyphCacheKey, ColorMode, ContentType, FontSystem, GlyphDetails,
    GlyphToRender, GpuCacheStatus, PrepareError, RasterizeCustomGlyphRequest,
    RasterizedCustomGlyph, RenderError, SwashCache, SwashContent, TextArea, TextAtlas, Viewport,
};
use cosmic_text::{Color, SubpixelBin};

#[cfg(feature = "egui")]
use egui_wgpu::wgpu::{
    Buffer, BufferDescriptor, BufferUsages, DepthStencilState, Device, Extent3d, ImageCopyTexture,
    ImageDataLayout, MultisampleState, Origin3d, Queue, RenderPass, RenderPipeline, TextureAspect,
    COPY_BUFFER_ALIGNMENT,
};

use std::{slice, sync::Arc};
#[cfg(not(feature = "egui"))]
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
        self.prepare_with_depth_and_custom(
            device,
            queue,
            font_system,
            atlas,
            viewport,
            text_areas,
            cache,
            zero_depth,
            |_| None,
        )
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
        metadata_to_depth: impl FnMut(usize) -> f32,
    ) -> Result<(), PrepareError> {
        self.prepare_with_depth_and_custom(
            device,
            queue,
            font_system,
            atlas,
            viewport,
            text_areas,
            cache,
            metadata_to_depth,
            |_| None,
        )
    }

    /// Prepares all of the provided text areas for rendering.
    pub fn prepare_with_custom<'a>(
        &mut self,
        device: &Device,
        queue: &Queue,
        font_system: &mut FontSystem,
        atlas: &mut TextAtlas,
        viewport: &Viewport,
        text_areas: impl IntoIterator<Item = TextArea<'a>>,
        cache: &mut SwashCache,
        rasterize_custom_glyph: impl FnMut(RasterizeCustomGlyphRequest) -> Option<RasterizedCustomGlyph>,
    ) -> Result<(), PrepareError> {
        self.prepare_with_depth_and_custom(
            device,
            queue,
            font_system,
            atlas,
            viewport,
            text_areas,
            cache,
            zero_depth,
            rasterize_custom_glyph,
        )
    }

    /// Prepares all of the provided text areas for rendering.
    pub fn prepare_with_depth_and_custom<'a>(
        &mut self,
        device: &Device,
        queue: &Queue,
        font_system: &mut FontSystem,
        atlas: &mut TextAtlas,
        viewport: &Viewport,
        text_areas: impl IntoIterator<Item = TextArea<'a>>,
        cache: &mut SwashCache,
        mut metadata_to_depth: impl FnMut(usize) -> f32,
        mut rasterize_custom_glyph: impl FnMut(
            RasterizeCustomGlyphRequest,
        ) -> Option<RasterizedCustomGlyph>,
    ) -> Result<(), PrepareError> {
        self.glyph_vertices.clear();

        let resolution = viewport.resolution();

        for text_area in text_areas {
            let bounds_min_x = text_area.bounds.left.max(0);
            let bounds_min_y = text_area.bounds.top.max(0);
            let bounds_max_x = text_area.bounds.right.min(resolution.width as i32);
            let bounds_max_y = text_area.bounds.bottom.min(resolution.height as i32);

            for glyph in text_area.custom_glyphs.iter() {
                let x = text_area.left + (glyph.left * text_area.scale);
                let y = text_area.top + (glyph.top * text_area.scale);
                let width = (glyph.width * text_area.scale).round() as u16;
                let height = (glyph.height * text_area.scale).round() as u16;

                let (x, y, x_bin, y_bin) = if glyph.snap_to_physical_pixel {
                    (
                        x.round() as i32,
                        y.round() as i32,
                        SubpixelBin::Zero,
                        SubpixelBin::Zero,
                    )
                } else {
                    let (x, x_bin) = SubpixelBin::new(x);
                    let (y, y_bin) = SubpixelBin::new(y);
                    (x, y, x_bin, y_bin)
                };

                let cache_key = GlyphonCacheKey::Custom(CustomGlyphCacheKey {
                    glyph_id: glyph.id,
                    width,
                    height,
                    x_bin,
                    y_bin,
                });

                let color = glyph.color.unwrap_or(text_area.default_color);

                if let Some(glyph_to_render) = prepare_glyph(
                    x,
                    y,
                    0.0,
                    color,
                    glyph.metadata,
                    cache_key,
                    atlas,
                    device,
                    queue,
                    cache,
                    font_system,
                    text_area.scale,
                    bounds_min_x,
                    bounds_min_y,
                    bounds_max_x,
                    bounds_max_y,
                    |_cache, _font_system, rasterize_custom_glyph| -> Option<GetGlyphImageResult> {
                        if width == 0 || height == 0 {
                            return None;
                        }

                        let input = RasterizeCustomGlyphRequest {
                            id: glyph.id,
                            width,
                            height,
                            x_bin,
                            y_bin,
                            scale: text_area.scale,
                        };

                        let output = (rasterize_custom_glyph)(input)?;

                        output.validate(&input, None);

                        Some(GetGlyphImageResult {
                            content_type: output.content_type,
                            top: 0,
                            left: 0,
                            width,
                            height,
                            data: output.data,
                        })
                    },
                    &mut metadata_to_depth,
                    &mut rasterize_custom_glyph,
                )? {
                    self.glyph_vertices.push(glyph_to_render);
                }
            }

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

                    let color = match glyph.color_opt {
                        Some(some) => some,
                        None => text_area.default_color,
                    };

                    if let Some(glyph_to_render) = prepare_glyph(
                        physical_glyph.x,
                        physical_glyph.y,
                        run.line_y,
                        color,
                        glyph.metadata,
                        GlyphonCacheKey::Text(physical_glyph.cache_key),
                        atlas,
                        device,
                        queue,
                        cache,
                        font_system,
                        text_area.scale,
                        bounds_min_x,
                        bounds_min_y,
                        bounds_max_x,
                        bounds_max_y,
                        |cache,
                         font_system,
                         _rasterize_custom_glyph|
                         -> Option<GetGlyphImageResult> {
                            let image =
                                cache.get_image_uncached(font_system, physical_glyph.cache_key)?;

                            let content_type = match image.content {
                                SwashContent::Color => ContentType::Color,
                                SwashContent::Mask => ContentType::Mask,
                                SwashContent::SubpixelMask => {
                                    // Not implemented yet, but don't panic if this happens.
                                    ContentType::Mask
                                }
                            };

                            Some(GetGlyphImageResult {
                                content_type,
                                top: image.placement.top as i16,
                                left: image.placement.left as i16,
                                width: image.placement.width as u16,
                                height: image.placement.height as u16,
                                data: image.data,
                            })
                        },
                        &mut metadata_to_depth,
                        &mut rasterize_custom_glyph,
                    )? {
                        self.glyph_vertices.push(glyph_to_render);
                    }
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

    /// Renders all layouts that were previously provided to `prepare`.
    pub fn render(
        &self,
        atlas: &TextAtlas,
        viewport: &Viewport,
        pass: &mut RenderPass<'_>,
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
enum TextColorConversion {
    None = 0,
    ConvertToLinear = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum GlyphonCacheKey {
    Text(cosmic_text::CacheKey),
    Custom(CustomGlyphCacheKey),
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

struct GetGlyphImageResult {
    content_type: ContentType,
    top: i16,
    left: i16,
    width: u16,
    height: u16,
    data: Vec<u8>,
}

fn prepare_glyph<R>(
    x: i32,
    y: i32,
    line_y: f32,
    color: Color,
    metadata: usize,
    cache_key: GlyphonCacheKey,
    atlas: &mut TextAtlas,
    device: &Device,
    queue: &Queue,
    cache: &mut SwashCache,
    font_system: &mut FontSystem,
    scale_factor: f32,
    bounds_min_x: i32,
    bounds_min_y: i32,
    bounds_max_x: i32,
    bounds_max_y: i32,
    get_glyph_image: impl FnOnce(
        &mut SwashCache,
        &mut FontSystem,
        &mut R,
    ) -> Option<GetGlyphImageResult>,
    mut metadata_to_depth: impl FnMut(usize) -> f32,
    mut rasterize_custom_glyph: R,
) -> Result<Option<GlyphToRender>, PrepareError>
where
    R: FnMut(RasterizeCustomGlyphRequest) -> Option<RasterizedCustomGlyph>,
{
    let details = if let Some(details) = atlas.mask_atlas.glyph_cache.get(&cache_key) {
        atlas.mask_atlas.glyphs_in_use.insert(cache_key);
        details
    } else if let Some(details) = atlas.color_atlas.glyph_cache.get(&cache_key) {
        atlas.color_atlas.glyphs_in_use.insert(cache_key);
        details
    } else {
        let Some(image) = (get_glyph_image)(cache, font_system, &mut rasterize_custom_glyph) else {
            return Ok(None);
        };

        let should_rasterize = image.width > 0 && image.height > 0;

        let (gpu_cache, atlas_id, inner) = if should_rasterize {
            let mut inner = atlas.inner_for_content_mut(image.content_type);

            // Find a position in the packer
            let allocation = loop {
                match inner.try_allocate(image.width as usize, image.height as usize) {
                    Some(a) => break a,
                    None => {
                        if !atlas.grow(
                            device,
                            queue,
                            font_system,
                            cache,
                            image.content_type,
                            scale_factor,
                            &mut rasterize_custom_glyph,
                        ) {
                            return Err(PrepareError::AtlasFull);
                        }

                        inner = atlas.inner_for_content_mut(image.content_type);
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
                    bytes_per_row: Some(image.width as u32 * inner.num_channels() as u32),
                    rows_per_image: None,
                },
                Extent3d {
                    width: image.width as u32,
                    height: image.height as u32,
                    depth_or_array_layers: 1,
                },
            );

            (
                GpuCacheStatus::InAtlas {
                    x: atlas_min.x as u16,
                    y: atlas_min.y as u16,
                    content_type: image.content_type,
                },
                Some(allocation.id),
                inner,
            )
        } else {
            let inner = &mut atlas.color_atlas;
            (GpuCacheStatus::SkipRasterization, None, inner)
        };

        inner.glyphs_in_use.insert(cache_key);
        // Insert the glyph into the cache and return the details reference
        inner.glyph_cache.get_or_insert(cache_key, || GlyphDetails {
            width: image.width,
            height: image.height,
            gpu_cache,
            atlas_id,
            top: image.top,
            left: image.left,
        })
    };

    let mut x = x + details.left as i32;
    let mut y = (line_y * scale_factor).round() as i32 + y - details.top as i32;

    let (mut atlas_x, mut atlas_y, content_type) = match details.gpu_cache {
        GpuCacheStatus::InAtlas { x, y, content_type } => (x, y, content_type),
        GpuCacheStatus::SkipRasterization => return Ok(None),
    };

    let mut width = details.width as i32;
    let mut height = details.height as i32;

    // Starts beyond right edge or ends beyond left edge
    let max_x = x + width;
    if x > bounds_max_x || max_x < bounds_min_x {
        return Ok(None);
    }

    // Starts beyond bottom edge or ends beyond top edge
    let max_y = y + height;
    if y > bounds_max_y || max_y < bounds_min_y {
        return Ok(None);
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

    let depth = metadata_to_depth(metadata);

    Ok(Some(GlyphToRender {
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
    }))
}
