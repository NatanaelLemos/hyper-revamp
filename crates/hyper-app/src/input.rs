use alacritty_terminal::term::TermMode;
use egui::{Key, Modifiers};
use hyper_config::KeyChord;

/// The egui key a config chord names, or None when egui can't represent it.
///
/// Mousetrap's key names are lowercase; `Key::from_name` only accepts egui's
/// own capitalization ("F11", "Insert", "A"), so every name a keymap can use
/// is mapped explicitly and `from_name` is only a last resort for the
/// single-character keys, tried in egui's spelling.
pub fn chord_key(chord: &KeyChord) -> Option<Key> {
    let key = match chord.key.as_str() {
        "plus" => Key::Plus,
        "minus" | "-" => Key::Minus,
        "esc" | "escape" => Key::Escape,
        "enter" | "return" => Key::Enter,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "ins" | "insert" => Key::Insert,
        "tab" => Key::Tab,
        "left" => Key::ArrowLeft,
        "right" => Key::ArrowRight,
        "up" => Key::ArrowUp,
        "down" => Key::ArrowDown,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        // Mousetrap spells function keys "f1".."f19"; egui wants "F1".
        other if is_function_key(other) => Key::from_name(&other.to_uppercase())?,
        other => Key::from_name(other).or_else(|| Key::from_name(&other.to_uppercase()))?,
    };
    Some(key)
}

fn is_function_key(name: &str) -> bool {
    name.len() >= 2
        && name.starts_with('f')
        && name[1..].chars().all(|c| c.is_ascii_digit())
}

/// Does a pressed modifier set match what the binding asks for, exactly?
///
/// Mousetrap compared the full modifier list (`_modifiersMatch`), so `cmd+w`
/// never fired on `cmd+shift+w`. egui's `consume_key` deliberately does the
/// opposite — it ignores extra Shift/Alt and expects callers to test the most
/// specific shortcut first — which, over a keymap with no inherent order, let
/// whichever binding came first swallow the other.
pub fn chord_matches(chord: &KeyChord, pressed: Modifiers) -> bool {
    // `KeyChord::command` is the Meta key: ⌘ on macOS, Super elsewhere.
    // egui has no Super modifier, so a Meta binding is only bindable on macOS
    // (`chord_key`'s caller reports that; see `chord_is_bindable`).
    let want_cmd = cfg!(target_os = "macos") && chord.command;
    // Off macOS egui reports Ctrl in both `ctrl` and `command`; compare `ctrl`.
    let want_ctrl = chord.ctrl;
    pressed.alt == chord.alt
        && pressed.shift == chord.shift
        && pressed.ctrl == want_ctrl
        && pressed.mac_cmd == want_cmd
}

/// Meta bindings can't be expressed on platforms without a ⌘ key: egui has no
/// Super/Win modifier, and treating Meta as Ctrl would hijack unrelated
/// bindings (a `command+c` keymap entry would eat Ctrl+C, so the shell would
/// never see SIGINT). Such a binding simply doesn't fire, as on the original
/// where it needed a Super key press.
pub fn chord_is_bindable(chord: &KeyChord) -> bool {
    if chord.command && !cfg!(target_os = "macos") {
        return false;
    }
    true
}

/// Consume every key event matching `chord` exactly; returns how many fired.
pub fn consume_chord(ctx: &egui::Context, chord: &KeyChord) -> usize {
    if !chord_is_bindable(chord) {
        return 0;
    }
    let Some(key) = chord_key(chord) else {
        return 0;
    };
    ctx.input_mut(|input| {
        let mut count = 0;
        input.events.retain(|event| {
            let is_match = matches!(
                event,
                egui::Event::Key {
                    key: event_key,
                    modifiers,
                    pressed: true,
                    ..
                } if *event_key == key && chord_matches(chord, *modifiers)
            );
            count += usize::from(is_match);
            !is_match
        });
        count
    })
}

/// Options affecting key encoding, from the resolved profile config.
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyOptions {
    /// modifierKeys.altIsMeta — send ESC+char for option-modified keys.
    pub alt_is_meta: bool,
}

/// xterm-style modifier parameter: 1 + (shift=1, alt=2, ctrl=4, meta=8).
fn mod_param(m: Modifiers) -> u8 {
    1 + (m.shift as u8) + ((m.alt as u8) << 1) + ((m.ctrl as u8) << 2) + ((m.mac_cmd as u8) << 3)
}

fn csi(seq: &str) -> Vec<u8> {
    let mut v = vec![0x1b, b'['];
    v.extend_from_slice(seq.as_bytes());
    v
}

/// Kitty keyboard protocol CSI u encoding: `ESC [ code ; mod u`, with the
/// `;mod` omitted when no modifier is held.
fn csi_u(code: u32, mods: Modifiers) -> Vec<u8> {
    let m = mod_param(mods);
    if m == 1 {
        csi(&format!("{code}u"))
    } else {
        csi(&format!("{code};{m}u"))
    }
}

/// Codepoint the kitty protocol reports for a key: the unshifted base
/// character. Only single-character key names qualify; functional keys
/// keep their legacy escapes.
fn kitty_codepoint(key: Key) -> Option<u32> {
    if key == Key::Space {
        return Some(32);
    }
    let name = key.name();
    if name.len() != 1 {
        return None;
    }
    let c = name.chars().next()?;
    Some(c.to_ascii_lowercase() as u32)
}

/// Encode arrow/home/end respecting DECCKM (application cursor keys) and
/// modifiers (CSI 1;<mod> <ch>).
fn cursor_key(ch: char, mods: Modifiers, mode: TermMode) -> Vec<u8> {
    if mods.any() {
        csi(&format!("1;{}{}", mod_param(mods), ch))
    } else if mode.contains(TermMode::APP_CURSOR) {
        vec![0x1b, b'O', ch as u8]
    } else {
        csi(&ch.to_string())
    }
}

/// Tilde-coded keys (Insert/Delete/PgUp/PgDn/F5+) with modifier support.
fn tilde_key(num: u8, mods: Modifiers) -> Vec<u8> {
    if mods.any() {
        csi(&format!("{};{}~", num, mod_param(mods)))
    } else {
        csi(&format!("{num}~"))
    }
}

/// F1-F4 are SS3-coded when unmodified (`ESC O P..S`) and CSI-coded with a
/// modifier parameter otherwise (`ESC [ 1;<mod> P..S`), as in xterm.js.
fn function_key_1_to_4(ch: char, mods: Modifiers) -> Vec<u8> {
    if mods.any() {
        csi(&format!("1;{}{}", mod_param(mods), ch))
    } else {
        vec![0x1b, b'O', ch as u8]
    }
}

/// Encode a non-text key press to PTY bytes. Returns None when the key
/// produces no bytes itself (printables arrive via `Event::Text`).
pub fn encode_key(key: Key, mods: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    // cmd-modified keys are app shortcuts on macOS, never terminal input.
    if mods.mac_cmd || mods.command && cfg!(target_os = "macos") {
        return None;
    }

    // Kitty keyboard protocol, "disambiguate escape codes" level (apps like
    // Claude Code, fish 4 and neovim push this via CSI > 1 u; alacritty_terminal
    // tracks it in TermMode). Esc and any modified Enter/Tab/Backspace/ctrl-key
    // must arrive as CSI u so the app can tell e.g. shift+enter from enter.
    // Unmodified Enter/Tab/Backspace keep their legacy bytes per the spec so a
    // shell stuck in this mode stays usable.
    if mode.contains(TermMode::DISAMBIGUATE_ESC_CODES) {
        match key {
            Key::Escape => return Some(csi_u(27, mods)),
            Key::Enter if mods.any() => return Some(csi_u(13, mods)),
            Key::Tab if mods.any() => return Some(csi_u(9, mods)),
            Key::Backspace if mods.any() => return Some(csi_u(127, mods)),
            _ if mods.ctrl => {
                if let Some(cp) = kitty_codepoint(key) {
                    return Some(csi_u(cp, mods));
                }
            }
            _ => {}
        }
    }

    // Ctrl+letter → C0 control byte (ctrl+c = 0x03 etc.).
    if mods.ctrl && !mods.alt {
        let c0 = match key {
            Key::A => Some(0x01u8),
            Key::B => Some(0x02),
            Key::C => Some(0x03),
            Key::D => Some(0x04),
            Key::E => Some(0x05),
            Key::F => Some(0x06),
            Key::G => Some(0x07),
            Key::H => Some(0x08),
            Key::I => Some(0x09),
            Key::J => Some(0x0a),
            Key::K => Some(0x0b),
            Key::L => Some(0x0c),
            Key::M => Some(0x0d),
            Key::N => Some(0x0e),
            Key::O => Some(0x0f),
            Key::P => Some(0x10),
            Key::Q => Some(0x11),
            Key::R => Some(0x12),
            Key::S => Some(0x13),
            Key::T => Some(0x14),
            Key::U => Some(0x15),
            Key::V => Some(0x16),
            Key::W => Some(0x17),
            Key::X => Some(0x18),
            Key::Y => Some(0x19),
            Key::Z => Some(0x1a),
            Key::OpenBracket => Some(0x1b),
            Key::Backslash => Some(0x1c),
            Key::CloseBracket => Some(0x1d),
            Key::Space => Some(0x00),
            // xterm.js's non-letter ctrl mappings. These keys emit no Text
            // event while ctrl is held, so without them they produced nothing
            // at all: ctrl+_ is readline's undo, ctrl+8 is DEL.
            Key::Minus if mods.shift => Some(0x1f), // ctrl+_
            Key::Num2 if mods.shift => Some(0x00),  // ctrl+@
            Key::Num3 => Some(0x1b),
            Key::Num4 => Some(0x1c),
            Key::Num5 => Some(0x1d),
            Key::Num6 => Some(0x1e),
            Key::Num7 => Some(0x1f),
            Key::Num8 => Some(0x7f),
            _ => None,
        };
        if let Some(byte) = c0 {
            return Some(vec![byte]);
        }
    }

    match key {
        Key::Enter => {
            if mods.shift || mods.alt {
                // alt+enter is xterm.js's `ESC CR`. shift+enter has no legacy
                // encoding there (it sends a bare CR), but this app
                // deliberately sends the same meta-enter: it is what Claude
                // Code and other REPLs treat as newline-insert, and the
                // binding iTerm2's /terminal-setup installs. Apps that want to
                // tell the two apart enable the kitty protocol above.
                Some(vec![0x1b, b'\r'])
            } else {
                Some(vec![b'\r'])
            }
        }
        // xterm.js: shift→BS (0x08), alt→ESC DEL, otherwise (ctrl included)
        // DEL (0x7f). Sending 0x08 for ctrl+backspace and 0x7f for
        // shift+backspace had these exactly inverted, so apps that
        // distinguish ^H from ^? (emacs' C-h help prefix, custom readline
        // bindings) saw the opposite key from the original.
        Key::Backspace => {
            if mods.alt {
                Some(vec![0x1b, 0x7f])
            } else if mods.shift {
                Some(vec![0x08])
            } else {
                Some(vec![0x7f])
            }
        }
        Key::Tab => {
            if mods.shift {
                Some(csi("Z"))
            } else {
                Some(vec![b'\t'])
            }
        }
        // alt+Escape is ESC ESC in xterm.js (`ev.altKey ? ESC+ESC : ESC`).
        Key::Escape => {
            if mods.alt {
                Some(vec![0x1b, 0x1b])
            } else {
                Some(vec![0x1b])
            }
        }
        Key::ArrowUp => Some(arrow_key('A', mods, mode)),
        Key::ArrowDown => Some(arrow_key('B', mods, mode)),
        Key::ArrowRight => Some(arrow_key('C', mods, mode)),
        Key::ArrowLeft => Some(arrow_key('D', mods, mode)),
        Key::Home => Some(cursor_key('H', strip_mods_for_cursor(mods), mode)),
        Key::End => Some(cursor_key('F', strip_mods_for_cursor(mods), mode)),
        Key::Insert => Some(tilde_key(2, strip_meta(mods))),
        Key::Delete => Some(tilde_key(3, strip_meta(mods))),
        Key::PageUp => Some(tilde_key(5, strip_meta(mods))),
        Key::PageDown => Some(tilde_key(6, strip_meta(mods))),
        // F-keys keep every modifier: ctrl+F5 is `ESC[15;5~`, shift+F1 is
        // `ESC[1;2P`. Stripping ctrl here made ctrl+F5 arrive as a bare F5.
        Key::F1 => Some(function_key_1_to_4('P', strip_meta(mods))),
        Key::F2 => Some(function_key_1_to_4('Q', strip_meta(mods))),
        Key::F3 => Some(function_key_1_to_4('R', strip_meta(mods))),
        Key::F4 => Some(function_key_1_to_4('S', strip_meta(mods))),
        Key::F5 => Some(tilde_key(15, strip_meta(mods))),
        Key::F6 => Some(tilde_key(17, strip_meta(mods))),
        Key::F7 => Some(tilde_key(18, strip_meta(mods))),
        Key::F8 => Some(tilde_key(19, strip_meta(mods))),
        Key::F9 => Some(tilde_key(20, strip_meta(mods))),
        Key::F10 => Some(tilde_key(21, strip_meta(mods))),
        Key::F11 => Some(tilde_key(23, strip_meta(mods))),
        Key::F12 => Some(tilde_key(24, strip_meta(mods))),
        _ => None,
    }
}

/// Arrows, with xterm.js's platform special-case: alt+←/→ is the readline
/// word-jump (`ESC b` / `ESC f`) on macOS — the binding option+arrow relies
/// on — and ctrl+←/→ elsewhere.
fn arrow_key(ch: char, mods: Modifiers, mode: TermMode) -> Vec<u8> {
    let stripped = strip_mods_for_cursor(mods);
    if stripped.alt && !stripped.ctrl && !stripped.shift && matches!(ch, 'C' | 'D') {
        if cfg!(target_os = "macos") {
            return vec![0x1b, if ch == 'D' { b'b' } else { b'f' }];
        }
        let ctrl_only = Modifiers {
            alt: false,
            ctrl: true,
            ..stripped
        };
        return cursor_key(ch, ctrl_only, mode);
    }
    cursor_key(ch, stripped, mode)
}

/// Drop the Cmd/Meta bits before computing an xterm modifier parameter: they
/// aren't part of the CSI encoding, and on Linux/Windows egui mirrors Ctrl
/// into `command`, which would otherwise double-count.
fn strip_meta(mods: Modifiers) -> Modifiers {
    Modifiers {
        command: false,
        mac_cmd: false,
        ..mods
    }
}

fn strip_mods_for_cursor(mods: Modifiers) -> Modifiers {
    strip_meta(mods)
}

/// modifierKeys.altIsMeta: option+key sends ESC+base-char instead of the
/// macOS composed character (CSI u form while the kitty protocol is
/// active). Letters and digits only; other keys fall through to the
/// composed text.
pub fn alt_meta_bytes(key: Key, mods: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    let name = key.name();
    let base = if name.len() == 1 {
        name.chars().next()?
    } else {
        return None;
    };
    let ch = if base.is_ascii_alphabetic() {
        if mods.shift {
            base.to_ascii_uppercase()
        } else {
            base.to_ascii_lowercase()
        }
    } else if base.is_ascii_digit() {
        base
    } else {
        return None;
    };
    if mode.contains(TermMode::DISAMBIGUATE_ESC_CODES) {
        return Some(csi_u(ch.to_ascii_lowercase() as u32, mods));
    }
    Some(vec![0x1b, ch as u8])
}

/// macOS convention: ctrl+primary-click is a secondary click (Finder,
/// iTerm2, kitty parity). winit reports it as a plain left click with the
/// ctrl modifier, so rewrite it before egui sees it. `converted` carries
/// press→release state so the release still converts when ctrl was lifted
/// mid-click.
pub fn rewrite_ctrl_click_as_secondary(events: &mut [egui::Event], converted: &mut bool) {
    for event in events {
        let egui::Event::PointerButton {
            button,
            pressed,
            modifiers,
            ..
        } = event
        else {
            continue;
        };
        if *button != egui::PointerButton::Primary {
            continue;
        }
        if *pressed && modifiers.ctrl && !modifiers.command && !modifiers.mac_cmd {
            *button = egui::PointerButton::Secondary;
            modifiers.ctrl = false;
            *converted = true;
        } else if !*pressed && *converted {
            *button = egui::PointerButton::Secondary;
            modifiers.ctrl = false;
            *converted = false;
        }
    }
}

/// Sanitize pasted text and wrap in bracketed-paste markers when the
/// application enabled them. Ports Hyper's paste sanitization.
pub fn encode_paste(text: &str, mode: TermMode) -> Vec<u8> {
    let clean: String = text
        .replace("\r\n", "\r")
        .replace('\n', "\r")
        .chars()
        .filter(|&c| c == '\r' || c == '\t' || !c.is_control())
        .collect();

    if mode.contains(TermMode::BRACKETED_PASTE) {
        let mut out = b"\x1b[200~".to_vec();
        out.extend_from_slice(clean.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        clean.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_respect_app_cursor_mode() {
        let none = Modifiers::NONE;
        assert_eq!(
            encode_key(Key::ArrowUp, none, TermMode::empty()),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            encode_key(Key::ArrowUp, none, TermMode::APP_CURSOR),
            Some(b"\x1bOA".to_vec())
        );
    }

    #[test]
    fn ctrl_c_is_etx() {
        let mods = Modifiers {
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(
            encode_key(Key::C, mods, TermMode::empty()),
            Some(vec![0x03])
        );
    }

    #[test]
    fn kitty_disambiguate_key_encoding() {
        let kitty = TermMode::DISAMBIGUATE_ESC_CODES;
        let none = Modifiers::NONE;
        let shift = Modifiers {
            shift: true,
            ..Default::default()
        };
        let ctrl = Modifiers {
            ctrl: true,
            ..Default::default()
        };

        // shift+enter is CSI u so apps can tell it from enter…
        assert_eq!(
            encode_key(Key::Enter, shift, kitty),
            Some(b"\x1b[13;2u".to_vec())
        );
        // …while unmodified enter/tab/backspace stay legacy per the spec.
        assert_eq!(encode_key(Key::Enter, none, kitty), Some(b"\r".to_vec()));
        assert_eq!(encode_key(Key::Tab, none, kitty), Some(b"\t".to_vec()));
        assert_eq!(
            encode_key(Key::Backspace, none, kitty),
            Some(vec![0x7f])
        );

        assert_eq!(encode_key(Key::Escape, none, kitty), Some(b"\x1b[27u".to_vec()));
        assert_eq!(encode_key(Key::C, ctrl, kitty), Some(b"\x1b[99;5u".to_vec()));

        // Without the mode flag: shift+enter falls back to ESC CR
        // (meta-enter), everything else is unchanged.
        assert_eq!(
            encode_key(Key::Enter, shift, TermMode::empty()),
            Some(b"\x1b\r".to_vec())
        );
        assert_eq!(
            encode_key(Key::Enter, Modifiers::NONE, TermMode::empty()),
            Some(b"\r".to_vec())
        );
        assert_eq!(
            encode_key(Key::C, ctrl, TermMode::empty()),
            Some(vec![0x03])
        );
    }

    #[test]
    fn alt_meta_respects_kitty_mode() {
        let alt = Modifiers {
            alt: true,
            ..Default::default()
        };
        assert_eq!(
            alt_meta_bytes(Key::B, alt, TermMode::empty()),
            Some(vec![0x1b, b'b'])
        );
        assert_eq!(
            alt_meta_bytes(Key::B, alt, TermMode::DISAMBIGUATE_ESC_CODES),
            Some(b"\x1b[98;3u".to_vec())
        );
    }

    fn button_event(button: egui::PointerButton, pressed: bool, ctrl: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: egui::Pos2::ZERO,
            button,
            pressed,
            modifiers: Modifiers {
                ctrl,
                ..Default::default()
            },
        }
    }

    fn button_of(event: &egui::Event) -> (egui::PointerButton, bool, bool) {
        match event {
            egui::Event::PointerButton {
                button,
                pressed,
                modifiers,
                ..
            } => (*button, *pressed, modifiers.ctrl),
            _ => panic!("not a pointer button event"),
        }
    }

    #[test]
    fn ctrl_click_becomes_secondary_click() {
        use egui::PointerButton::{Primary, Secondary};

        // Press with ctrl converts and strips the modifier; the release
        // converts too even though ctrl was lifted in between.
        let mut converted = false;
        let mut events = vec![button_event(Primary, true, true)];
        rewrite_ctrl_click_as_secondary(&mut events, &mut converted);
        assert_eq!(button_of(&events[0]), (Secondary, true, false));
        assert!(converted);

        let mut events = vec![button_event(Primary, false, false)];
        rewrite_ctrl_click_as_secondary(&mut events, &mut converted);
        assert_eq!(button_of(&events[0]), (Secondary, false, false));
        assert!(!converted);

        // A plain click stays primary.
        let mut events = vec![
            button_event(Primary, true, false),
            button_event(Primary, false, false),
        ];
        rewrite_ctrl_click_as_secondary(&mut events, &mut converted);
        assert_eq!(button_of(&events[0]), (Primary, true, false));
        assert_eq!(button_of(&events[1]), (Primary, false, false));
        assert!(!converted);
    }

    #[test]
    fn bracketed_paste_wraps_and_sanitizes() {
        let bytes = encode_paste("a\nb\x07", TermMode::BRACKETED_PASTE);
        assert_eq!(bytes, b"\x1b[200~a\rb\x1b[201~".to_vec());
    }
}
