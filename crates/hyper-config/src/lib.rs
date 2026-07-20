//! Config model, themes, and keymaps for hyper-revamp.
//!
//! Reads/writes the same `~/.config/hyper-revamp/hyper-revamp.json` file as
//! the Electron app, tolerating and preserving unknown keys.

pub mod keymap;
pub mod lenient;
pub mod model;
pub mod paths;
pub mod resolve;
pub mod service;
pub mod theme;
pub mod watcher;

pub use keymap::{build_keymap, parse_chord, parse_command, Command, KeyChord, OneOrMany};
pub use model::{
    Bell, ModifierKeys, ProfileDef, ResolvedConfig, SshProfileDef, TypedRoot, WindowOpacity,
};
pub use resolve::resolve_profile_config;
pub use service::{save_document, save_text, ConfigSnapshot, DEFAULT_CONFIG_JSON};
pub use theme::{bundled_themes, parse_color, ColorMap, FontWeight, ThemeColors};
pub use watcher::{watch, ConfigWatcher};
