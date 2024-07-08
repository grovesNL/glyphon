//! Glyphon provides a simple way to render 2D text with [wgpu], [cosmic-text] and [etagere].
//!
//! [wpgu]: https://github.com/gfx-rs/wgpu
//! [cosmic-text]: https://github.com/pop-os/cosmic-text
//! [etagere]: https://github.com/nical/etagere

mod cache;
mod error;
mod text_atlas;
mod text_render;
mod viewport;

#[cfg(feature = "svg")]
pub mod svg;

pub use cache::Cache;
pub use error::{PrepareError, RenderError};
pub use text_atlas::{ColorMode, TextAtlas};
pub use text_render::{ContentType, TextRenderer};
pub use viewport::Viewport;

// Re-export all top-level types from `cosmic-text` for convenience.
#[doc(no_inline)]
pub use cosmic_text::{
    self, fontdb, Action, Affinity, Attrs, AttrsList, AttrsOwned, Buffer, BufferLine, CacheKey,
    Color, Command, Cursor, Edit, Editor, Family, FamilyOwned, Font, FontSystem, LayoutCursor,
    LayoutGlyph, LayoutLine, LayoutRun, LayoutRunIter, Metrics, ShapeGlyph, ShapeLine, ShapeSpan,
    ShapeWord, Shaping, Stretch, Style, SubpixelBin, SwashCache, SwashContent, SwashImage, Weight,
    Wrap,
};

use etagere::AllocId;

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
    content_type_with_srgb: [u16; 2],
    depth: f32,
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

/// Controls the visible area of the text. Any text outside of the visible area will be clipped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextBounds {
    /// The position of the left edge of the visible area.
    pub left: i32,
    /// The position of the top edge of the visible area.
    pub top: i32,
    /// The position of the right edge of the visible area.
    pub right: i32,
    /// The position of the bottom edge of the visible area.
    pub bottom: i32,
}

/// The default visible area doesn't clip any text.
impl Default for TextBounds {
    fn default() -> Self {
        Self {
            left: i32::MIN,
            top: i32::MIN,
            right: i32::MAX,
            bottom: i32::MAX,
        }
    }
}

/// A text area containing text to be rendered along with its overflow behavior.
#[derive(Clone)]
pub struct TextArea<'a> {
    /// The buffer containing the text to be rendered.
    pub buffer: &'a Buffer,
    /// The left edge of the buffer.
    pub left: f32,
    /// The top edge of the buffer.
    pub top: f32,
    /// The scaling to apply to the buffer.
    pub scale: f32,
    /// The visible bounds of the text area. This is used to clip the text and doesn't have to
    /// match the `left` and `top` values.
    pub bounds: TextBounds,
    // The default color of the text area.
    pub default_color: Color,

    // Since this has no effect on text layout yet, only expose this if
    // the custom glyph feature is enabled.
    #[cfg(feature = "custom-glyphs")]
    /// Any additional boxes of non-textual content that is inline with text.
    ///
    /// Note, this currently does not affect layout of text.
    /// (see: https://github.com/pop-os/cosmic-text/issues/80)
    pub inline_boxes: Vec<InlineBox>,
}

// Since this has no effect on text layout yet, only expose this if
// the custom glyph feature is enabled.
#[cfg(feature = "custom-glyphs")]
/// An arbitrary box of non-textual content that is inline with text.
///
/// Note, this currently does not affect layout of text
/// (see: https://github.com/pop-os/cosmic-text/issues/80)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InlineBox {
    /// The position of the left edge of the rectangular area.
    pub left: f32,
    /// The position of the top edge of the rectangular area.
    pub top: f32,
    /// The width of the rectangular area.
    pub width: f32,
    /// The height of the rectangular area.
    pub height: f32,

    /// The content of the box.
    pub content: InlineBoxContent,
}

// Since this has no effect on text layout yet, only expose this if
// the custom glyph feature is enabled.
#[cfg(feature = "custom-glyphs")]
/// The contents of an [`InlineBox`]
#[non_exhaustive]
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub enum InlineBoxContent {
    #[default]
    None,
    CustomGlyph {
        /// The unique identifier for this glyph
        id: CustomGlyphID,
        /// The size of the glyph
        size: f32,
        /// The x offset of the glyph relative to the box
        left: f32,
        /// The y offset of the glyph relative to the box
        top: f32,
        /// The color of this glyph (only relevant if the glyph is rendered with the
        /// type [`ContentType::Mask`])
        ///
        /// Set to `None` to use [`TextArea::default_color`].
        color: Option<Color>,
        /// Additional metadata about the glyph
        metadata: usize,
    },
}

#[cfg(feature = "custom-glyphs")]
pub type CustomGlyphID = u16;
