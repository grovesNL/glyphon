//! Glyphon provides a simple way to render 2D text with [wgpu], [cosmic-text] and [etagere].
//!
//! [wpgu]: https://github.com/gfx-rs/wgpu
//! [cosmic-text]: https://github.com/pop-os/cosmic-text
//! [etagere]: https://github.com/nical/etagere

mod error;
mod text_atlas;
mod text_render;

pub use error::{PrepareError, RenderError};
pub use text_atlas::{ColorMode, TextAtlas};
pub use text_render::TextRenderer;

use text_render::ContentType;

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

#[derive(Clone, Debug)]
pub enum Screen {
    Resolution(Resolution),
    CustomTransformation(Matrix),
}

impl Screen {
    fn matrix(&self) -> Matrix {
        match self {
            Screen::Resolution(resolution) => {
                #[rustfmt::skip]
                let matrix = [
                    [2. / resolution.width as f32,                             0., -1.0],
                    [                           0., 2. / resolution.height as f32, -1.0],
                    [                           0.,                            0.,  1.0],
                ];
                Matrix { matrix }
            }
            Screen::CustomTransformation(matrix) => *matrix,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Matrix {
    pub matrix: [[f32; 3]; 3],
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
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub(crate) struct Params {
    transform: [f32; (3 + 1) * 3],
}

impl From<&Screen> for Params {
    fn from(value: &Screen) -> Self {
        value.matrix().into()
    }
}

impl From<Matrix> for Params {
    fn from(value: Matrix) -> Self {
        Self {
            // wgsl matrix is column-major
            transform: std::array::from_fn(|idx| {
                let row = idx % 4;
                let col = idx / 4;
                if row == 3 {
                    0.
                } else {
                    value.matrix[row][col]
                }
            }),
        }
    }
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
}
