use crate::{
    text_render::ContentType, CacheKey, FontSystem, GlyphDetails, GpuCacheStatus, Pipeline,
    SwashCache,
};
use etagere::{size2, Allocation, BucketedAtlasAllocator};
use lru::LruCache;
use rustc_hash::FxHasher;
use std::{collections::HashSet, hash::BuildHasherDefault, sync::Arc};
use wgpu::{
    BindGroup, Buffer, DepthStencilState, Device, Extent3d, ImageCopyTexture, ImageDataLayout,
    MultisampleState, Origin3d, Queue, RenderPipeline, Texture, TextureAspect, TextureDescriptor,
    TextureDimension, TextureFormat, TextureUsages, TextureView, TextureViewDescriptor,
};

type Hasher = BuildHasherDefault<FxHasher>;

#[allow(dead_code)]
pub(crate) struct InnerAtlas {
    pub kind: Kind,
    pub texture: Texture,
    pub texture_view: TextureView,
    pub packer: BucketedAtlasAllocator,
    pub size: u32,
    pub glyph_cache: LruCache<CacheKey, GlyphDetails, Hasher>,
    pub glyphs_in_use: HashSet<CacheKey, Hasher>,
    pub max_texture_dimension_2d: u32,
}

impl InnerAtlas {
    const INITIAL_SIZE: u32 = 256;

    fn new(device: &Device, _queue: &Queue, kind: Kind) -> Self {
        let max_texture_dimension_2d = device.limits().max_texture_dimension_2d;
        let size = Self::INITIAL_SIZE.min(max_texture_dimension_2d);

        let packer = BucketedAtlasAllocator::new(size2(size as i32, size as i32));

        // Create a texture to use for our atlas
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("glyphon atlas"),
            size: Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: kind.texture_format(),
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let texture_view = texture.create_view(&TextureViewDescriptor::default());

        let glyph_cache = LruCache::unbounded_with_hasher(Hasher::default());
        let glyphs_in_use = HashSet::with_hasher(Hasher::default());

        Self {
            kind,
            texture,
            texture_view,
            packer,
            size,
            glyph_cache,
            glyphs_in_use,
            max_texture_dimension_2d,
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
            let (mut key, mut value) = self.glyph_cache.peek_lru()?;

            // Find a glyph with an actual size
            while value.atlas_id.is_none() {
                // All sized glyphs are in use, cache is full
                if self.glyphs_in_use.contains(key) {
                    return None;
                }

                let _ = self.glyph_cache.pop_lru();

                (key, value) = self.glyph_cache.peek_lru()?;
            }

            // All sized glyphs are in use, cache is full
            if self.glyphs_in_use.contains(key) {
                return None;
            }

            let (_, value) = self.glyph_cache.pop_lru().unwrap();
            self.packer.deallocate(value.atlas_id.unwrap());
        }
    }

    pub fn num_channels(&self) -> usize {
        self.kind.num_channels()
    }

    pub(crate) fn promote(&mut self, glyph: CacheKey) {
        self.glyph_cache.promote(&glyph);
        self.glyphs_in_use.insert(glyph);
    }

    pub(crate) fn put(&mut self, glyph: CacheKey, details: GlyphDetails) {
        self.glyph_cache.put(glyph, details);
        self.glyphs_in_use.insert(glyph);
    }

    pub(crate) fn grow(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_system: &mut FontSystem,
        cache: &mut SwashCache,
    ) -> bool {
        if self.size >= self.max_texture_dimension_2d {
            return false;
        }

        // Grow each dimension by a factor of 2. The growth factor was chosen to match the growth
        // factor of `Vec`.`
        const GROWTH_FACTOR: u32 = 2;
        let new_size = (self.size * GROWTH_FACTOR).min(self.max_texture_dimension_2d);

        self.packer.grow(size2(new_size as i32, new_size as i32));

        // Create a texture to use for our atlas
        self.texture = device.create_texture(&TextureDescriptor {
            label: Some("glyphon atlas"),
            size: Extent3d {
                width: new_size,
                height: new_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: self.kind.texture_format(),
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Re-upload glyphs
        for (&cache_key, glyph) in &self.glyph_cache {
            let (x, y) = match glyph.gpu_cache {
                GpuCacheStatus::InAtlas { x, y, .. } => (x, y),
                GpuCacheStatus::SkipRasterization => continue,
            };

            let image = cache.get_image_uncached(font_system, cache_key).unwrap();

            let width = image.placement.width as usize;
            let height = image.placement.height as usize;

            queue.write_texture(
                ImageCopyTexture {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: Origin3d {
                        x: x as u32,
                        y: y as u32,
                        z: 0,
                    },
                    aspect: TextureAspect::All,
                },
                &image.data,
                ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(width as u32 * self.kind.num_channels() as u32),
                    rows_per_image: None,
                },
                Extent3d {
                    width: width as u32,
                    height: height as u32,
                    depth_or_array_layers: 1,
                },
            );
        }

        self.texture_view = self.texture.create_view(&TextureViewDescriptor::default());
        self.size = new_size;

        true
    }

    fn trim(&mut self) {
        self.glyphs_in_use.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Mask,
    Color { srgb: bool },
}

impl Kind {
    fn num_channels(self) -> usize {
        match self {
            Kind::Mask => 1,
            Kind::Color { .. } => 4,
        }
    }

    fn texture_format(self) -> wgpu::TextureFormat {
        match self {
            Kind::Mask => TextureFormat::R8Unorm,
            Kind::Color { srgb } => {
                if srgb {
                    TextureFormat::Rgba8UnormSrgb
                } else {
                    TextureFormat::Rgba8Unorm
                }
            }
        }
    }
}

/// The color mode of an [`Atlas`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// Accurate color management.
    ///
    /// This mode will use a proper sRGB texture for colored glyphs. This will
    /// produce physically accurate color blending when rendering.
    Accurate,

    /// Web color management.
    ///
    /// This mode reproduces the color management strategy used in the Web and
    /// implemented by browsers.
    ///
    /// This entails storing glyphs colored using the sRGB color space in a
    /// linear RGB texture. Blending will not be physically accurate, but will
    /// produce the same results as most UI toolkits.
    ///
    /// This mode should be used to render to a linear RGB texture containing
    /// sRGB colors.
    Web,
}

/// An atlas containing a cache of rasterized glyphs that can be rendered.
pub struct TextAtlas {
    pipeline: Pipeline,
    pub(crate) bind_group: BindGroup,
    pub(crate) color_atlas: InnerAtlas,
    pub(crate) mask_atlas: InnerAtlas,
    pub(crate) format: TextureFormat,
    pub(crate) color_mode: ColorMode,
}

impl TextAtlas {
    /// Creates a new [`TextAtlas`].
    pub fn new(device: &Device, queue: &Queue, pipeline: &Pipeline, format: TextureFormat) -> Self {
        Self::with_color_mode(device, queue, pipeline, format, ColorMode::Accurate)
    }

    /// Creates a new [`TextAtlas`] with the given [`ColorMode`].
    pub fn with_color_mode(
        device: &Device,
        queue: &Queue,
        pipeline: &Pipeline,
        format: TextureFormat,
        color_mode: ColorMode,
    ) -> Self {
        let color_atlas = InnerAtlas::new(
            device,
            queue,
            Kind::Color {
                srgb: match color_mode {
                    ColorMode::Accurate => true,
                    ColorMode::Web => false,
                },
            },
        );
        let mask_atlas = InnerAtlas::new(device, queue, Kind::Mask);

        let bind_group = pipeline.create_atlas_bind_group(
            device,
            &color_atlas.texture_view,
            &mask_atlas.texture_view,
        );

        Self {
            pipeline: pipeline.clone(),
            bind_group,
            color_atlas,
            mask_atlas,
            format,
            color_mode,
        }
    }

    pub fn trim(&mut self) {
        self.mask_atlas.trim();
        self.color_atlas.trim();
    }

    pub(crate) fn grow(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_system: &mut FontSystem,
        cache: &mut SwashCache,
        content_type: ContentType,
    ) -> bool {
        let did_grow = match content_type {
            ContentType::Mask => self.mask_atlas.grow(device, queue, font_system, cache),
            ContentType::Color => self.color_atlas.grow(device, queue, font_system, cache),
        };

        if did_grow {
            self.rebind(device);
        }

        did_grow
    }

    pub(crate) fn glyph(&self, glyph: &CacheKey) -> Option<&GlyphDetails> {
        self.mask_atlas
            .glyph_cache
            .peek(glyph)
            .or_else(|| self.color_atlas.glyph_cache.peek(glyph))
    }

    pub(crate) fn inner_for_content_mut(&mut self, content_type: ContentType) -> &mut InnerAtlas {
        match content_type {
            ContentType::Color => &mut self.color_atlas,
            ContentType::Mask => &mut self.mask_atlas,
        }
    }

    pub(crate) fn get_or_create_pipeline(
        &self,
        device: &Device,
        multisample: MultisampleState,
        depth_stencil: Option<DepthStencilState>,
    ) -> Arc<RenderPipeline> {
        self.pipeline
            .get_or_create(device, self.format, multisample, depth_stencil)
    }

    pub(crate) fn create_uniforms_bind_group(&self, device: &Device, buffer: &Buffer) -> BindGroup {
        self.pipeline.create_uniforms_bind_group(device, buffer)
    }

    fn rebind(&mut self, device: &wgpu::Device) {
        self.bind_group = self.pipeline.create_atlas_bind_group(
            device,
            &self.color_atlas.texture_view,
            &self.mask_atlas.texture_view,
        );
    }
}
