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
        write!(f, "Prepare error: glyph texture atlas is full")
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
                write!(
                    f,
                    "Render error: glyph no longer exists within the texture atlas"
                )
            }
            RenderError::ScreenResolutionChanged => write!(
                f,
                "Render error: screen resolution changed since last `prepare` call"
            ),
        }
    }
}
