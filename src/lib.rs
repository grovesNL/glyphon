use etagere::AllocId;

mod error;
mod recently_used;
mod text_atlas;
mod text_render;

pub use error::{PrepareError, RenderError};
use recently_used::RecentlyUsedMap;
pub use text_atlas::TextAtlas;
use text_render::ContentType;
pub use text_render::TextRenderer;

pub use cosmic_text;

pub(crate) enum GpuCacheStatus {
    InAtlas {
        x: u16,
        y: u16,
        content_type: ContentType,
    },
    SkipRasterization,
}

pub(crate) struct GlyphDetails {
    width: u16,
    height: u16,
    gpu_cache: GpuCacheStatus,
    atlas_id: Option<AllocId>,
    top: i16,
    left: i16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct GlyphToRender {
    pos: [i32; 2],
    dim: [u16; 2],
    uv: [u16; 2],
    color: u32,
    content_type: u32,
}

/// The screen resolution to use when rendering text.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Resolution {
    /// The width of the screen in pixels.
    pub width: u32,
    /// The height of the screen in pixels.
    pub height: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Params {
    screen_resolution: Resolution,
    _pad: [u32; 2],
}

/// Controls the overflow behavior of any glyphs that are outside of the layout bounds.
pub enum TextOverflow {
    /// Glyphs can overflow the bounds.
    Overflow,
    /// Hide any glyphs outside the bounds. If a glyph is partially outside the bounds, it will be
    /// clipped to the bounds.
    Hide,
}
