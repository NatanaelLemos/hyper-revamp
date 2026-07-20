use std::collections::HashMap;

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::lenient;
use crate::theme::{ColorMap, FontWeight, ThemeColors};

/// `opacity`: single value or per-focus values.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WindowOpacity {
    Single(f32),
    FocusBlur {
        #[serde(skip_serializing_if = "Option::is_none")]
        focus: Option<f32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        blur: Option<f32>,
    },
}

impl WindowOpacity {
    pub fn focused(&self) -> f32 {
        match self {
            WindowOpacity::Single(v) => *v,
            WindowOpacity::FocusBlur { focus, .. } => focus.unwrap_or(1.0),
        }
    }
    pub fn blurred(&self) -> f32 {
        match self {
            WindowOpacity::Single(v) => *v,
            WindowOpacity::FocusBlur { blur, focus } => blur.or(*focus).unwrap_or(1.0),
        }
    }
}

impl Default for WindowOpacity {
    fn default() -> Self {
        WindowOpacity::Single(1.0)
    }
}

/// `bell`: `"SOUND"` or `false`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Bell {
    Mode(String),
    Off(bool),
}

impl Bell {
    pub fn is_sound(&self) -> bool {
        matches!(self, Bell::Mode(m) if m.eq_ignore_ascii_case("sound"))
    }
}

impl Default for Bell {
    fn default() -> Self {
        Bell::Mode("SOUND".into())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModifierKeys {
    pub alt_is_meta: bool,
    pub cmd_is_meta: bool,
}

/// SSH settings of a profile (`SshProfile` in `typings/config.d.ts`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshProfileDef {
    pub host: String,
    pub user: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forward_agent: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

impl SshProfileDef {
    /// The original treated a stored password as password auth even without
    /// an explicit `authType` (`const usePassword = ssh.authType === 'password'
    /// || !!ssh.password` — `app/ui/window.ts`).
    pub fn uses_password_auth(&self) -> bool {
        self.auth_type.as_deref() == Some("password")
            || self.password.as_deref().is_some_and(|p| !p.is_empty())
    }
}

impl<'de> Deserialize<'de> for SshProfileDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let fields = lenient::fields(deserializer, "profile ssh")?;
        Ok(SshProfileDef {
            host: fields.get_or("host", String::new()),
            user: fields.get_or("user", String::new()),
            // `ssh-args.ts` stringified whatever `port` held, so "2222" worked.
            port: fields.get_loose_number("port"),
            identity_file: fields.get("identityFile"),
            forward_agent: fields.get("forwardAgent"),
            extra_args: fields.get("extraArgs"),
            auth_type: fields.get("authType"),
            password: fields.get("password"),
        })
    }
}

/// One entry of `config.profiles[]`. `config` stays a raw JSON map so the
/// shallow-merge semantics of `resolve-profile.ts` are preserved exactly.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDef {
    pub name: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub profile_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh: Option<SshProfileDef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub config: serde_json::Map<String, Value>,
}

impl ProfileDef {
    pub fn is_ssh(&self) -> bool {
        self.profile_type.as_deref() == Some("ssh")
    }
}

impl<'de> Deserialize<'de> for ProfileDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let fields = lenient::fields(deserializer, "profile")?;
        Ok(ProfileDef {
            name: fields.get_or("name", String::new()),
            profile_type: fields.get("type"),
            ssh: fields.get("ssh"),
            theme: fields.get("theme"),
            color: fields.get("color"),
            config: fields.get_or("config", serde_json::Map::new()),
        })
    }
}

/// Typed view of the root-level `config` object (only the fields the app
/// consumes structurally; everything else is reached via the resolved map).
#[derive(Debug, Clone, Default)]
pub struct TypedRoot {
    pub default_profile: Option<String>,
    pub default_theme: Option<String>,
    pub restore_session: Option<bool>,
    pub quit_on_last_window_closed: Option<bool>,
    pub default_ssh_app: Option<bool>,
    pub profiles: Vec<ProfileDef>,
    pub themes: Option<IndexMap<String, ThemeColors>>,
}

impl<'de> Deserialize<'de> for TypedRoot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let fields = lenient::fields(deserializer, "config")?;
        // Decode profiles one at a time: `init.ts` used the profiles array
        // as-is, so one malformed entry must not take the others with it.
        let profiles = match fields.raw("profiles") {
            Some(Value::Array(items)) => items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| match ProfileDef::deserialize(item.clone()) {
                    Ok(profile) => Some(profile),
                    Err(err) => {
                        log::warn!("config: ignoring malformed profiles[{index}]: {err}");
                        None
                    }
                })
                .collect(),
            Some(other) if !other.is_null() => {
                log::warn!("config: `profiles` must be an array, found {other}; ignoring");
                Vec::new()
            }
            _ => Vec::new(),
        };
        // Same per-entry tolerance for themes.
        let themes = match fields.raw("themes") {
            Some(Value::Object(map)) => Some(
                map.iter()
                    .filter_map(|(id, value)| match ThemeColors::deserialize(value.clone()) {
                        Ok(theme) => Some((id.clone(), theme)),
                        Err(err) => {
                            log::warn!("config: ignoring malformed theme `{id}`: {err}");
                            None
                        }
                    })
                    .collect(),
            ),
            Some(other) if !other.is_null() => {
                log::warn!("config: `themes` must be an object, found {other}; ignoring");
                None
            }
            _ => None,
        };
        Ok(TypedRoot {
            default_profile: fields.get("defaultProfile"),
            // `conf.defaultTheme = conf.defaultTheme || 'default'` (init.ts):
            // an empty string is falsy there and still selects the default.
            default_theme: fields
                .get::<String>("defaultTheme")
                .filter(|name| !name.is_empty())
                .or_else(|| Some("default".to_string())),
            restore_session: fields.get("restoreSession"),
            quit_on_last_window_closed: fields.get("quitOnLastWindowClosed"),
            default_ssh_app: fields.get("defaultSSHApp"),
            profiles,
            themes,
        })
    }
}

/// Fully resolved per-profile options: root ← theme ← profile overrides,
/// deserialized with defaults for anything unset. This is what the terminal
/// engine and views consume.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConfig {
    pub background_color: String,
    pub foreground_color: String,
    pub cursor_color: String,
    pub cursor_accent_color: String,
    pub border_color: String,
    pub selection_color: String,
    pub colors: ColorMap,
    pub font_family: String,
    pub ui_font_family: Option<String>,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_weight_bold: FontWeight,
    pub line_height: f32,
    pub letter_spacing: f32,
    pub disable_ligatures: bool,
    pub cursor_shape: String,
    pub cursor_blink: bool,
    pub padding: String,
    pub bell: Bell,
    pub bell_sound_url: Option<String>,
    pub copy_on_select: bool,
    pub quick_edit: bool,
    pub scrollback: usize,
    pub shell: String,
    pub shell_args: Vec<String>,
    pub env: HashMap<String, String>,
    pub working_directory: String,
    pub preserve_cwd: bool,
    pub mac_option_selection_mode: String,
    pub modifier_keys: ModifierKeys,
    pub opacity: WindowOpacity,
    pub web_links_activation_key: String,
    pub image_support: bool,
    pub screen_reader_mode: bool,
    pub window_size: Option<(f32, f32)>,
}

impl<'de> Deserialize<'de> for ResolvedConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let f = lenient::fields(deserializer, "config")?;
        let d = ResolvedConfig::default();
        // Keys are spelled out rather than derived: several of Hyper's config
        // keys (`preserveCWD`, `bellSoundURL`, `defaultSSHApp`) do not follow
        // serde's camelCase rule, and a silent mismatch means the option is
        // never read at all.
        Ok(ResolvedConfig {
            background_color: f.get_or("backgroundColor", d.background_color),
            foreground_color: f.get_or("foregroundColor", d.foreground_color),
            cursor_color: f.get_or("cursorColor", d.cursor_color),
            cursor_accent_color: f.get_or("cursorAccentColor", d.cursor_accent_color),
            border_color: f.get_or("borderColor", d.border_color),
            selection_color: f.get_or("selectionColor", d.selection_color),
            colors: f.get_or("colors", d.colors),
            font_family: f.get_or("fontFamily", d.font_family),
            ui_font_family: f.get("uiFontFamily").or(d.ui_font_family),
            font_size: f.get_or("fontSize", d.font_size),
            font_weight: f.get_or("fontWeight", d.font_weight),
            font_weight_bold: f.get_or("fontWeightBold", d.font_weight_bold),
            line_height: f.get_or("lineHeight", d.line_height),
            letter_spacing: f.get_or("letterSpacing", d.letter_spacing),
            disable_ligatures: f.get_or("disableLigatures", d.disable_ligatures),
            cursor_shape: f.get_or("cursorShape", d.cursor_shape),
            cursor_blink: f.get_or("cursorBlink", d.cursor_blink),
            padding: f.get_or("padding", d.padding),
            bell: f.get_or("bell", d.bell),
            bell_sound_url: f.get("bellSoundURL").or(d.bell_sound_url),
            copy_on_select: f.get_or("copyOnSelect", d.copy_on_select),
            quick_edit: f.get_or("quickEdit", d.quick_edit),
            scrollback: f.get_or("scrollback", d.scrollback),
            shell: f.get_or("shell", d.shell),
            shell_args: f.get_or("shellArgs", d.shell_args),
            env: f.get_or("env", d.env),
            working_directory: f.get_or("workingDirectory", d.working_directory),
            preserve_cwd: f.get_or("preserveCWD", d.preserve_cwd),
            mac_option_selection_mode: f
                .get_or("macOptionSelectionMode", d.mac_option_selection_mode),
            modifier_keys: f.get_or("modifierKeys", d.modifier_keys),
            opacity: f.get_or("opacity", d.opacity),
            web_links_activation_key: f
                .get_or("webLinksActivationKey", d.web_links_activation_key),
            image_support: f.get_or("imageSupport", d.image_support),
            screen_reader_mode: f.get_or("screenReaderMode", d.screen_reader_mode),
            window_size: f.get("windowSize").or(d.window_size),
        })
    }
}

impl Default for ResolvedConfig {
    fn default() -> Self {
        ResolvedConfig {
            background_color: "#000".into(),
            foreground_color: "#fff".into(),
            cursor_color: "rgba(248,28,229,0.8)".into(),
            cursor_accent_color: "#000".into(),
            border_color: "#333".into(),
            selection_color: "rgba(248,28,229,0.3)".into(),
            colors: ColorMap::default(),
            // Full fallback stacks from `lib/reducers/ui.ts`.
            font_family:
                "\"Cascadia Code\", Menlo, \"DejaVu Sans Mono\", \"Lucida Console\", monospace"
                    .into(),
            ui_font_family: Some(
                "-apple-system, BlinkMacSystemFont, \"Segoe UI\", \"Roboto\", \"Oxygen\", \
                 \"Ubuntu\", \"Cantarell\", \"Fira Sans\", \"Droid Sans\", \"Helvetica Neue\", \
                 sans-serif"
                    .into(),
            ),
            font_size: 14.0,
            font_weight: FontWeight::Keyword("normal".into()),
            font_weight_bold: FontWeight::Keyword("bold".into()),
            line_height: 1.0,
            letter_spacing: 0.0,
            disable_ligatures: false,
            cursor_shape: "BLOCK".into(),
            cursor_blink: false,
            padding: "12px 14px".into(),
            bell: Bell::default(),
            bell_sound_url: None,
            copy_on_select: false,
            quick_edit: false,
            scrollback: 1000,
            shell: String::new(),
            shell_args: vec!["--login".into()],
            env: HashMap::new(),
            working_directory: String::new(),
            preserve_cwd: true,
            mac_option_selection_mode: "vertical".into(),
            modifier_keys: ModifierKeys::default(),
            opacity: WindowOpacity::Single(0.95),
            web_links_activation_key: String::new(),
            image_support: true,
            screen_reader_mode: false,
            window_size: None,
        }
    }
}

impl ResolvedConfig {
    /// Parse the CSS-shorthand `padding` string into (top, right, bottom,
    /// left) pixels. Accepts 1–4 space-separated values with optional `px`.
    pub fn padding_px(&self) -> (f32, f32, f32, f32) {
        parse_padding(&self.padding).unwrap_or((12.0, 14.0, 12.0, 14.0))
    }

    /// Home-expanded workingDirectory, if set and absolute after expansion.
    pub fn working_directory_path(&self) -> Option<std::path::PathBuf> {
        if self.working_directory.is_empty() {
            return None;
        }
        let expanded = if self.working_directory == "~" {
            dirs::home_dir()?
        } else if let Some(rest) = self.working_directory.strip_prefix("~/") {
            dirs::home_dir()?.join(rest)
        } else {
            std::path::PathBuf::from(&self.working_directory)
        };
        if expanded.is_absolute() && expanded.exists() {
            Some(expanded)
        } else {
            None
        }
    }
}

pub fn parse_padding(padding: &str) -> Option<(f32, f32, f32, f32)> {
    let values: Vec<f32> = padding
        .split_whitespace()
        .map(|v| v.trim_end_matches("px").parse::<f32>())
        .collect::<Result<_, _>>()
        .ok()?;
    match values.as_slice() {
        [all] => Some((*all, *all, *all, *all)),
        [v, h] => Some((*v, *h, *v, *h)),
        [t, h, b] => Some((*t, *h, *b, *h)),
        [t, r, b, l] => Some((*t, *r, *b, *l)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_shorthand_forms() {
        assert_eq!(parse_padding("12px 14px"), Some((12.0, 14.0, 12.0, 14.0)));
        assert_eq!(parse_padding("5"), Some((5.0, 5.0, 5.0, 5.0)));
        assert_eq!(
            parse_padding("1px 2px 3px 4px"),
            Some((1.0, 2.0, 3.0, 4.0))
        );
        assert_eq!(parse_padding("1 2 3"), Some((1.0, 2.0, 3.0, 2.0)));
        assert_eq!(parse_padding("garbage"), None);
    }

    #[test]
    fn opacity_variants_deserialize() {
        let single: WindowOpacity = serde_json::from_str("0.9").unwrap();
        assert_eq!(single.focused(), 0.9);
        let split: WindowOpacity =
            serde_json::from_str(r#"{"focus": 1.0, "blur": 0.7}"#).unwrap();
        assert_eq!(split.blurred(), 0.7);
        assert_eq!(split.focused(), 1.0);
    }

    #[test]
    fn bell_variants_deserialize() {
        let sound: Bell = serde_json::from_str(r#""SOUND""#).unwrap();
        assert!(sound.is_sound());
        let off: Bell = serde_json::from_str("false").unwrap();
        assert!(!off.is_sound());
    }
}
