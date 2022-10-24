use etagere::AllocId;

mod error;
mod recently_used;
mod text_atlas;
mod text_render;

pub use error::{PrepareError, RenderError};
pub use fontdue;
use recently_used::RecentlyUsedMap;
pub use text_atlas::TextAtlas;
pub use text_render::TextRenderer;

/// The color to use when rendering text.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Color {
    /// The red component of the color.
    pub r: u8,
    /// The green component of the color.
    pub g: u8,
    /// The blue component of the color.
    pub b: u8,
    /// The alpha component of the color.
    pub a: u8,
}

/// Allows text to be colored during rendering.
pub trait HasColor: Copy {
    /// The color to use when rendering text.
    fn color(&self) -> Color;
}

pub(crate) enum GpuCache {
    InAtlas { x: u16, y: u16 },
    SkipRasterization,
}

pub(crate) struct GlyphDetails {
    width: u16,
    height: u16,
    gpu_cache: GpuCache,
    atlas_id: Option<AllocId>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct GlyphToRender {
    pos: [u32; 2],
    dim: [u16; 2],
    uv: [u16; 2],
    color: [u8; 4],
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
