use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily, FontId};

pub const TERM_REGULAR: &str = "term-regular";
pub const TERM_BOLD: &str = "term-bold";
pub const TERM_ITALIC: &str = "term-italic";
pub const TERM_BOLD_ITALIC: &str = "term-bold-italic";

/// Register the bundled Cascadia Code faces. Each terminal face gets its own
/// family so bold/italic map to real faces (no synthetic styling), with
/// egui's default monospace appended for glyph fallback.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    let faces: [(&str, &[u8]); 4] = [
        (
            TERM_REGULAR,
            include_bytes!("../../../assets/fonts/CascadiaCode-Regular.ttf"),
        ),
        (
            TERM_BOLD,
            include_bytes!("../../../assets/fonts/CascadiaCode-Bold.ttf"),
        ),
        (
            TERM_ITALIC,
            include_bytes!("../../../assets/fonts/CascadiaCode-Italic.ttf"),
        ),
        (
            TERM_BOLD_ITALIC,
            include_bytes!("../../../assets/fonts/CascadiaCode-BoldItalic.ttf"),
        ),
    ];

    let fallback: Vec<String> = fonts
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();

    for (name, bytes) in faces {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
        let mut family = vec![name.to_owned()];
        family.extend(fallback.iter().cloned());
        fonts
            .families
            .insert(FontFamily::Name(name.into()), family);
    }

    // Make Cascadia the default monospace for chrome consistency too.
    if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
        mono.insert(0, TERM_REGULAR.to_owned());
    }

    ctx.set_fonts(fonts);
}

/// Terminal cell geometry derived from the font at the current size.
#[derive(Clone, Debug, PartialEq)]
pub struct CellMetrics {
    pub width: f32,
    pub height: f32,
    /// Vertical offset from cell top to where the glyph row is painted.
    pub glyph_offset: f32,
    pub font_size: f32,
    pub regular: FontId,
    pub bold: FontId,
    pub italic: FontId,
    pub bold_italic: FontId,
}

pub fn measure(
    ctx: &egui::Context,
    font_size: f32,
    line_height: f32,
    letter_spacing: f32,
) -> CellMetrics {
    let regular = FontId::new(font_size, FontFamily::Name(TERM_REGULAR.into()));
    let bold = FontId::new(font_size, FontFamily::Name(TERM_BOLD.into()));
    let italic = FontId::new(font_size, FontFamily::Name(TERM_ITALIC.into()));
    let bold_italic = FontId::new(font_size, FontFamily::Name(TERM_BOLD_ITALIC.into()));

    let (glyph_width, row_height) = ctx.fonts_mut(|f| {
        (
            f.glyph_width(&regular, 'M'),
            f.row_height(&regular),
        )
    });

    let width = glyph_width + letter_spacing;
    let height = (row_height * line_height).ceil();

    CellMetrics {
        width,
        height,
        glyph_offset: ((height - row_height) / 2.0).max(0.0),
        font_size,
        regular,
        bold,
        italic,
        bold_italic,
    }
}
