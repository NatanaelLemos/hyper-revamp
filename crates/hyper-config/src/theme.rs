use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};

use crate::lenient;

/// The 16-color ANSI map, exactly the shape of Hyper's `ColorMap`
/// (`typings/config.d.ts`). All fields optional so themes can be partial.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorMap {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub black: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub red: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub green: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yellow: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blue: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magenta: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cyan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub white: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_black: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_red: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_green: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_yellow: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_blue: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_magenta: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_cyan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_white: Option<String>,
}

/// Field order of the array form, from `colorList` in `app/utils/colors.ts`.
/// Entries 16/17 (`colorCubes`, `grayscale`) are accepted and ignored — the
/// palette derives the 6x6x6 cube and grayscale ramp itself.
const COLOR_LIST: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "lightBlack",
    "lightRed",
    "lightGreen",
    "lightYellow",
    "lightBlue",
    "lightMagenta",
    "lightCyan",
    "lightWhite",
];

impl ColorMap {
    /// Set one ANSI slot by its camelCase name.
    fn set(&mut self, key: &str, value: String) {
        let slot = match key {
            "black" => &mut self.black,
            "red" => &mut self.red,
            "green" => &mut self.green,
            "yellow" => &mut self.yellow,
            "blue" => &mut self.blue,
            "magenta" => &mut self.magenta,
            "cyan" => &mut self.cyan,
            "white" => &mut self.white,
            "lightBlack" => &mut self.light_black,
            "lightRed" => &mut self.light_red,
            "lightGreen" => &mut self.light_green,
            "lightYellow" => &mut self.light_yellow,
            "lightBlue" => &mut self.light_blue,
            "lightMagenta" => &mut self.light_magenta,
            "lightCyan" => &mut self.light_cyan,
            "lightWhite" => &mut self.light_white,
            _ => return,
        };
        *slot = Some(value);
    }

    /// ANSI order 0-15: black..white then bright variants.
    pub fn as_array(&self) -> [Option<&String>; 16] {
        [
            self.black.as_ref(),
            self.red.as_ref(),
            self.green.as_ref(),
            self.yellow.as_ref(),
            self.blue.as_ref(),
            self.magenta.as_ref(),
            self.cyan.as_ref(),
            self.white.as_ref(),
            self.light_black.as_ref(),
            self.light_red.as_ref(),
            self.light_green.as_ref(),
            self.light_yellow.as_ref(),
            self.light_blue.as_ref(),
            self.light_magenta.as_ref(),
            self.light_cyan.as_ref(),
            self.light_white.as_ref(),
        ]
    }
}

impl ColorMap {
    /// Accepts both documented shapes of `colors`: the named map, or the full
    /// array ("if you're going to provide the full color palette […] just
    /// provide an array here instead of a color map object" —
    /// `typings/config.d.ts`), which `getColorMap` (`app/utils/colors.ts`)
    /// folded into the same map. Non-color entries are dropped individually.
    fn from_value(value: &serde_json::Value) -> ColorMap {
        use serde_json::Value;
        let mut out = ColorMap::default();
        match value {
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    let Some(name) = COLOR_LIST.get(index) else {
                        break; // colorCubes / grayscale / overlong arrays
                    };
                    match item {
                        Value::String(color) => out.set(name, color.clone()),
                        other if other.is_null() => {}
                        other => {
                            log::warn!("colors[{index}] ({name}) is not a color string: {other}")
                        }
                    }
                }
            }
            Value::Object(map) => {
                for name in COLOR_LIST {
                    match map.get(name) {
                        Some(Value::String(color)) => out.set(name, color.clone()),
                        Some(other) if !other.is_null() => {
                            log::warn!("colors.{name} is not a color string: {other}")
                        }
                        _ => {}
                    }
                }
            }
            Value::Null => {}
            other => log::warn!("`colors` must be an object or array, found {other}; ignoring"),
        }
        out
    }
}

impl<'de> Deserialize<'de> for ColorMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(ColorMap::from_value(&value))
    }
}

/// `fontWeight` accepts CSS keywords or numbers ("normal" | "bold" | 600 | "600").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FontWeight {
    Number(u16),
    Keyword(String),
}

impl FontWeight {
    pub fn is_bold(&self) -> bool {
        match self {
            FontWeight::Number(n) => *n >= 600,
            FontWeight::Keyword(k) => k.eq_ignore_ascii_case("bold"),
        }
    }
}

/// A named theme: visual overrides applied between root config and profile
/// overrides. Mirrors `ThemeColors` in `typings/config.d.ts`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeColors {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreground_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_accent_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colors: Option<ColorMap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_weight: Option<FontWeight>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_weight_bold: Option<FontWeight>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub letter_spacing: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_ligatures: Option<bool>,
    /// Any other keys the theme carries (`padding`, `cursorShape`, …).
    /// `mergeShallow` in `app/config/resolve-profile.ts` applied every key a
    /// theme defined, not just the color ones, so they are kept and merged.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Keys `ThemeColors` decodes into its own fields; everything else lands in
/// `extra` and is merged verbatim.
const THEME_KNOWN_KEYS: [&str; 15] = [
    "backgroundColor",
    "foregroundColor",
    "cursorColor",
    "cursorAccentColor",
    "borderColor",
    "selectionColor",
    "colors",
    "fontFamily",
    "uiFontFamily",
    "fontSize",
    "fontWeight",
    "fontWeightBold",
    "lineHeight",
    "letterSpacing",
    "disableLigatures",
];

impl<'de> Deserialize<'de> for ThemeColors {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let fields = lenient::fields(deserializer, "theme")?;
        let colors = fields.raw("colors").map(ColorMap::from_value);
        let mut theme = ThemeColors {
            background_color: fields.get("backgroundColor"),
            foreground_color: fields.get("foregroundColor"),
            cursor_color: fields.get("cursorColor"),
            cursor_accent_color: fields.get("cursorAccentColor"),
            border_color: fields.get("borderColor"),
            selection_color: fields.get("selectionColor"),
            colors: colors.filter(|c| *c != ColorMap::default()),
            font_family: fields.get("fontFamily"),
            ui_font_family: fields.get("uiFontFamily"),
            font_size: fields.get("fontSize"),
            font_weight: fields.get("fontWeight"),
            font_weight_bold: fields.get("fontWeightBold"),
            line_height: fields.get("lineHeight"),
            letter_spacing: fields.get("letterSpacing"),
            disable_ligatures: fields.get("disableLigatures"),
            extra: serde_json::Map::new(),
        };
        theme.extra = fields
            .into_map()
            .into_iter()
            .filter(|(key, _)| !THEME_KNOWN_KEYS.contains(&key.as_str()))
            .collect();
        Ok(theme)
    }
}

/// The 12 themes shipped with the app, embedded verbatim from the old repo.
pub const BUNDLED_THEMES: &[(&str, &str)] = &[
    ("default", include_str!("../../../assets/themes/default.json")),
    ("tokyo-night", include_str!("../../../assets/themes/tokyo-night.json")),
    ("catppuccin-mocha", include_str!("../../../assets/themes/catppuccin-mocha.json")),
    ("gruvbox-dark", include_str!("../../../assets/themes/gruvbox-dark.json")),
    ("dracula", include_str!("../../../assets/themes/dracula.json")),
    ("nord", include_str!("../../../assets/themes/nord.json")),
    ("synthwave-84", include_str!("../../../assets/themes/synthwave-84.json")),
    ("one-dark", include_str!("../../../assets/themes/one-dark.json")),
    ("rose-pine", include_str!("../../../assets/themes/rose-pine.json")),
    ("monokai", include_str!("../../../assets/themes/monokai.json")),
    ("solarized-dark", include_str!("../../../assets/themes/solarized-dark.json")),
    ("ayu-mirage", include_str!("../../../assets/themes/ayu-mirage.json")),
];

pub fn bundled_themes() -> IndexMap<String, ThemeColors> {
    let mut map = IndexMap::new();
    for (id, json) in BUNDLED_THEMES {
        match serde_json::from_str::<ThemeColors>(json) {
            Ok(theme) => {
                map.insert((*id).to_string(), theme);
            }
            Err(err) => log::warn!("bundled theme {id} failed to parse: {err}"),
        }
    }
    map
}

/// Parse any CSS color ("#000", "rgba(1,2,3,.5)", named) to RGBA bytes.
pub fn parse_color(value: &str) -> Option<[u8; 4]> {
    csscolorparser::parse(value).ok().map(|c| c.to_rgba8())
}

/// Parse a CSS theme file's `--hyper-*` custom properties into ThemeColors.
/// Port of `parseCssTheme` in `app/config/themes/index.ts`.
pub fn parse_css_theme(css: &str) -> ThemeColors {
    let re = regex::Regex::new(r"(?i)--hyper-([a-z-]+)\s*:\s*([^;}\n]+?)\s*(?:;|\n|$)")
        .expect("static regex");
    let mut vars: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for caps in re.captures_iter(css) {
        vars.insert(caps[1].to_lowercase(), caps[2].trim().to_string());
    }

    // `if (vars['background']) …` in `parseCssTheme` — an empty value is
    // falsy in JS and leaves the key unset, so the config value is inherited
    // rather than overridden with "" (which would render as black).
    let get = |k: &str| {
        vars.get(k)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let mut theme = ThemeColors {
        background_color: get("background"),
        foreground_color: get("foreground"),
        cursor_color: get("cursor"),
        cursor_accent_color: get("cursor-accent"),
        border_color: get("border"),
        selection_color: get("selection"),
        ..Default::default()
    };

    let colors = ColorMap {
        black: get("black"),
        red: get("red"),
        green: get("green"),
        yellow: get("yellow"),
        blue: get("blue"),
        magenta: get("magenta"),
        cyan: get("cyan"),
        white: get("white"),
        light_black: get("light-black"),
        light_red: get("light-red"),
        light_green: get("light-green"),
        light_yellow: get("light-yellow"),
        light_blue: get("light-blue"),
        light_magenta: get("light-magenta"),
        light_cyan: get("light-cyan"),
        light_white: get("light-white"),
    };
    if colors != ColorMap::default() {
        theme.colors = Some(colors);
    }
    theme
}

/// User theme overrides from `<cfgDir>/themes/*.{json,css}` (id = file
/// stem, user wins on collision). Malformed files are skipped with a warn.
pub fn load_user_themes(dir: &std::path::Path) -> IndexMap<String, ThemeColors> {
    let mut out = IndexMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let (Some(stem), Some(ext)) = (
            path.file_stem().and_then(|s| s.to_str()),
            path.extension().and_then(|s| s.to_str()),
        ) else {
            continue;
        };
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        match ext.to_lowercase().as_str() {
            "json" => match serde_json::from_str::<ThemeColors>(&raw) {
                Ok(theme) => {
                    out.insert(stem.to_string(), theme);
                }
                Err(err) => log::warn!("failed to load theme {}: {err}", path.display()),
            },
            "css" => {
                out.insert(stem.to_string(), parse_css_theme(&raw));
            }
            _ => {}
        }
    }
    out
}

/// Full theme registry: bundled ← user files ← `config.themes` overrides.
pub fn load_all_themes(
    themes_dir: &std::path::Path,
    config_themes: Option<&IndexMap<String, ThemeColors>>,
) -> IndexMap<String, ThemeColors> {
    let mut all = bundled_themes();
    for (id, theme) in load_user_themes(themes_dir) {
        all.insert(id, theme);
    }
    if let Some(config_themes) = config_themes {
        for (id, theme) in config_themes {
            all.insert(id.clone(), theme.clone());
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_bundled_themes_parse() {
        let themes = bundled_themes();
        assert_eq!(themes.len(), 12);
        let default = &themes["default"];
        assert_eq!(default.background_color.as_deref(), Some("#000"));
        assert_eq!(
            default.colors.as_ref().unwrap().light_white.as_deref(),
            Some("#FFFFFF")
        );
    }

    #[test]
    fn parses_css_colors() {
        assert_eq!(parse_color("#000"), Some([0, 0, 0, 255]));
        assert_eq!(parse_color("rgba(248,28,229,0.8)"), Some([248, 28, 229, 204]));
    }

    #[test]
    fn parses_css_theme_custom_properties() {
        let css = r#"
:root {
  --hyper-background: #1a1b26;
  --hyper-foreground: #c0caf5;
  --hyper-cursor: #c0caf5;
  --hyper-cursor-accent: #1a1b26;
  --hyper-light-white: #ffffff;
  --hyper-red: #f7768e;
}
"#;
        let theme = parse_css_theme(css);
        assert_eq!(theme.background_color.as_deref(), Some("#1a1b26"));
        assert_eq!(theme.cursor_accent_color.as_deref(), Some("#1a1b26"));
        let colors = theme.colors.unwrap();
        assert_eq!(colors.red.as_deref(), Some("#f7768e"));
        assert_eq!(colors.light_white.as_deref(), Some("#ffffff"));
        assert_eq!(colors.green, None);
    }
}
