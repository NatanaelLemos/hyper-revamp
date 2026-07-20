use egui::Color32;
use hyper_config::{parse_color, ThemeColors};

/// Resolved visual theme: full 269-slot color table (matching alacritty's
/// NamedColor indices) plus chrome colors and typography.
#[derive(Clone)]
pub struct TermTheme {
    pub palette: [Color32; 269],
    pub background: Color32,
    pub foreground: Color32,
    pub cursor: Color32,
    pub cursor_accent: Color32,
    pub selection: Color32,
    pub border: Color32,
    pub font_size: f32,
    pub line_height: f32,
    pub letter_spacing: f32,
}

fn c32(value: &str, fallback: Color32) -> Color32 {
    parse_color(value)
        .map(|[r, g, b, a]| Color32::from_rgba_unmultiplied(r, g, b, a))
        .unwrap_or(fallback)
}

fn dim(color: Color32) -> Color32 {
    Color32::from_rgb(
        (color.r() as f32 * 0.66) as u8,
        (color.g() as f32 * 0.66) as u8,
        (color.b() as f32 * 0.66) as u8,
    )
}

impl TermTheme {
    pub fn from_theme(t: &ThemeColors) -> Self {
        let background = t
            .background_color
            .as_deref()
            .map(|s| c32(s, Color32::BLACK))
            .unwrap_or(Color32::BLACK);
        let foreground = t
            .foreground_color
            .as_deref()
            .map(|s| c32(s, Color32::WHITE))
            .unwrap_or(Color32::WHITE);
        let cursor = t
            .cursor_color
            .as_deref()
            .map(|s| c32(s, foreground))
            .unwrap_or(foreground);
        let cursor_accent = t
            .cursor_accent_color
            .as_deref()
            .map(|s| c32(s, background))
            .unwrap_or(background);
        let selection = t
            .selection_color
            .as_deref()
            .map(|s| c32(s, Color32::from_rgba_unmultiplied(248, 28, 229, 77)))
            .unwrap_or(Color32::from_rgba_unmultiplied(248, 28, 229, 77));
        let border = t
            .border_color
            .as_deref()
            .map(|s| c32(s, Color32::from_gray(51)))
            .unwrap_or(Color32::from_gray(51));

        // Default 16-color table (Hyper's default theme values) overridden by
        // whatever the theme provides.
        let default_ansi: [Color32; 16] = [
            Color32::from_rgb(0x00, 0x00, 0x00),
            Color32::from_rgb(0xC5, 0x1E, 0x14),
            Color32::from_rgb(0x1D, 0xC1, 0x21),
            Color32::from_rgb(0xC7, 0xC3, 0x29),
            Color32::from_rgb(0x0A, 0x2F, 0xC4),
            Color32::from_rgb(0xC8, 0x39, 0xC5),
            Color32::from_rgb(0x20, 0xC5, 0xC6),
            Color32::from_rgb(0xC7, 0xC7, 0xC7),
            Color32::from_rgb(0x68, 0x68, 0x68),
            Color32::from_rgb(0xFD, 0x6F, 0x6B),
            Color32::from_rgb(0x67, 0xF8, 0x6F),
            Color32::from_rgb(0xFF, 0xFA, 0x72),
            Color32::from_rgb(0x6A, 0x76, 0xFB),
            Color32::from_rgb(0xFD, 0x7C, 0xFC),
            Color32::from_rgb(0x68, 0xFD, 0xFE),
            Color32::from_rgb(0xFF, 0xFF, 0xFF),
        ];
        let mut ansi = default_ansi;
        if let Some(map) = &t.colors {
            for (i, value) in map.as_array().iter().enumerate() {
                if let Some(value) = value {
                    ansi[i] = c32(value, ansi[i]);
                }
            }
        }

        let mut palette = [Color32::BLACK; 269];
        palette[..16].copy_from_slice(&ansi);
        // 6x6x6 color cube (16..232).
        for i in 0..216 {
            let (r, g, b) = (i / 36, (i / 6) % 6, i % 6);
            let comp = |c: usize| if c == 0 { 0 } else { (c * 40 + 55) as u8 };
            palette[16 + i] = Color32::from_rgb(comp(r), comp(g), comp(b));
        }
        // Grayscale ramp (232..256).
        for i in 0..24 {
            let v = (8 + i * 10) as u8;
            palette[232 + i] = Color32::from_rgb(v, v, v);
        }
        palette[256] = foreground;
        palette[257] = background;
        palette[258] = cursor;
        for i in 0..8 {
            palette[259 + i] = dim(ansi[i]);
        }
        palette[267] = foreground; // BrightForeground
        palette[268] = dim(foreground); // DimForeground

        TermTheme {
            palette,
            background,
            foreground,
            cursor,
            cursor_accent,
            selection,
            border,
            font_size: t.font_size.unwrap_or(14.0),
            line_height: t.line_height.unwrap_or(1.0).max(0.5),
            letter_spacing: t.letter_spacing.unwrap_or(0.0),
        }
    }

    /// Resolve a cell color against runtime OSC overrides then this palette.
    pub fn resolve(
        &self,
        color: alacritty_terminal::vte::ansi::Color,
        overrides: &alacritty_terminal::term::color::Colors,
    ) -> Color32 {
        use alacritty_terminal::vte::ansi::Color as AColor;
        match color {
            AColor::Spec(rgb) => Color32::from_rgb(rgb.r, rgb.g, rgb.b),
            AColor::Named(named) => {
                let idx = named as usize;
                match overrides[named] {
                    Some(rgb) => Color32::from_rgb(rgb.r, rgb.g, rgb.b),
                    None => self.palette[idx.min(268)],
                }
            }
            AColor::Indexed(idx) => match overrides[idx as usize] {
                Some(rgb) => Color32::from_rgb(rgb.r, rgb.g, rgb.b),
                None => self.palette[idx as usize],
            },
        }
    }

    /// Foreground for a cell, applying xterm.js's `drawBoldTextInBrightColors`
    /// (on by default, and the original never turned it off): bold text in one
    /// of the 8 basic ANSI colors renders in that color's bright variant.
    ///
    /// Only those 8 qualify — xterm.js brightens the palette index when
    /// `fg < 8`, so 256-color/RGB colors and the *default* foreground (a
    /// different color mode, not index 0-7) are left alone.
    pub fn resolve_fg(
        &self,
        color: alacritty_terminal::vte::ansi::Color,
        overrides: &alacritty_terminal::term::color::Colors,
        bold: bool,
    ) -> Color32 {
        use alacritty_terminal::vte::ansi::Color as AColor;
        if bold {
            if let Some(bright) = basic_ansi(color).map(|named| named.to_bright()) {
                return self.resolve(AColor::Named(bright), overrides);
            }
        }
        self.resolve(color, overrides)
    }
}

/// The basic ANSI color (index 0-7) a color refers to, if any.
fn basic_ansi(
    color: alacritty_terminal::vte::ansi::Color,
) -> Option<alacritty_terminal::vte::ansi::NamedColor> {
    use alacritty_terminal::vte::ansi::{Color as AColor, NamedColor::*};
    let named = match color {
        AColor::Named(named @ (Black | Red | Green | Yellow | Blue | Magenta | Cyan | White)) => {
            named
        }
        AColor::Indexed(index @ 0..=7) => match index {
            0 => Black,
            1 => Red,
            2 => Green,
            3 => Yellow,
            4 => Blue,
            5 => Magenta,
            6 => Cyan,
            _ => White,
        },
        _ => return None,
    };
    Some(named)
}

impl TermTheme {
    /// Build from fully resolved profile options (root ← theme ← profile).
    pub fn from_resolved(cfg: &hyper_config::ResolvedConfig) -> Self {
        let as_theme = hyper_config::ThemeColors {
            background_color: Some(cfg.background_color.clone()),
            foreground_color: Some(cfg.foreground_color.clone()),
            cursor_color: Some(cfg.cursor_color.clone()),
            cursor_accent_color: Some(cfg.cursor_accent_color.clone()),
            border_color: Some(cfg.border_color.clone()),
            selection_color: Some(cfg.selection_color.clone()),
            colors: Some(cfg.colors.clone()),
            font_size: Some(cfg.font_size),
            line_height: Some(cfg.line_height),
            letter_spacing: Some(cfg.letter_spacing),
            ..Default::default()
        };
        TermTheme::from_theme(&as_theme)
    }
}

impl Default for TermTheme {
    fn default() -> Self {
        let themes = hyper_config::bundled_themes();
        TermTheme::from_theme(&themes["default"])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::term::color::Colors;
    use alacritty_terminal::vte::ansi::{Color as AColor, NamedColor};

    /// `printf '\e[1;31mERROR\e[0m'` must render in bright red, as it did
    /// under xterm.js's drawBoldTextInBrightColors.
    #[test]
    fn bold_basic_ansi_renders_bright() {
        let theme = TermTheme::default();
        let overrides = Colors::default();

        let plain = theme.resolve_fg(AColor::Named(NamedColor::Red), &overrides, false);
        let bold = theme.resolve_fg(AColor::Named(NamedColor::Red), &overrides, true);
        assert_eq!(plain, theme.palette[NamedColor::Red as usize]);
        assert_eq!(bold, theme.palette[NamedColor::BrightRed as usize]);
        assert_ne!(plain, bold);

        // The indexed spelling of the same color behaves identically.
        assert_eq!(theme.resolve_fg(AColor::Indexed(1), &overrides, true), bold);
    }

    /// Only indices 0-7 brighten: 256-color, RGB, and the default foreground
    /// are untouched (xterm.js brightens only when `fg < 8`).
    #[test]
    fn bold_leaves_other_colors_alone() {
        let theme = TermTheme::default();
        let overrides = Colors::default();

        for color in [
            AColor::Named(NamedColor::Foreground),
            AColor::Named(NamedColor::BrightRed),
            AColor::Indexed(9),
            AColor::Indexed(200),
            AColor::Spec(alacritty_terminal::vte::ansi::Rgb { r: 1, g: 2, b: 3 }),
        ] {
            assert_eq!(
                theme.resolve_fg(color, &overrides, true),
                theme.resolve_fg(color, &overrides, false),
                "{color:?} must not brighten when bold"
            );
        }
    }
}
