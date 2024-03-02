use std::{
    error::Error,
    fmt::{self, Display, Formatter},
};

/// An error that occurred while preparing text for rendering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrepareError {
    AtlasFull,
}

impl Display for PrepareError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "Prepare error: glyph texture atlas is full")
    }
}

impl Error for PrepareError {}

/// An error that occurred while rendering text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderError {
    RemovedFromAtlas,
    ScreenResolutionChanged,
    RangeOutOfBounds {
        start: usize,
        end: usize,
        num_text_areas: usize,
    },
}

impl Display for RenderError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            RenderError::RemovedFromAtlas => {
                write!(
                    f,
                    "Render error: glyph no longer exists within the texture atlas"
                )
            }
            RenderError::ScreenResolutionChanged => write!(
                f,
                "Render error: screen resolution changed since last `prepare` call"
            ),
            RenderError::RangeOutOfBounds { start, end, num_text_areas } => write!(
                f,
                "Render error: the range [{}..{}] is out of bounds in renderer with {} prepared text areas",
                start,
                end,
                num_text_areas,
            )
        }
    }
}

impl Error for RenderError {}
