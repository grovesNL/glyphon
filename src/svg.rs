use resvg::{tiny_skia::Pixmap, usvg::Transform};
use rustc_hash::FxHashMap;

// Re-export resvg for convenience.
pub use resvg::*;

use crate::{
    text_render::{ContentType, CustomGlyphInput, CustomGlyphOutput},
    CustomGlyphID,
};

#[derive(Default, Clone)]
pub struct SvgGlyphSystem {
    svgs: FxHashMap<CustomGlyphID, SvgData>,
}

impl SvgGlyphSystem {
    /// Add an svg source to this system.
    ///
    /// * id - A unique identifier for this resource.
    /// * source - The parsed SVG data.
    /// * is_symbolic - If `true`, then only the alpha channel will be used and the icon can
    /// be filled with any solid color. If `false`, then the icon will be rendered in full
    /// color.
    pub fn add_svg(&mut self, id: CustomGlyphID, source: usvg::Tree, content_type: ContentType) {
        self.svgs.insert(
            id,
            SvgData {
                tree: source,
                content_type,
            },
        );
    }

    // Returns `true` if the source was removed, or `false` if there was
    // no source with that ID.
    pub fn remove(&mut self, id: CustomGlyphID) -> bool {
        self.svgs.remove(&id).is_some()
    }

    pub fn rasterize_custom_glyph(&mut self, input: CustomGlyphInput) -> Option<CustomGlyphOutput> {
        let Some(svg_data) = self.svgs.get(&input.id) else {
            return None;
        };

        let svg_size = svg_data.tree.size();
        let max_side_len = svg_size.width().max(svg_size.height());

        let should_rasterize = max_side_len > 0.0;

        let (scale, width, height, pixmap) = if should_rasterize {
            let scale = input.size / max_side_len;
            let width = (svg_size.width() * scale).ceil();
            let height = (svg_size.height() * scale).ceil();

            if width <= 0.0 || height <= 0.0 {
                (0.0, 0, 0, None)
            } else if let Some(pixmap) = Pixmap::new(width as u32, height as u32) {
                (scale, width as u32, height as u32, Some(pixmap))
            } else {
                (0.0, 0, 0, None)
            }
        } else {
            (0.0, 0, 0, None)
        };

        if let Some(mut pixmap) = pixmap {
            let mut transform = Transform::from_scale(scale, scale);

            let offset_x = input.x_bin.as_float();
            let offset_y = input.y_bin.as_float();

            if offset_x != 0.0 || offset_y != 0.0 {
                transform = transform.post_translate(offset_x, offset_y);
            }

            resvg::render(&svg_data.tree, transform, &mut pixmap.as_mut());

            let data: Vec<u8> = if let ContentType::Mask = svg_data.content_type {
                // Only use the alpha channel for symbolic icons.
                pixmap.data().iter().skip(3).step_by(4).copied().collect()
            } else {
                pixmap.data().to_vec()
            };

            Some(CustomGlyphOutput {
                data,
                width,
                height,
                content_type: svg_data.content_type,
            })
        } else {
            None
        }
    }
}

#[derive(Clone)]
struct SvgData {
    tree: usvg::Tree,
    content_type: ContentType,
}
