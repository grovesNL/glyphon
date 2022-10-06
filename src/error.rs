use std::{
    error::Error,
    fmt::{self, Display, Formatter},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrepareError {
    AtlasFull,
}

impl Display for PrepareError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "Prepare Error: The Glyph texture atlas is full.")
    }
}

impl Error for PrepareError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderError {
    RemovedFromAtlas,
    ScreenResolutionChanged,
}

impl Display for RenderError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            RenderError::RemovedFromAtlas => {
                write!(f, "Render Error: Glyph no longer Exists within the Atlas.")
            }
            RenderError::ScreenResolutionChanged => write!(
                f,
                "Render Error: Screen resolution changed since prepare was last executed."
            ),
        }
    }
}
