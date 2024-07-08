use cosmic_text::{CacheKey, CacheKeyFlags, Color, SubpixelBin};
use resvg::{
    tiny_skia::Pixmap,
    usvg::{self, Transform},
};
use rustc_hash::FxHashMap;
use std::{path::Path, slice, sync::Arc};
use wgpu::{
    Buffer, BufferDescriptor, BufferUsages, DepthStencilState, Device, Extent3d, ImageCopyTexture,
    ImageDataLayout, MultisampleState, Origin3d, Queue, RenderPass, RenderPipeline, TextureAspect,
};

use crate::{
    text_render::{ContentType, TextColorConversion},
    ColorMode, FontSystem, GlyphDetails, GlyphToRender, GpuCacheStatus, PrepareError, RenderError,
    SwashCache, TextAtlas, TextBounds, Viewport,
};

/// An svg icon renderer that uses cached glyphs to render icons into an existing render pass.
pub struct IconRenderer {
    vertex_buffer: Buffer,
    vertex_buffer_size: u64,
    pipeline: Arc<RenderPipeline>,
    glyph_vertices: Vec<GlyphToRender>,
}

impl IconRenderer {
    /// Creates a new [`IconRenderer`].
    pub fn new(
        atlas: &mut TextAtlas,
        device: &Device,
        multisample: MultisampleState,
        depth_stencil: Option<DepthStencilState>,
    ) -> Self {
        let vertex_buffer_size = crate::text_render::next_copy_buffer_size(32);
        let vertex_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("glyphon icon vertices"),
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

    /// Prepares all of the given icons for rendering.
    pub fn prepare_with_depth<'a>(
        &mut self,
        device: &Device,
        queue: &Queue,
        icon_system: &mut IconSystem,
        font_system: &mut FontSystem,
        atlas: &mut TextAtlas,
        viewport: &Viewport,
        icons: impl IntoIterator<Item = IconDesc>,
        cache: &mut SwashCache,
        mut metadata_to_depth: impl FnMut(usize) -> f32,
    ) -> Result<(), PrepareError> {
        self.glyph_vertices.clear();

        let resolution = viewport.resolution();

        let font_id = cosmic_text::fontdb::ID::dummy();
        
        let flags = CacheKeyFlags::from_bits_retain(u32::MAX);

        for icon in icons {
            let cache_key = CacheKey {
                font_id,
                glyph_id: icon.id.0,
                font_size_bits: icon.size.to_bits(),
                x_bin: SubpixelBin::Zero,
                y_bin: SubpixelBin::Zero,
                flags,
            };

            if atlas.mask_atlas.glyph_cache.contains(&cache_key) {
                atlas.mask_atlas.promote(cache_key);
            } else if atlas.color_atlas.glyph_cache.contains(&cache_key) {
                atlas.color_atlas.promote(cache_key);
            } else {
                let Some(svg_data) = icon_system.svgs.get(&icon.id) else {
                    continue;
                };

                let content_type = if svg_data.is_symbolic {
                    ContentType::Mask
                } else {
                    ContentType::Color
                };

                let icon_size = svg_data.tree.size();
                let max_side_len = icon_size.width().max(icon_size.height());

                let should_rasterize = max_side_len > 0.0;

                let (scale, width, height, mut pixmap) = if should_rasterize {
                    let scale = icon.size / max_side_len;
                    let width = (icon_size.width() * scale).ceil();
                    let height = (icon_size.height() * scale).ceil();

                    if width <= 0.0 || height <= 0.0 {
                        (0.0, 0, 0, None)
                    } else if let Some(pixmap) = Pixmap::new(width as u32, height as u32) {
                        (scale, width as u32, height as u32, Some(pixmap))
                    } else {
                        (0.0, 0, 0, None)
                    }
                } else {
                    (0.0, 0, 0, None)
                };

                let (gpu_cache, atlas_id, inner) = if let Some(mut pixmap) = pixmap.take() {
                    let transform = Transform::from_scale(scale, scale);

                    resvg::render(&svg_data.tree, transform, &mut pixmap.as_mut());

                    let alpha_image: Vec<u8>;
                    let data = if let ContentType::Mask = content_type {
                        // Only use the alpha channel for symbolic icons.
                        alpha_image = pixmap.data().iter().skip(3).step_by(4).copied().collect();
                        &alpha_image
                    } else {
                        pixmap.data()
                    };

                    let mut inner = atlas.inner_for_content_mut(content_type);

                    // Find a position in the packer
                    let allocation = loop {
                        match inner.try_allocate(width as usize, height as usize) {
                            Some(a) => break a,
                            None => {
                                if !atlas.grow(device, queue, font_system, cache, content_type) {
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
                        data,
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
                    cache_key,
                    GlyphDetails {
                        width: width as u16,
                        height: height as u16,
                        gpu_cache,
                        atlas_id,
                        top: 0,
                        left: 0,
                    },
                );
            }

            let details = atlas.glyph(&cache_key).unwrap();

            let mut x = icon.left;
            let mut y = icon.top;

            let (mut atlas_x, mut atlas_y, content_type) = match details.gpu_cache {
                GpuCacheStatus::InAtlas { x, y, content_type } => (x, y, content_type),
                GpuCacheStatus::SkipRasterization => continue,
            };

            let mut width = details.width as i32;
            let mut height = details.height as i32;

            let bounds_min_x = icon.bounds.left.max(0);
            let bounds_min_y = icon.bounds.top.max(0);
            let bounds_max_x = icon.bounds.right.min(resolution.width as i32);
            let bounds_max_y = icon.bounds.bottom.min(resolution.height as i32);

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

            let depth = metadata_to_depth(icon.metadata);

            self.glyph_vertices.push(GlyphToRender {
                pos: [x, y],
                dim: [width as u16, height as u16],
                uv: [atlas_x, atlas_y],
                color: icon.color.0,
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

            let (buffer, buffer_size) = crate::text_render::create_oversized_buffer(
                device,
                Some("glyphon icon vertices"),
                vertices_raw,
                BufferUsages::VERTEX | BufferUsages::COPY_DST,
            );

            self.vertex_buffer = buffer;
            self.vertex_buffer_size = buffer_size;
        }

        Ok(())
    }

    /// Prepares all of the given icons for rendering.
    pub fn prepare<'a>(
        &mut self,
        device: &Device,
        queue: &Queue,
        icon_system: &mut IconSystem,
        font_system: &mut FontSystem,
        atlas: &mut TextAtlas,
        viewport: &Viewport,
        icons: impl IntoIterator<Item = IconDesc>,
        cache: &mut SwashCache,
    ) -> Result<(), PrepareError> {
        self.prepare_with_depth(
            device,
            queue,
            icon_system,
            font_system,
            atlas,
            viewport,
            icons,
            cache,
            zero_depth,
        )
    }

    /// Renders all icons that were previously provided to [`IconRenderer::prepare`].
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

/// The description of an icon to be rendered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconDesc {
    /// The unique identifier for the source of data to use for this icon.
    pub id: IconSourceID,
    /// The size of the icon in points. This will be the length of the longest side.
    pub size: f32,
    /// The left edge of the icon.
    pub left: i32,
    /// The top edge of the icon.
    pub top: i32,
    /// The color of the icon. This is only relevant if the icon source data is symbolic.
    pub color: Color,
    /// The visible bounds of the text area. This is used to clip the icon and doesn't have to
    /// match the `left` and `top` values.
    pub bounds: TextBounds,
    /// Additional metadata about this icon.
    pub metadata: usize,
}

/// A unique identifier for a given source of icon data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IconSourceID(pub u16);

struct IconData {
    tree: usvg::Tree,
    is_symbolic: bool,
}

/// A system of loaded resources for icons.
pub struct IconSystem {
    svgs: FxHashMap<IconSourceID, IconData>,
}

impl IconSystem {
    /// Construct a new [`IconSystem`].
    pub fn new() -> Self {
        Self {
            svgs: FxHashMap::default(),
        }
    }

    /// Add an svg source to this system.
    ///
    /// * id - A unique identifier for this resource.
    /// * source - The parsed SVG data.
    /// * is_symbolic - If `true`, then only the alpha channel will be used and the icon can
    /// be filled with any solid color. If `false`, then the icon will be rendered in full
    /// color.
    pub fn add_svg(&mut self, id: IconSourceID, source: usvg::Tree, is_symbolic: bool) {
        self.svgs.insert(
            id,
            IconData {
                tree: source,
                is_symbolic,
            },
        );
    }

    // Returns `true` if the source was removed, or `false` if there was
    // no source with that ID.
    pub fn remove(&mut self, id: IconSourceID) -> bool {
        self.svgs.remove(&id).is_some()
    }
}

fn zero_depth(_: usize) -> f32 {
    0f32
}
