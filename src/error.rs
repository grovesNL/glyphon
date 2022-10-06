use thiserror::Error;

pub type Result<T> = std::result::Result<T, GlyphonError>;

#[derive(Debug, Error)]
pub enum GlyphonError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Surface(#[from] wgpu::SurfaceError),
    #[error(transparent)]
    Wgpu(#[from] wgpu::Error),
    #[error(transparent)]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("Render Error: Glyph no longer Exists within the Atlas.")]
    RemovedFromAtlas,
    #[error("Render Error: Screen Resolution Changed Since Prepare was last executed.")]
    ScreenResolutionChanged,
    #[error("Prepare Error: The Glyph Texture Atlas is full.")]
    AtlasFull,
}
