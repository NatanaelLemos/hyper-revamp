use alacritty_terminal::event::WindowSize;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use egui::{CursorIcon, PointerButton, Rect, Sense, Ui, Vec2};
use hyper_term::Session;

use crate::fonts::CellMetrics;
use crate::grid_paint::{GridPainter, GridSnapshot};
use crate::mouse::{self, MouseButton};
use crate::theme::TermTheme;

/// Behavior toggles from the resolved profile config.
#[derive(Clone, Debug, Default)]
pub struct TermBehavior {
    pub copy_on_select: bool,
    pub quick_edit: bool,
    pub cursor_blink: bool,
    /// webLinksActivationKey: "" | "ctrl" | "alt" | "meta" | "shift".
    pub web_links_activation: String,
}

/// Per-pane view state that isn't part of the terminal itself.
#[derive(Default)]
pub struct TermViewState {
    pub last_grid: (u16, u16),
    /// Fractional wheel scroll accumulator.
    scroll_accum: f32,
    /// Selection snapshot taken on selection change — what ⌘C copies, even
    /// if a TUI app has since rewritten the cells (Hyper's clipboard fix).
    pub selection_snapshot: Option<String>,
    /// Last cell reported to the app for mouse-motion coalescing.
    last_report_cell: Option<(u16, u16)>,
    /// Buttons this pane has reported as pressed and not yet released, so the
    /// release is reported by the same pane and motion carries the right code.
    buttons_down: Vec<MouseButton>,
    dragging_selection: bool,
}

/// The reportable button for an egui pointer button (extra buttons are not
/// part of the X10/SGR protocols).
fn report_button(button: PointerButton) -> Option<MouseButton> {
    match button {
        PointerButton::Primary => Some(MouseButton::Left),
        PointerButton::Middle => Some(MouseButton::Middle),
        PointerButton::Secondary => Some(MouseButton::Right),
        _ => None,
    }
}

pub struct TermViewOutput {
    pub response: egui::Response,
    pub mode: TermMode,
    /// Text copied by copyOnSelect/quickEdit this frame (app puts on clipboard).
    pub copied: Option<String>,
    /// quickEdit right-click with no selection requests a paste.
    pub wants_paste: bool,
    /// A link was activated (already allowlist-checked by the app).
    pub open_url: Option<String>,
}

/// Lay out, resize, scroll, select, and paint one terminal pane.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    session: &Session,
    state: &mut TermViewState,
    painter: &mut GridPainter,
    metrics: &CellMetrics,
    theme: &TermTheme,
    padding: (f32, f32, f32, f32),
    focused: bool,
    behavior: &TermBehavior,
) -> TermViewOutput {
    let response = ui.interact(rect, ui.id().with(session.id.0), Sense::click_and_drag());

    let (top, right, bottom, left) = padding;
    let inner = Rect::from_min_max(
        rect.min + Vec2::new(left, top),
        rect.max - Vec2::new(right, bottom),
    );

    // Grid size from available space; resize PTY+term when it changes.
    let cols = ((inner.width() / metrics.width).floor() as u16).max(2);
    let rows = ((inner.height() / metrics.height).floor() as u16).max(2);
    if (cols, rows) != state.last_grid {
        state.last_grid = (cols, rows);
        session.resize(WindowSize {
            num_lines: rows,
            num_cols: cols,
            cell_width: metrics.width as u16,
            cell_height: metrics.height as u16,
        });
    }

    let mut copied = None;
    let mut wants_paste = false;
    let mut open_url = None;
    let mut osc8_link: Option<(String, usize, usize)> = None; // uri, row, col
    let mut to_pty: Vec<Vec<u8>> = Vec::new();

    let snap = {
        let mut term = session.term.lock();
        let mode = *term.mode();
        let shift = ui.input(|i| i.modifiers.shift);
        let report_mouse = mode.intersects(TermMode::MOUSE_MODE) && !shift;

        // Wheel scrolling / wheel reporting.
        if response.hovered() {
            let wheel = ui.input(|i| i.smooth_scroll_delta.y);
            if wheel != 0.0 {
                state.scroll_accum += wheel / metrics.height;
                let lines = state.scroll_accum.trunc() as i32;
                state.scroll_accum -= lines as f32;
                if lines != 0 {
                    if report_mouse {
                        if let Some(pos) = response.hover_pos() {
                            let cell = mouse::report_cell(
                                pos,
                                inner,
                                metrics,
                                cols as usize,
                                rows as usize,
                            );
                            let button = if lines > 0 {
                                MouseButton::WheelUp
                            } else {
                                MouseButton::WheelDown
                            };
                            let mods = ui.input(|i| i.modifiers);
                            for _ in 0..lines.unsigned_abs() {
                                to_pty.push(mouse::encode_mouse(
                                    mode, button, true, false, mods, cell,
                                ));
                            }
                        }
                    } else if mode.contains(TermMode::ALT_SCREEN) {
                        // Alt screen without mouse reporting: arrow keys.
                        // SS3 form only in application cursor mode — an app
                        // that never set DECCKM may not recognize `ESC O A`
                        // and the wheel would do nothing.
                        let seq: &[u8] = match (mode.contains(TermMode::APP_CURSOR), lines > 0) {
                            (true, true) => b"\x1bOA",
                            (true, false) => b"\x1bOB",
                            (false, true) => b"\x1b[A",
                            (false, false) => b"\x1b[B",
                        };
                        let mut bytes = Vec::new();
                        for _ in 0..lines.unsigned_abs() {
                            bytes.extend_from_slice(seq);
                        }
                        to_pty.push(bytes);
                    } else {
                        term.scroll_display(Scroll::Delta(lines));
                    }
                }
            }
        }

        let display_offset = term.grid().display_offset();
        let pointer_pos = response.interact_pointer_pos();

        if report_mouse {
            // Application handles the mouse: forward events, no selection.
            //
            // Press/release come from raw pointer events rather than egui's
            // click/drag helpers: with `Sense::click_and_drag` a stationary
            // click never becomes a drag, so `drag_started_by` never fires,
            // `clicked_by` fires once at button-*up*, and `drag_stopped_by`
            // never fires at all — apps saw a press with no matching release
            // and got stuck in drag state.
            let events = ui.input(|i| i.events.clone());
            for event in &events {
                let egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                } = event
                else {
                    continue;
                };
                let Some(btn) = report_button(*button) else {
                    continue;
                };
                if *pressed {
                    if !rect.contains(*pos) {
                        continue;
                    }
                    state.buttons_down.push(btn);
                } else if let Some(index) = state.buttons_down.iter().position(|b| *b == btn) {
                    // Release is reported wherever the pointer ended up, but
                    // only by the pane that saw the press.
                    state.buttons_down.remove(index);
                } else {
                    continue;
                }
                let cell = mouse::report_cell(*pos, inner, metrics, cols as usize, rows as usize);
                to_pty.push(mouse::encode_mouse(mode, btn, *pressed, false, *modifiers, cell));
                state.last_report_cell = Some(cell);
            }

            // Motion: reported while a button is held (1002) or unconditionally
            // in any-event mode (1003), carrying whichever button is actually
            // down rather than always Left.
            let mods = ui.input(|i| i.modifiers);
            let held = state.buttons_down.last().copied();
            if let Some(pos) = ui.input(|i| i.pointer.latest_pos()).or(pointer_pos) {
                let over_pane = rect.contains(pos);
                if (over_pane || held.is_some())
                    && mouse::should_report(mode, true, held.is_some())
                {
                    let cell =
                        mouse::report_cell(pos, inner, metrics, cols as usize, rows as usize);
                    if state.last_report_cell != Some(cell) {
                        // No button down ⇒ button code 3 ("released"), which is
                        // what xterm reports for bare motion in 1003.
                        let button = held.unwrap_or(MouseButton::None);
                        to_pty.push(mouse::encode_mouse(mode, button, true, true, mods, cell));
                        state.last_report_cell = Some(cell);
                    }
                }
            }
        } else {
            // Not reporting (no mouse mode, or shift held to force selection):
            // drop any button we were tracking, so a button released while
            // shift is down can't leave this pane reporting phantom drags.
            state.buttons_down.clear();
            state.last_report_cell = None;

            // Local selection handling.
            if let Some(pos) = pointer_pos {
                let (point, side) = mouse::grid_point(
                    pos,
                    inner,
                    metrics,
                    display_offset,
                    cols as usize,
                    rows as usize,
                );

                if response.drag_started_by(PointerButton::Primary)
                    || response.clicked_by(PointerButton::Primary)
                {
                    let alt = ui.input(|i| i.modifiers.alt);
                    let clicks = ui.input(|i| {
                        i.pointer
                            .button_double_clicked(PointerButton::Primary)
                            .then_some(2)
                            .or_else(|| {
                                i.pointer
                                    .button_triple_clicked(PointerButton::Primary)
                                    .then_some(3)
                            })
                            .unwrap_or(1)
                    });
                    let ty = match (clicks, alt) {
                        (2, _) => SelectionType::Semantic,
                        (3, _) => SelectionType::Lines,
                        (_, true) => SelectionType::Block,
                        _ => SelectionType::Simple,
                    };
                    term.selection = Some(Selection::new(ty, point, side));
                    state.dragging_selection = clicks == 1;
                    if clicks > 1 {
                        if let Some(text) = term.selection_to_string() {
                            if !text.is_empty() {
                                state.selection_snapshot = Some(text.clone());
                                if behavior.copy_on_select {
                                    copied = Some(text);
                                }
                            }
                        }
                    }
                }

                if state.dragging_selection && response.dragged_by(PointerButton::Primary) {
                    if let Some(selection) = &mut term.selection {
                        selection.update(point, side);
                    }
                }

                if response.drag_stopped_by(PointerButton::Primary) {
                    state.dragging_selection = false;
                    match term.selection_to_string() {
                        Some(text) if !text.is_empty() => {
                            state.selection_snapshot = Some(text.clone());
                            if behavior.copy_on_select {
                                copied = Some(text);
                            }
                        }
                        _ => {
                            // Plain click: clear selection.
                            term.selection = None;
                            state.selection_snapshot = None;
                        }
                    }
                }
            }

            // quickEdit: right-click copies the selection, else pastes.
            if behavior.quick_edit && response.secondary_clicked() {
                let has_selection = term
                    .selection_to_string()
                    .map_or(false, |s| !s.is_empty());
                if has_selection {
                    if let Some(text) = state.selection_snapshot.clone().or_else(|| {
                        term.selection_to_string()
                    }) {
                        copied = Some(text);
                    }
                    term.selection = None;
                    state.selection_snapshot = None;
                } else {
                    wants_paste = true;
                }
            }

            if response.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::Text);
            }
        }

        // OSC 8 hyperlink under the pointer (explicit app-emitted links).
        if let Some(pos) = response.hover_pos() {
            if inner.contains(pos) {
                let (point, _) = mouse::grid_point(
                    pos,
                    inner,
                    metrics,
                    term.grid().display_offset(),
                    cols as usize,
                    rows as usize,
                );
                let row = (point.line.0 + term.grid().display_offset() as i32).max(0) as usize;
                let col = point.column.0;
                if let Some(link) = term.grid()[point].hyperlink() {
                    osc8_link = Some((link.uri().to_string(), row, col));
                }
            }
        }

        GridSnapshot::capture(&term, theme)
    };

    for bytes in to_pty {
        session.write(bytes);
    }

    // Cursor blink: 600ms phase, repaint scheduled at next flip.
    let cursor_visible = if behavior.cursor_blink && focused {
        let time = ui.input(|i| i.time);
        let phase = (time / 0.6) as u64;
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(
                (600 - ((time * 1000.0) as u64 % 600)).max(16),
            ));
        phase % 2 == 0
    } else {
        true
    };

    let mode = snap.mode;
    painter.paint(ui, inner, &snap, metrics, theme, focused, cursor_visible);

    // Link hover/click (painted after the grid so the underline shows):
    // OSC 8 first, then regex on the hovered row.
    let mods = ui.input(|i| i.modifiers);
    let activation_ok = crate::links::activation_matches(&behavior.web_links_activation, mods);
    if activation_ok && !snap.mode.intersects(TermMode::MOUSE_MODE) {
        let hovered_link = if let Some((uri, row, col)) = osc8_link {
            Some(crate::links::HoveredLink {
                uri,
                row,
                col_start: col,
                col_end: col,
            })
        } else if let Some(pos) = response.hover_pos().filter(|p| inner.contains(*p)) {
            let col = (((pos.x - inner.min.x) / metrics.width) as usize)
                .min(snap.cols.saturating_sub(1));
            let row = (((pos.y - inner.min.y) / metrics.height) as usize)
                .min(snap.lines.saturating_sub(1));
            crate::links::link_at(&snap, row, col)
        } else {
            None
        };

        if let Some(link) = hovered_link {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            let y = inner.min.y + (link.row + 1) as f32 * metrics.height - 1.0;
            ui.painter().line_segment(
                [
                    egui::Pos2::new(inner.min.x + link.col_start as f32 * metrics.width, y),
                    egui::Pos2::new(
                        inner.min.x + (link.col_end + 1) as f32 * metrics.width,
                        y,
                    ),
                ],
                egui::Stroke::new(1.0, theme.foreground),
            );
            if response.clicked() && !response.dragged() {
                open_url = Some(link.uri);
            }
        }
    }

    TermViewOutput {
        response,
        mode,
        copied,
        wants_paste,
        open_url,
    }
}

#[cfg(test)]
mod tests {
    /// Framework probe: the pane pattern (ui.interact response +
    /// response.context_menu) must open a popup on a synthetic
    /// secondary click in headless egui.
    #[test]
    fn context_menu_opens_on_secondary_click() {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(800.0, 600.0));
        let pos = egui::Pos2::new(200.0, 200.0);
        let mut menu_shown = false;

        let run_frame = |events: Vec<egui::Event>, menu_shown: &mut bool| {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| {
                let response =
                    ui.interact(screen, ui.id().with(1u64), egui::Sense::click_and_drag());
                response.context_menu(|ui| {
                    *menu_shown = true;
                    let _ = ui.button("Copy");
                });
            });
        };

        run_frame(vec![], &mut menu_shown);
        run_frame(
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
            &mut menu_shown,
        );
        run_frame(
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed: false,
                modifiers: Default::default(),
            }],
            &mut menu_shown,
        );
        run_frame(vec![], &mut menu_shown);

        assert!(menu_shown, "context menu never opened");
    }
}
