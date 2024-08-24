use crate::Color;
use cosmic_text::SubpixelBin;

pub type CustomGlyphId = u16;

/// A custom glyph to render
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct CustomGlyph {
    /// The unique identifier for this glyph
    pub id: CustomGlyphId,
    /// The position of the left edge of the glyph
    pub left: f32,
    /// The position of the top edge of the glyph
    pub top: f32,
    /// The width of the glyph
    pub width: f32,
    /// The height of the glyph
    pub height: f32,
    /// The color of this glyph (only relevant if the glyph is rendered with the
    /// type [`ContentType::Mask`])
    ///
    /// Set to `None` to use [`TextArea::default_color`].
    pub color: Option<Color>,
    /// If `true`, then this glyph will be snapped to the nearest whole physical
    /// pixel and the resulting `SubpixelBin`'s in `RasterizationRequest` will always
    /// be `Zero` (useful for images and other large glyphs).
    pub snap_to_physical_pixel: bool,
    /// Additional metadata about the glyph
    pub metadata: usize,
}

/// A request to rasterize a custom glyph
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RasterizationRequest {
    /// The unique identifier of the glyph
    pub id: CustomGlyphId,
    /// The width of the glyph in physical pixels
    pub width: u16,
    /// The height of the glyph in physical pixels
    pub height: u16,
    /// Binning of fractional X offset
    ///
    /// If `CustomGlyph::snap_to_physical_pixel` was set to `true`, then this
    /// will always be `Zero`.
    pub x_bin: SubpixelBin,
    /// Binning of fractional Y offset
    ///
    /// If `CustomGlyph::snap_to_physical_pixel` was set to `true`, then this
    /// will always be `Zero`.
    pub y_bin: SubpixelBin,
    /// The scaling factor applied to the text area (Note that `width` and
    /// `height` are already scaled by this factor.)
    pub scale: f32,
}

/// A rasterized custom glyph
#[derive(Debug, Clone)]
pub struct RasterizedCustomGlyph {
    /// The raw image data
    pub data: Vec<u8>,
    /// The type of image data contained in `data`
    pub content_type: ContentType,
}

impl RasterizedCustomGlyph {
    pub(crate) fn validate(&self, input: &RasterizationRequest, expected_type: Option<ContentType>) {
        if let Some(expected_type) = expected_type {
            assert_eq!(self.content_type, expected_type, "Custom glyph rasterizer must always produce the same content type for a given input. Expected {:?}, got {:?}. Input: {:?}", expected_type, self.content_type, input);
        }

        assert_eq!(
            self.data.len(),
            input.width as usize * input.height as usize * self.content_type.bytes_per_pixel(),
            "Invalid custom glyph rasterizer output. Expected data of length {}, got length {}. Input: {:?}",
            input.width as usize * input.height as usize * self.content_type.bytes_per_pixel(),
            self.data.len(),
            input,
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CustomGlyphCacheKey {
    /// Font ID
    pub glyph_id: CustomGlyphId,
    /// Glyph width
    pub width: u16,
    /// Glyph height
    pub height: u16,
    /// Binning of fractional X offset
    pub x_bin: SubpixelBin,
    /// Binning of fractional Y offset
    pub y_bin: SubpixelBin,
}

/// The type of image data contained in a rasterized glyph
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ContentType {
    /// Each pixel contains 32 bits of rgba data
    Color,
    /// Each pixel contains a single 8 bit channel
    Mask,
}

impl ContentType {
    /// The number of bytes per pixel for this content type
    pub fn bytes_per_pixel(&self) -> usize {
        match self {
            Self::Color => 4,
            Self::Mask => 1,
        }
    }
}
