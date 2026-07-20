//! Exact port of `app/config/resolve-profile.ts`: flatten root config +
//! optional named theme + profile-local overrides. Plain-object values are
//! shallow-merged one level (so a profile's `colors.red` doesn't wipe the
//! theme's `colors.green`); primitives/arrays replace.

use serde_json::{Map, Value};

use crate::model::{ProfileDef, ResolvedConfig};
use crate::theme::ThemeColors;

fn merge_shallow(acc: &mut Map<String, Value>, patch: &Map<String, Value>) {
    for (key, next) in patch {
        if next.is_null() {
            // JS skips `undefined`; JSON null is a real value (e.g.
            // bellSound: null) — keep parity with spreading which copies it.
            acc.insert(key.clone(), next.clone());
            continue;
        }
        match (acc.get(key), next) {
            (Some(Value::Object(base)), Value::Object(patch_obj)) => {
                let mut merged = base.clone();
                for (k, v) in patch_obj {
                    merged.insert(k.clone(), v.clone());
                }
                acc.insert(key.clone(), Value::Object(merged));
            }
            _ => {
                acc.insert(key.clone(), next.clone());
            }
        }
    }
}

/// Merge in JSON space and deserialize into typed options with defaults.
pub fn resolve_profile_config(
    root_config: &Map<String, Value>,
    profile: Option<&ProfileDef>,
    themes: &indexmap::IndexMap<String, ThemeColors>,
    default_theme: Option<&str>,
) -> ResolvedConfig {
    let mut out = root_config.clone();
    for meta_key in ["profiles", "themes", "defaultProfile", "defaultTheme"] {
        out.remove(meta_key);
    }

    let theme_name = profile
        .and_then(|p| p.theme.as_deref())
        .or(default_theme);
    if let Some(theme) = theme_name.and_then(|n| themes.get(n)) {
        if let Ok(Value::Object(theme_map)) = serde_json::to_value(theme) {
            merge_shallow(&mut out, &theme_map);
        }
    }

    if let Some(profile) = profile {
        if !profile.config.is_empty() {
            merge_shallow(&mut out, &profile.config);
        }
    }

    // `ResolvedConfig` decodes leniently (see `lenient.rs`): a malformed key
    // is dropped on its own and every other key still applies, the way the
    // renderer's per-key reducers behaved. The struct itself cannot fail.
    serde_json::from_value::<ResolvedConfig>(Value::Object(out)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn as_map(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn merge_order_root_theme_profile() {
        let root = as_map(json!({
            "fontSize": 14,
            "backgroundColor": "#000",
            "colors": {"red": "#f00", "green": "#0f0"}
        }));
        let mut themes = indexmap::IndexMap::new();
        themes.insert(
            "night".to_string(),
            serde_json::from_value::<ThemeColors>(json!({
                "backgroundColor": "#111",
                "colors": {"red": "#e00"}
            }))
            .unwrap(),
        );
        let profile = ProfileDef {
            name: "p".into(),
            config: as_map(json!({"colors": {"green": "#1f1"}, "fontSize": 16})),
            ..Default::default()
        };

        let resolved =
            resolve_profile_config(&root, Some(&profile), &themes, Some("night"));

        // profile beats theme beats root; objects shallow-merge.
        assert_eq!(resolved.font_size, 16.0);
        assert_eq!(resolved.background_color, "#111");
        assert_eq!(resolved.colors.red.as_deref(), Some("#e00")); // theme kept
        assert_eq!(resolved.colors.green.as_deref(), Some("#1f1")); // profile override
    }

    #[test]
    fn arrays_replace_not_merge() {
        let root = as_map(json!({"shellArgs": ["--login"]}));
        let profile = ProfileDef {
            name: "p".into(),
            config: as_map(json!({"shellArgs": ["-i"]})),
            ..Default::default()
        };
        let themes = indexmap::IndexMap::new();
        let resolved = resolve_profile_config(&root, Some(&profile), &themes, None);
        assert_eq!(resolved.shell_args, vec!["-i".to_string()]);
    }

    #[test]
    fn defaults_fill_unset_fields() {
        let resolved =
            resolve_profile_config(&Map::new(), None, &indexmap::IndexMap::new(), None);
        assert_eq!(resolved.scrollback, 1000);
        assert_eq!(resolved.padding, "12px 14px");
        assert!(resolved.preserve_cwd);
    }

    /// Keys that don't follow serde's camelCase rule must still be read.
    #[test]
    fn reads_non_camel_case_config_keys() {
        let root = as_map(json!({
            "preserveCWD": false,
            "bellSoundURL": "/tmp/ding.wav",
        }));
        let resolved = resolve_profile_config(&root, None, &indexmap::IndexMap::new(), None);
        assert!(!resolved.preserve_cwd);
        assert_eq!(resolved.bell_sound_url.as_deref(), Some("/tmp/ding.wav"));
    }

    /// One malformed value must not reset every other option.
    #[test]
    fn malformed_field_does_not_discard_the_rest() {
        let root = as_map(json!({
            "fontSize": "not a number",
            "cursorColor": null,
            "backgroundColor": "#1a1b26",
            "shell": "/bin/fish",
            "scrollback": 9000,
        }));
        let resolved = resolve_profile_config(&root, None, &indexmap::IndexMap::new(), None);
        assert_eq!(resolved.background_color, "#1a1b26");
        assert_eq!(resolved.shell, "/bin/fish");
        assert_eq!(resolved.scrollback, 9000);
        // The bad ones fall back individually.
        assert_eq!(resolved.font_size, 14.0);
        assert_eq!(resolved.cursor_color, "rgba(248,28,229,0.8)");
    }

    /// The documented array form of `colors` (`getColorMap`).
    #[test]
    fn colors_accept_the_array_form() {
        let root = as_map(json!({
            "colors": [
                "#000000", "#C51E14", "#1DC121", "#C7C329", "#0A2FC4", "#C839C5", "#20C5C6",
                "#C7C7C7", "#686868", "#FD6F6B", "#67F86F", "#FFFA72", "#6A76FB", "#FD7CFC",
                "#68FDFE", "#FFFFFF",
                // colorCubes / grayscale trail the 16 ANSI slots and are ignored.
                {}, {}
            ],
            "fontSize": 16,
        }));
        let resolved = resolve_profile_config(&root, None, &indexmap::IndexMap::new(), None);
        assert_eq!(resolved.colors.black.as_deref(), Some("#000000"));
        assert_eq!(resolved.colors.light_white.as_deref(), Some("#FFFFFF"));
        assert_eq!(resolved.colors.red.as_deref(), Some("#C51E14"));
        assert_eq!(resolved.font_size, 16.0, "array colors don't break siblings");
    }

    /// Themes carry non-color keys too; `mergeShallow` applied all of them.
    #[test]
    fn theme_extra_keys_merge() {
        let mut themes = indexmap::IndexMap::new();
        themes.insert(
            "compact".to_string(),
            serde_json::from_value::<ThemeColors>(json!({
                "backgroundColor": "#111",
                "padding": "0px",
                "cursorShape": "BEAM"
            }))
            .unwrap(),
        );
        let resolved =
            resolve_profile_config(&Map::new(), None, &themes, Some("compact"));
        assert_eq!(resolved.background_color, "#111");
        assert_eq!(resolved.padding, "0px");
        assert_eq!(resolved.cursor_shape, "BEAM");
    }
}
