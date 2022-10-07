use std::fmt::{self, Display, Formatter};

pub type Result<T> = std::result::Result<T, GlyphonError>;

#[derive(Debug)]
pub enum GlyphonError {
    RemovedFromAtlas,
    ScreenResolutionChanged,
    AtlasFull,
}

impl Display for GlyphonError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            GlyphonError::RemovedFromAtlas => {
                write!(f, "Render Error: Glyph no longer Exists within the Atlas.")
            }
            GlyphonError::ScreenResolutionChanged => write!(
                f,
                "Render Error: Screen Resolution Changed Since Prepare was last executed."
            ),
            GlyphonError::AtlasFull => write!(f, "Prepare Error: The Glyph Texture Atlas is full."),
        }
    }
}
