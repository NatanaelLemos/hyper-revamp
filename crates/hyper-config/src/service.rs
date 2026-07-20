use std::collections::HashMap;
use std::io::Write;

use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::keymap::{build_keymap, Command, KeyChord, OneOrMany};
use crate::model::{ProfileDef, ResolvedConfig, TypedRoot};
use crate::paths;
use crate::resolve::resolve_profile_config;
use crate::theme::{load_all_themes, ThemeColors};

pub const DEFAULT_CONFIG_JSON: &str = include_str!("../../../assets/config-default.json");

/// Deep-merge `user` over `defaults`: objects merge recursively,
/// arrays/scalars replace (parity with the Electron app's import merge).
fn deep_merge(defaults: &Value, user: &Value) -> Value {
    match (defaults, user) {
        (Value::Object(d), Value::Object(u)) => {
            let mut out = d.clone();
            for (k, v) in u {
                let merged = match out.get(k) {
                    Some(existing) => deep_merge(existing, v),
                    None => v.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        _ => user.clone(),
    }
}

/// One immutable, fully-parsed view of the config file plus themes and
/// keymap. Rebuilt wholesale on every change (cheap; config files are tiny).
pub struct ConfigSnapshot {
    /// Defaults merged with the user's file — what the app reads.
    pub document: Value,
    /// The user's file exactly as parsed, with no defaults folded in. Saves
    /// start from this so unknown keys (plugins, $schema…) survive and the
    /// defaults never get baked into the user's file.
    pub user_document: Value,
    /// The user's file as literal text, or None when it doesn't exist yet.
    /// The raw JSON editor shows these bytes rather than a re-serialization.
    pub user_text: Option<String>,
    pub root: TypedRoot,
    pub themes: IndexMap<String, ThemeColors>,
    pub keymap: Vec<(KeyChord, Command)>,
    /// Set when the file existed but could not be parsed.
    pub parse_error: Option<String>,
    pub generation: u64,
}

impl ConfigSnapshot {
    pub fn load(generation: u64) -> ConfigSnapshot {
        let path = paths::cfg_path();
        let mut parse_error = None;
        let mut user_text = None;
        write_schema_file();

        let user_doc: Value = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let parsed = match serde_json::from_str(&text) {
                    Ok(value) => value,
                    Err(err) => {
                        parse_error = Some(err.to_string());
                        log::warn!("couldn't parse {}: {err}; using defaults", path.display());
                        Value::Object(Map::new())
                    }
                };
                user_text = Some(text);
                parsed
            }
            // Only a *missing* file is a first run. Any other read error
            // (permissions, invalid UTF-8, a busy network mount) leaves the
            // file alone: `migrate.ts` guarded the default write with
            // `existsSync` and never clobbered an existing config.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Err(err) = std::fs::write(&path, DEFAULT_CONFIG_JSON) {
                    log::warn!("couldn't write default config: {err}");
                } else {
                    user_text = Some(DEFAULT_CONFIG_JSON.to_string());
                }
                serde_json::from_str(DEFAULT_CONFIG_JSON).unwrap_or(Value::Object(Map::new()))
            }
            Err(err) => {
                parse_error = Some(format!("couldn't read {}: {err}", path.display()));
                log::warn!(
                    "couldn't read {}: {err}; using defaults for this session and leaving the \
                     file untouched",
                    path.display()
                );
                Value::Object(Map::new())
            }
        };

        let defaults: Value =
            serde_json::from_str(DEFAULT_CONFIG_JSON).expect("embedded default config parses");
        let document = deep_merge(&defaults, &user_doc);

        let mut root: TypedRoot = document
            .get("config")
            .cloned()
            .and_then(|v| match serde_json::from_value(v) {
                Ok(root) => Some(root),
                Err(err) => {
                    log::warn!("config section malformed: {err}");
                    None
                }
            })
            .unwrap_or_default();

        // Parity with init.ts: always at least one profile; unnamed profiles
        // get generated names.
        if root.profiles.is_empty() {
            root.profiles.push(ProfileDef {
                name: "default".into(),
                ..Default::default()
            });
        }
        for (i, profile) in root.profiles.iter_mut().enumerate() {
            if profile.name.trim().is_empty() {
                profile.name = format!("profile-{}", i + 1);
            }
        }

        let themes = load_all_themes(&paths::themes_dir(), root.themes.as_ref());

        let user_keymaps: HashMap<String, OneOrMany> = document
            .get("keymaps")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let keymap = build_keymap(&user_keymaps);

        ConfigSnapshot {
            document,
            user_document: user_doc,
            user_text,
            root,
            themes,
            keymap,
            parse_error,
            generation,
        }
    }

    pub fn default_profile_name(&self) -> String {
        let name = self
            .root
            .default_profile
            .clone()
            .unwrap_or_else(|| "default".into());
        if self.root.profiles.iter().any(|p| p.name == name) {
            name
        } else {
            self.root
                .profiles
                .first()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "default".into())
        }
    }

    pub fn profile(&self, name: &str) -> Option<&ProfileDef> {
        self.root.profiles.iter().find(|p| p.name == name)
    }

    /// Resolve options for a profile (None ⇒ default profile).
    pub fn resolved(&self, profile_name: Option<&str>) -> ResolvedConfig {
        let name = profile_name
            .map(str::to_string)
            .unwrap_or_else(|| self.default_profile_name());
        let profile = self.profile(&name);
        let empty = Map::new();
        let root_map = self
            .document
            .get("config")
            .and_then(|v| v.as_object())
            .unwrap_or(&empty);
        resolve_profile_config(
            root_map,
            profile,
            &self.themes,
            self.root.default_theme.as_deref(),
        )
    }
}

/// The JSON schema the default config's `$schema` key points at. `migrate.ts`
/// copied this next to the config on every startup; without it the reference
/// dangles and editors lose config completion.
const SCHEMA_JSON: &str = include_str!("../../../assets/schema.json");

fn write_schema_file() {
    let path = paths::schema_path();
    // `migrate.ts` re-copied the schema on every startup; skip the write when
    // it's already current so hot reloads stay quiet.
    if std::fs::read_to_string(&path).is_ok_and(|current| current == SCHEMA_JSON) {
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(err) = std::fs::write(&path, SCHEMA_JSON) {
        log::warn!("couldn't write {}: {err}", path.display());
    }
}

/// Reject documents the app can't represent before they reach the file.
/// Port of `normalizeRawConfig` (`app/settings.ts`), which refused a
/// non-object root rather than writing it.
fn validate_root(value: &Value) -> Result<(), String> {
    match value {
        Value::Object(map) => match map.get("config") {
            None | Some(Value::Object(_)) | Some(Value::Null) => Ok(()),
            Some(_) => Err("`config` must be a JSON object.".into()),
        },
        _ => Err("Configuration root must be a JSON object.".into()),
    }
}

/// Validate and atomically save raw JSON text to the config file.
pub fn save_text(text: &str) -> Result<(), String> {
    let parsed: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    validate_root(&parsed)?;
    let mut bytes = text.as_bytes().to_vec();
    // `JSON.stringify(parsed, null, 2) + '\n'` — keep the trailing newline the
    // Electron build always wrote.
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    atomic_write(&paths::cfg_path(), &bytes).map_err(|e| e.to_string())
}

/// Atomically save a document as pretty 2-space JSON.
pub fn save_document(document: &Value) -> Result<(), String> {
    validate_root(document)?;
    let mut text = serde_json::to_string_pretty(document).map_err(|e| e.to_string())?;
    text.push('\n');
    atomic_write(&paths::cfg_path(), text.as_bytes()).map_err(|e| e.to_string())
}

fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    // Write through symlinks: a config symlinked into a dotfiles repo must
    // keep pointing at its target instead of being replaced by a regular file
    // (`writeFileSync` followed the link).
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = target
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    std::fs::create_dir_all(&dir)?;

    let mut tmp = tempfile::NamedTempFile::new_in(&dir)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;

    // Keep the existing file's mode; tempfiles are 0600, which would silently
    // tighten permissions on every save.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&target)
            .map(|meta| meta.permissions().mode() & 0o777)
            .unwrap_or(0o644);
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(mode))?;
    }

    tmp.persist(&target).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deep_merge_keeps_unknown_and_overrides() {
        let defaults = json!({"config": {"scrollback": 1000, "padding": "12px 14px"}, "plugins": []});
        let user = json!({"config": {"scrollback": 5000}, "plugins": ["hyperpower"], "$schema": "x"});
        let merged = deep_merge(&defaults, &user);
        assert_eq!(merged["config"]["scrollback"], 5000);
        assert_eq!(merged["config"]["padding"], "12px 14px");
        assert_eq!(merged["plugins"], json!(["hyperpower"]));
        assert_eq!(merged["$schema"], "x");
    }

    #[test]
    fn default_document_parses_and_resolves() {
        let defaults: Value = serde_json::from_str(DEFAULT_CONFIG_JSON).unwrap();
        let root: TypedRoot =
            serde_json::from_value(defaults.get("config").cloned().unwrap()).unwrap();
        assert_eq!(root.default_profile.as_deref(), Some("default"));
        assert_eq!(root.profiles.len(), 1);
    }
}
