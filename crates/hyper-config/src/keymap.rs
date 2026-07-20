use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A command binding value in config: `"cmd+t"` or `["cmd+t", "ctrl+t"]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        match self {
            OneOrMany::One(s) => std::slice::from_ref(s).iter(),
            OneOrMany::Many(v) => v.iter(),
        }
    }
}

/// GUI-toolkit-agnostic key chord parsed from a mousetrap-style string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyChord {
    /// The Meta key: ⌘ on macOS, Super/Win elsewhere (mousetrap's `meta`).
    pub command: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Lowercased key name: "a", "1", "[", "left", "esc", "plus", "f5", …
    pub key: String,
}

/// Parse "command+shift+d" etc.
///
/// Modifier names follow mousetrap: `cmd`/`command`/`meta` are the Meta key,
/// `option` aliases `alt`, and `mod` is platform-dependent — Meta on macOS,
/// Ctrl everywhere else (`_SPECIAL_ALIASES` in mousetrap). Resolving `mod`
/// here keeps `KeyChord` a plain description of physical modifiers.
pub fn parse_chord(binding: &str) -> Option<KeyChord> {
    let mut chord = KeyChord {
        command: false,
        ctrl: false,
        alt: false,
        shift: false,
        key: String::new(),
    };
    let lower = binding.trim().to_lowercase();
    if lower.is_empty() {
        return None;
    }
    // Mousetrap's `_keysFromString`: a lone "+" is the plus key, and "++"
    // means "+plus" (so "command++" is command plus the '+' key).
    let normalized = if lower == "+" {
        "plus".to_string()
    } else {
        lower.replace("++", "+plus")
    };

    let parts: Vec<&str> = normalized.split('+').collect();
    let (mods, key) = parts.split_at(parts.len().checked_sub(1)?);
    for m in mods {
        match *m {
            "command" | "cmd" | "meta" => chord.command = true,
            "mod" => {
                if cfg!(target_os = "macos") {
                    chord.command = true;
                } else {
                    chord.ctrl = true;
                }
            }
            "ctrl" | "control" => chord.ctrl = true,
            "alt" | "option" => chord.alt = true,
            "shift" => chord.shift = true,
            other => {
                log::warn!("unknown modifier '{other}' in binding '{binding}'");
                return None;
            }
        }
    }
    let key = key.first()?.trim();
    if key.is_empty() {
        return None;
    }
    chord.key = key.to_string();
    Some(chord)
}

/// Every app command reachable from keymaps/menus. String forms match the
/// Hyper command names (`app/commands.ts`).
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    NewWindow,
    NewWindowProfile(String),
    CloseWindow,
    MinimizeWindow,
    ZoomWindow,
    ToggleFullScreen,
    Preferences,
    OpenConfigFile,
    ZoomReset,
    ZoomIn,
    ZoomOut,
    NewTab,
    NewTabProfile(String),
    NextTab,
    PrevTab,
    JumpTab(u8),
    JumpTabLast,
    NextPane,
    PrevPane,
    SplitRight,
    SplitRightProfile(String),
    SplitDown,
    SplitDownProfile(String),
    ClosePane,
    Copy,
    Paste,
    SelectAll,
    Search,
    SearchClose,
    ClearBuffer,
    /// Fixed byte sequence written to the PTY (editor:move*/delete*/break).
    PtyBytes(&'static [u8]),
}

/// Map a Hyper command name to a Command. Unknown / web-only commands
/// (devtools, reload, undo/redo/cut, plugins) return None and are skipped.
pub fn parse_command(name: &str) -> Option<Command> {
    use Command::*;
    let cmd = match name {
        "window:new" => NewWindow,
        "window:close" => CloseWindow,
        "window:minimize" => MinimizeWindow,
        "window:zoom" => ZoomWindow,
        "window:toggleFullScreen" => ToggleFullScreen,
        "window:preferences" => Preferences,
        "window:openConfigFile" => OpenConfigFile,
        "zoom:reset" => ZoomReset,
        "zoom:in" => ZoomIn,
        "zoom:out" => ZoomOut,
        "tab:new" => NewTab,
        "tab:next" => NextTab,
        "tab:prev" => PrevTab,
        "pane:next" => NextPane,
        "pane:prev" => PrevPane,
        "pane:splitRight" => SplitRight,
        "pane:splitDown" => SplitDown,
        "pane:close" => ClosePane,
        "editor:copy" => Copy,
        "editor:paste" => Paste,
        "editor:selectAll" => SelectAll,
        "editor:search" => Search,
        "editor:search-close" => SearchClose,
        "editor:clearBuffer" => ClearBuffer,
        // Byte sequences match the renderer's rpc handlers in `lib/index.tsx`.
        "editor:break" => PtyBytes(b"\x03"),
        "editor:movePreviousWord" => PtyBytes(b"\x1bb"),
        "editor:moveNextWord" => PtyBytes(b"\x1bf"),
        "editor:moveBeginningLine" => PtyBytes(b"\x1bOH"),
        "editor:moveEndLine" => PtyBytes(b"\x1bOF"),
        "editor:deletePreviousWord" => PtyBytes(b"\x1b\x7f"),
        "editor:deleteNextWord" => PtyBytes(b"\x1bd"),
        "editor:deleteBeginningLine" => PtyBytes(b"\x1bw"),
        // `lib/index.tsx` sends "\x10B" (DLE + 'B') here, which no shell binds
        // — it types garbage. That is an upstream typo for \x0B (ctrl-K,
        // kill-to-end-of-line), which is what this command is meant to do and
        // what every keymap documents it as; send the working sequence.
        "editor:deleteEndLine" => PtyBytes(b"\x0b"),
        "tab:jump:last" => JumpTabLast,
        other => {
            // Concrete jump slots. `commands.ts` registers tab:jump:1…8 and
            // tab:jump:last, so a user can rebind one slot on its own.
            if let Some(index) = other.strip_prefix("tab:jump:") {
                return match index.parse::<u8>() {
                    Ok(n @ 1..=8) => Some(JumpTab(n)),
                    _ => None,
                };
            }
            // Per-profile commands: "<base>:<profile name>".
            for (prefix, ctor) in [
                ("tab:new:", NewTabProfile as fn(String) -> Command),
                ("window:new:", NewWindowProfile),
                ("pane:splitRight:", SplitRightProfile),
                ("pane:splitDown:", SplitDownProfile),
            ] {
                if let Some(profile) = other.strip_prefix(prefix) {
                    return Some(ctor(profile.to_string()));
                }
            }
            return None;
        }
    };
    Some(cmd)
}

const DARWIN_KEYMAP: &str = include_str!("../../../assets/keymaps/darwin.json");
const LINUX_KEYMAP: &str = include_str!("../../../assets/keymaps/linux.json");
const WIN32_KEYMAP: &str = include_str!("../../../assets/keymaps/win32.json");

fn platform_defaults_json() -> &'static str {
    if cfg!(target_os = "macos") {
        DARWIN_KEYMAP
    } else if cfg!(target_os = "windows") {
        WIN32_KEYMAP
    } else {
        LINUX_KEYMAP
    }
}

/// Fully resolved keymap: chord → command, defaults merged with user
/// overrides (user replaces per command), `<base>:prefix` expanded.
///
/// Ordering is deterministic (`BTreeMap`): the dispatch order of equally
/// specific bindings must not change from run to run.
pub fn build_keymap(user: &HashMap<String, OneOrMany>) -> Vec<(KeyChord, Command)> {
    let defaults: HashMap<String, OneOrMany> =
        serde_json::from_str(platform_defaults_json()).unwrap_or_default();
    let mut merged: std::collections::BTreeMap<String, OneOrMany> =
        defaults.into_iter().collect();
    for (cmd, bindings) in user {
        merged.insert(cmd.clone(), bindings.clone());
    }

    // `generatePrefixedCommand` in `app/utils/map-keys.ts`: any command
    // ending in `:prefix` expands to `<base>:1`…`<base>:8` plus `<base>:last`
    // (slot 9), binding "<shortcut>+<n>". The expansion lands in the same
    // command namespace, so an explicit `tab:jump:1` entry overrides it.
    let mut resolved: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for (name, bindings) in &merged {
        let Some(base) = name.strip_suffix(":prefix") else {
            continue;
        };
        for n in 1..=9u8 {
            let slot = if n == 9 {
                "last".to_string()
            } else {
                n.to_string()
            };
            resolved
                .entry(format!("{base}:{slot}"))
                .or_default()
                .extend(bindings.iter().map(|binding| format!("{binding}+{n}")));
        }
    }
    for (name, bindings) in &merged {
        if name.ends_with(":prefix") {
            continue;
        }
        resolved.insert(name.clone(), bindings.iter().cloned().collect());
    }

    let mut out = Vec::new();
    for (name, bindings) in &resolved {
        let Some(command) = parse_command(name) else {
            continue;
        };
        for binding in bindings {
            if let Some(chord) = parse_chord(binding) {
                out.push((chord, command.clone()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_and_aliased_chords() {
        assert_eq!(
            parse_chord("command+shift+d"),
            Some(KeyChord {
                command: true,
                ctrl: false,
                alt: false,
                shift: true,
                key: "d".into()
            })
        );
        assert_eq!(parse_chord("cmd+,").unwrap().key, ",");
        assert_eq!(parse_chord("command+plus").unwrap().key, "plus");
        assert_eq!(parse_chord("command+=").unwrap().key, "=");
        assert_eq!(parse_chord("esc").unwrap().key, "esc");
    }

    #[test]
    fn parses_mousetrap_plus_forms() {
        // "++" is mousetrap's escape for the literal '+' key.
        let chord = parse_chord("command++").expect("command++ parses");
        assert!(chord.command);
        assert_eq!(chord.key, "plus");
        assert_eq!(parse_chord("ctrl++").unwrap().key, "plus");
        assert_eq!(parse_chord("+").unwrap().key, "plus");
        assert_eq!(parse_chord(""), None);
    }

    #[test]
    fn mod_resolves_per_platform() {
        let chord = parse_chord("mod+t").expect("mod+t parses");
        if cfg!(target_os = "macos") {
            assert!(chord.command && !chord.ctrl);
        } else {
            assert!(chord.ctrl && !chord.command);
        }
    }

    #[test]
    fn jump_slots_parse_and_override_the_prefix() {
        assert_eq!(parse_command("tab:jump:1"), Some(Command::JumpTab(1)));
        assert_eq!(parse_command("tab:jump:last"), Some(Command::JumpTabLast));
        assert_eq!(parse_command("tab:jump:9"), None);

        let mut user = HashMap::new();
        user.insert("tab:jump:1".to_string(), OneOrMany::One("alt+1".into()));
        let map = build_keymap(&user);
        let slots: Vec<_> = map
            .iter()
            .filter(|(_, cmd)| *cmd == Command::JumpTab(1))
            .collect();
        assert_eq!(slots.len(), 1, "explicit slot replaces the expanded one");
        assert!(slots[0].0.alt && slots[0].0.key == "1");
        // The other slots still come from the prefix expansion.
        assert!(map.iter().any(|(_, cmd)| *cmd == Command::JumpTab(2)));
        assert!(map.iter().any(|(_, cmd)| *cmd == Command::JumpTabLast));
    }

    #[test]
    fn editor_sequences_match_the_renderer() {
        assert_eq!(
            parse_command("editor:moveBeginningLine"),
            Some(Command::PtyBytes(b"\x1bOH"))
        );
        assert_eq!(
            parse_command("editor:deletePreviousWord"),
            Some(Command::PtyBytes(b"\x1b\x7f"))
        );
        assert_eq!(
            parse_command("editor:deleteBeginningLine"),
            Some(Command::PtyBytes(b"\x1bw"))
        );
    }

    #[test]
    fn default_darwin_keymap_builds() {
        let map = build_keymap(&HashMap::new());
        // tab:jump expansion adds 9 chords; core commands present.
        assert!(map
            .iter()
            .any(|(c, cmd)| *cmd == Command::NewTab && c.command && c.key == "t"));
        assert!(map.iter().any(|(_, cmd)| *cmd == Command::JumpTabLast));
        assert!(map
            .iter()
            .any(|(c, cmd)| *cmd == Command::SplitRight && c.key == "d" && !c.shift));
        // multi-binding commands expand (tab:next has 4 bindings).
        assert_eq!(
            map.iter().filter(|(_, cmd)| *cmd == Command::NextTab).count(),
            4
        );
    }

    #[test]
    fn user_overrides_replace_defaults() {
        let mut user = HashMap::new();
        user.insert("tab:new".to_string(), OneOrMany::One("ctrl+shift+t".into()));
        let map = build_keymap(&user);
        let tab_new: Vec<_> = map
            .iter()
            .filter(|(_, cmd)| *cmd == Command::NewTab)
            .collect();
        assert_eq!(tab_new.len(), 1);
        assert!(tab_new[0].0.ctrl && tab_new[0].0.shift && !tab_new[0].0.command);
    }

    #[test]
    fn profile_commands_parse() {
        assert_eq!(
            parse_command("tab:new:Work SSH"),
            Some(Command::NewTabProfile("Work SSH".into()))
        );
    }
}
