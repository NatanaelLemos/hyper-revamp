//! Native menu bar (muda) — macOS only for now. Menu items dispatch the same
//! `Command`s as the keymap; accelerators are derived from the active keymap
//! so remaps show up in the menus after a config reload.
//!
//! Copy/Paste/Select All carry no accelerators on purpose: menu accelerators
//! intercept key equivalents before egui sees them, which would break
//! ⌘C/⌘V inside settings text fields. The keymap path handles those.

use std::collections::HashMap;

use hyper_config::{Command, KeyChord};

pub struct AppMenu {
    #[allow(dead_code)]
    menu: muda::Menu,
    commands: HashMap<muda::MenuId, Command>,
}

/// The muda accelerator spelling of a chord's key, or None when muda has no
/// name for it. The original passed the raw keymap string to Electron, which
/// accepted Tab/PageUp/Home/Esc/F-keys, so anything muda understands is
/// carried through rather than silently dropping the accelerator.
fn accel_key(key: &str) -> Option<String> {
    let name = match key {
        k if k.len() == 1 && k.chars().next().unwrap().is_ascii_alphanumeric() => {
            return Some(k.to_uppercase())
        }
        "plus" | "=" => "=",
        "minus" | "-" => "-",
        "," | "." | "/" | ";" | "'" | "\\" | "[" | "]" | "`" => return Some(key.to_string()),
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "enter" | "return" => "Enter",
        "tab" => "Tab",
        "space" => "Space",
        "backspace" => "Backspace",
        "delete" | "del" => "Delete",
        "ins" | "insert" => "Insert",
        "esc" | "escape" => "Escape",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        other
            if other.len() >= 2
                && other.starts_with('f')
                && other[1..].chars().all(|c| c.is_ascii_digit()) =>
        {
            return Some(other.to_uppercase())
        }
        _ => return None,
    };
    Some(name.to_string())
}

/// First keymap chord for a command, as a muda accelerator string.
fn accel_for(keymap: &[(KeyChord, Command)], command: &Command) -> Option<String> {
    let chord = keymap
        .iter()
        .find(|(_, c)| c == command)
        .map(|(chord, _)| chord)?;
    let mut parts = Vec::new();
    if chord.command {
        parts.push("Cmd".to_string());
    }
    if chord.ctrl {
        parts.push("Ctrl".to_string());
    }
    if chord.alt {
        parts.push("Alt".to_string());
    }
    if chord.shift {
        parts.push("Shift".to_string());
    }
    parts.push(accel_key(&chord.key)?);
    Some(parts.join("+"))
}

fn parse_accel(accel: Option<String>) -> Option<muda::accelerator::Accelerator> {
    accel.and_then(|s| s.parse().ok())
}

impl AppMenu {
    /// Build and install the menu bar. macOS only; returns None elsewhere.
    pub fn install(keymap: &[(KeyChord, Command)], profiles: &[String]) -> Option<AppMenu> {
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (keymap, profiles);
            None
        }
        #[cfg(target_os = "macos")]
        {
            use muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};

            let menu = Menu::new();
            let mut commands = HashMap::new();

            let mut item = |label: &str, command: Command, with_accel: bool| {
                let accel = with_accel
                    .then(|| parse_accel(accel_for(keymap, &command)))
                    .flatten();
                let item = MenuItem::new(label, true, accel);
                commands.insert(item.id().clone(), command);
                item
            };

            // App menu (name comes from the bundle).
            let app_menu = Submenu::new("Hyper Revamp", true);
            let _ = app_menu.append_items(&[
                &PredefinedMenuItem::about(None, None),
                &PredefinedMenuItem::separator(),
                &item("Settings…", Command::Preferences, true),
                &item("Open config file", Command::OpenConfigFile, true),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::services(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::quit(None),
            ]);
            let _ = menu.append(&app_menu);

            // Shell.
            let shell = Submenu::new("Shell", true);
            let _ = shell.append_items(&[
                &item("New Tab", Command::NewTab, true),
                &PredefinedMenuItem::separator(),
                &item("Split Right", Command::SplitRight, true),
                &item("Split Down", Command::SplitDown, true),
                &PredefinedMenuItem::separator(),
            ]);
            // Per-profile submenus (Electron shell-menu parity).
            for name in profiles {
                let per_profile = Submenu::new(name, true);
                let _ = per_profile.append_items(&[
                    &item("New Tab", Command::NewTabProfile(name.clone()), true),
                    &PredefinedMenuItem::separator(),
                    &item("Split Right", Command::SplitRightProfile(name.clone()), true),
                    &item("Split Down", Command::SplitDownProfile(name.clone()), true),
                ]);
                let _ = shell.append(&per_profile);
            }
            let _ = shell.append_items(&[
                &PredefinedMenuItem::separator(),
                &item("Close Pane", Command::ClosePane, true),
            ]);
            let _ = menu.append(&shell);

            // Edit — no accelerators (would steal ⌘C/⌘V from egui widgets).
            let edit = Submenu::new("Edit", true);
            let _ = edit.append_items(&[
                &item("Copy", Command::Copy, false),
                &item("Paste", Command::Paste, false),
                &item("Select All", Command::SelectAll, false),
                &PredefinedMenuItem::separator(),
                &item("Find", Command::Search, true),
                &item("Clear Buffer", Command::ClearBuffer, true),
            ]);
            let _ = menu.append(&edit);

            // View.
            let view = Submenu::new("View", true);
            let _ = view.append_items(&[
                &item("Zoom In", Command::ZoomIn, true),
                &item("Zoom Out", Command::ZoomOut, true),
                &item("Actual Size", Command::ZoomReset, true),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::fullscreen(None),
            ]);
            let _ = menu.append(&view);

            // Window.
            let window = Submenu::new("Window", true);
            let _ = window.append_items(&[
                &item("Minimize", Command::MinimizeWindow, true),
                &item("Zoom", Command::ZoomWindow, true),
                &PredefinedMenuItem::separator(),
                &item("Next Tab", Command::NextTab, true),
                &item("Previous Tab", Command::PrevTab, true),
            ]);
            let _ = menu.append(&window);

            menu.init_for_nsapp();
            Some(AppMenu { menu, commands })
        }
    }

    /// Drain menu clicks into Commands. Call once per frame.
    pub fn poll(&self) -> Vec<Command> {
        let mut out = Vec::new();
        while let Ok(event) = muda::MenuEvent::receiver().try_recv() {
            if let Some(cmd) = self.commands.get(event.id()) {
                out.push(cmd.clone());
            }
        }
        out
    }
}
