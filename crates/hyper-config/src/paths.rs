use std::path::PathBuf;

/// Config directory, matching the Electron app (`app/config/paths.ts`):
/// `$XDG_CONFIG_HOME/hyper-revamp`, else `~/.config/hyper-revamp` on
/// macOS/Linux, else the platform config dir on Windows.
pub fn cfg_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("hyper-revamp");
        }
    }
    if cfg!(windows) {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("hyper-revamp")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config")
            .join("hyper-revamp")
    }
}

pub fn cfg_path() -> PathBuf {
    cfg_dir().join("hyper-revamp.json")
}

pub fn themes_dir() -> PathBuf {
    cfg_dir().join("themes")
}

/// Target of the default config's `$schema: "./schema.json"` reference.
pub fn schema_path() -> PathBuf {
    cfg_dir().join("schema.json")
}

pub fn session_state_path() -> PathBuf {
    cfg_dir().join("session-state.json")
}

pub fn windows_state_path() -> PathBuf {
    cfg_dir().join("windows-state.json")
}

/// Single-instance socket. Unix socket paths are limited to ~104 bytes on
/// macOS (SUN_LEN), so fall back to a short temp-dir path — keyed by a hash
/// of the config dir so distinct configs get distinct instances.
pub fn socket_path() -> PathBuf {
    let preferred = cfg_dir().join("hyper-revamp.sock");
    if preferred.as_os_str().len() < 100 {
        return preferred;
    }
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cfg_dir().hash(&mut hasher);
    std::env::temp_dir().join(format!("hyper-revamp-{:016x}.sock", hasher.finish()))
}
