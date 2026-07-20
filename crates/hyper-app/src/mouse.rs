use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::term::TermMode;
use egui::{Modifiers, Pos2, Rect};

use crate::fonts::CellMetrics;

/// Map a pixel position inside the padded terminal rect to a grid point
/// (grid coords: visible top line = -display_offset) plus the cell side.
pub fn grid_point(
    pos: Pos2,
    inner: Rect,
    metrics: &CellMetrics,
    display_offset: usize,
    cols: usize,
    lines: usize,
) -> (Point, Side) {
    let rel_x = (pos.x - inner.min.x).max(0.0);
    let rel_y = (pos.y - inner.min.y).max(0.0);
    let col = ((rel_x / metrics.width) as usize).min(cols.saturating_sub(1));
    let row = ((rel_y / metrics.height) as usize).min(lines.saturating_sub(1));
    let line = Line(row as i32 - display_offset as i32);
    let side = if rel_x % metrics.width > metrics.width / 2.0 {
        Side::Right
    } else {
        Side::Left
    };
    (Point::new(line, Column(col)), side)
}

/// Terminal-visible position (1-based) for mouse reporting.
pub fn report_cell(
    pos: Pos2,
    inner: Rect,
    metrics: &CellMetrics,
    cols: usize,
    lines: usize,
) -> (u16, u16) {
    let col = (((pos.x - inner.min.x).max(0.0) / metrics.width) as u16)
        .min(cols.saturating_sub(1) as u16)
        + 1;
    let row = (((pos.y - inner.min.y).max(0.0) / metrics.height) as u16)
        .min(lines.saturating_sub(1) as u16)
        + 1;
    (col, row)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    /// No button — motion reporting with nothing held (button code 3).
    None,
}

impl MouseButton {
    fn code(self) -> u8 {
        match self {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::WheelUp => 64,
            MouseButton::WheelDown => 65,
            MouseButton::None => 3,
        }
    }
}

fn mod_bits(mods: Modifiers) -> u8 {
    (mods.shift as u8) * 4 + (mods.alt as u8) * 8 + (mods.ctrl as u8) * 16
}

/// Encode a mouse event per the terminal's active protocol (SGR preferred,
/// legacy X10 otherwise). `motion` marks drag/move events (+32).
pub fn encode_mouse(
    mode: TermMode,
    button: MouseButton,
    pressed: bool,
    motion: bool,
    mods: Modifiers,
    cell: (u16, u16),
) -> Vec<u8> {
    let mut code = button.code() + mod_bits(mods);
    if motion {
        code += 32;
    }

    let (col, row) = cell;
    if mode.contains(TermMode::SGR_MOUSE) {
        let suffix = if pressed { 'M' } else { 'm' };
        format!("\x1b[<{code};{col};{row}{suffix}").into_bytes()
    } else {
        // Legacy X10: release is reported as button 3.
        let code = if pressed || motion { code } else { 3 + mod_bits(mods) };
        let mut bytes = vec![0x1b, b'[', b'M', 32 + code];
        if mode.contains(TermMode::UTF8_MOUSE) {
            // DECSET 1005: the same +32 offset, but each coordinate is a UTF-8
            // code point rather than a raw byte, which lifts the 223 ceiling.
            //
            // xterm.js dropped 1005 and so the Electron build never spoke it —
            // but alacritty_terminal sets the mode when an app asks for it
            // either way, and answering a UTF-8 request with X10 bytes puts
            // mojibake in the app's input stream past column 95. Honoring the
            // mode we advertise is the only self-consistent option.
            push_utf8(&mut bytes, col);
            push_utf8(&mut bytes, row);
        } else {
            // Positions are byte+32, clamped to 223 to stay in a single byte.
            bytes.push((col.min(223) as u8) + 32);
            bytes.push((row.min(223) as u8) + 32);
        }
        bytes
    }
}

/// Push a 1005 coordinate: `value + 32` as UTF-8.
///
/// 2015 is the largest coordinate that still fits xterm's two-byte limit for
/// this mode (2015 + 32 = 2047, the last two-byte code point).
fn push_utf8(bytes: &mut Vec<u8>, value: u16) {
    let point = value.min(2015) + 32;
    match char::from_u32(point as u32) {
        Some(ch) => {
            let mut buf = [0u8; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        }
        None => bytes.push(point as u8),
    }
}

/// Whether this event class should be reported to the application at all.
pub fn should_report(mode: TermMode, motion: bool, button_held: bool) -> bool {
    if !mode.intersects(TermMode::MOUSE_MODE) {
        return false;
    }
    if !motion {
        return true; // presses/releases reported in all mouse modes
    }
    if mode.contains(TermMode::MOUSE_MOTION) {
        return true; // report all motion
    }
    mode.contains(TermMode::MOUSE_DRAG) && button_held
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sgr() -> TermMode {
        TermMode::SGR_MOUSE | TermMode::MOUSE_REPORT_CLICK
    }

    /// Under DECSET 1005 coordinates are UTF-8 code points, so a click past
    /// column 191 still reports its true position instead of saturating at the
    /// 223 byte ceiling.
    #[test]
    fn utf8_mouse_encodes_wide_coordinates() {
        let mode = TermMode::UTF8_MOUSE | TermMode::MOUSE_REPORT_CLICK;
        let mods = Modifiers::default();

        // Under 96 the encoding is byte-identical to X10 (ASCII range).
        let legacy = encode_mouse(
            TermMode::MOUSE_REPORT_CLICK,
            MouseButton::Left,
            true,
            false,
            mods,
            (10, 5),
        );
        let utf8 = encode_mouse(mode, MouseButton::Left, true, false, mods, (10, 5));
        assert_eq!(utf8, legacy);
        assert_eq!(utf8, vec![0x1b, b'[', b'M', 32, 42, 37]);

        // Past the single-byte range the coordinate becomes two UTF-8 bytes
        // rather than being clamped.
        let utf8 = encode_mouse(mode, MouseButton::Left, true, false, mods, (300, 250));
        let mut want = vec![0x1b, b'[', b'M', 32];
        let mut buf = [0u8; 4];
        for coord in [300u32, 250] {
            let ch = char::from_u32(coord + 32).unwrap();
            want.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        }
        assert_eq!(utf8, want);
        // Two bytes per coordinate above U+007F, so eight in total.
        assert_eq!(utf8.len(), 8);

        // X10 in the same spot saturates, which is what the mode exists to fix.
        let legacy = encode_mouse(
            TermMode::MOUSE_REPORT_CLICK,
            MouseButton::Left,
            true,
            false,
            mods,
            (300, 250),
        );
        assert_eq!(legacy, vec![0x1b, b'[', b'M', 32, 255, 255]);
    }

    /// SGR wins when an app has enabled both.
    #[test]
    fn sgr_takes_precedence_over_utf8_mouse() {
        let mode = TermMode::SGR_MOUSE | TermMode::UTF8_MOUSE | TermMode::MOUSE_REPORT_CLICK;
        let encoded = encode_mouse(
            mode,
            MouseButton::Left,
            true,
            false,
            Modifiers::default(),
            (300, 250),
        );
        assert_eq!(text(encoded), "\x1b[<0;300;250M");
    }

    fn text(bytes: Vec<u8>) -> String {
        String::from_utf8(bytes).unwrap()
    }

    /// Every click is a press *and* a release; a press with no release leaves
    /// tmux/vim stuck in drag state.
    #[test]
    fn sgr_click_reports_press_and_release() {
        let m = Modifiers::default();
        let press = encode_mouse(sgr(), MouseButton::Left, true, false, m, (5, 9));
        let release = encode_mouse(sgr(), MouseButton::Left, false, false, m, (5, 9));
        assert_eq!(text(press), "\x1b[<0;5;9M");
        assert_eq!(text(release), "\x1b[<0;5;9m");
    }

    #[test]
    fn sgr_buttons_and_modifiers() {
        let m = Modifiers::default();
        assert_eq!(
            text(encode_mouse(sgr(), MouseButton::Middle, true, false, m, (1, 1))),
            "\x1b[<1;1;1M"
        );
        assert_eq!(
            text(encode_mouse(sgr(), MouseButton::Right, true, false, m, (1, 1))),
            "\x1b[<2;1;1M"
        );
        let ctrl = Modifiers {
            ctrl: true,
            ..Default::default()
        };
        // ctrl = +16
        assert_eq!(
            text(encode_mouse(sgr(), MouseButton::Left, true, false, ctrl, (2, 3))),
            "\x1b[<16;2;3M"
        );
    }

    /// Drag motion carries the held button (+32), not always Left.
    #[test]
    fn motion_encodes_the_held_button() {
        let m = Modifiers::default();
        assert_eq!(
            text(encode_mouse(sgr(), MouseButton::Left, true, true, m, (4, 4))),
            "\x1b[<32;4;4M"
        );
        assert_eq!(
            text(encode_mouse(sgr(), MouseButton::Right, true, true, m, (4, 4))),
            "\x1b[<34;4;4M"
        );
        // Bare motion in any-event mode: button 3 + 32.
        assert_eq!(
            text(encode_mouse(sgr(), MouseButton::None, true, true, m, (4, 4))),
            "\x1b[<35;4;4M"
        );
    }

    #[test]
    fn legacy_encoding_offsets_and_clamps() {
        let mode = TermMode::MOUSE_REPORT_CLICK;
        let m = Modifiers::default();
        // Press: 32 + button, coords + 32.
        assert_eq!(
            encode_mouse(mode, MouseButton::Left, true, false, m, (1, 1)),
            vec![0x1b, b'[', b'M', 32, 33, 33]
        );
        // Release is button 3 regardless of which button.
        assert_eq!(
            encode_mouse(mode, MouseButton::Right, false, false, m, (1, 1)),
            vec![0x1b, b'[', b'M', 35, 33, 33]
        );
        // Coordinates clamp to 223 to stay single-byte.
        let far = encode_mouse(mode, MouseButton::Left, true, false, m, (500, 500));
        assert_eq!(far[4], 255);
        assert_eq!(far[5], 255);
    }

    #[test]
    fn motion_reporting_respects_the_mode() {
        // No mouse mode at all.
        assert!(!should_report(TermMode::empty(), false, false));
        // Click mode: presses yes, motion no.
        assert!(should_report(TermMode::MOUSE_REPORT_CLICK, false, false));
        assert!(!should_report(TermMode::MOUSE_REPORT_CLICK, true, true));
        // Drag mode (1002): motion only while a button is held.
        assert!(should_report(TermMode::MOUSE_DRAG, true, true));
        assert!(!should_report(TermMode::MOUSE_DRAG, true, false));
        // Any-event mode (1003): motion always, even with no button.
        assert!(should_report(TermMode::MOUSE_MOTION, true, false));
    }
}
