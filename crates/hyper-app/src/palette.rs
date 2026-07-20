//! Inline command palette — port of `lib/components/command-palette.tsx`.
//!
//! Typing `/` at the start of a prompt line opens an overlay; the characters
//! still reach the shell so the user sees what they typed. Enter (after
//! navigating or with an unambiguous match) sends `\x15` to wipe the typed
//! `/xyz` from the prompt line and runs the command.

use egui::{Color32, Key, Rect, Sense, Stroke, Ui, Vec2};
use hyper_config::Command;

use crate::theme::TermTheme;

pub struct PaletteItem {
    pub label: String,
    pub hint: String,
    pub command: Command,
}

#[derive(Default)]
pub struct PaletteState {
    pub open: bool,
    query: String,
    selected: usize,
    navigated: bool,
    /// Approximate column on the current shell line (0 = fresh prompt).
    line_col: usize,
}

/// What the app should do after this frame's palette input handling.
#[derive(Default)]
pub struct PaletteAction {
    /// Send `\x15` to the active session (wipe the typed `/xyz`), then run.
    pub run: Option<Command>,
}

fn items(profiles: &[String], query: &str) -> Vec<PaletteItem> {
    let q = query.to_lowercase();
    let mut out = Vec::new();
    for name in profiles {
        let label = format!("/profile {name}");
        if q.is_empty()
            || q == "/"
            || label.to_lowercase().starts_with(&q)
            || "/profile".starts_with(&q)
        {
            out.push(PaletteItem {
                label,
                hint: "Open a new tab in a specific profile".into(),
                command: Command::NewTabProfile(name.clone()),
            });
        }
    }
    out
}

impl PaletteState {
    /// Process this frame's keyboard input. Runs before terminal encoding;
    /// consumes only navigation keys — printable characters continue to the
    /// PTY so they stay visible on the prompt line.
    /// `tab_pressed`: Tab keypresses are hidden from egui by the app's
    /// raw_input_hook (they'd trigger focus traversal), so they arrive here
    /// as a flag instead of an event.
    pub fn handle_input(
        &mut self,
        ctx: &egui::Context,
        profiles: &[String],
        terminal_focused: bool,
        tab_pressed: bool,
    ) -> PaletteAction {
        let mut action = PaletteAction::default();
        if !terminal_focused {
            return action;
        }

        // Peek at text/keys without consuming (they must reach the PTY too).
        let (texts, enter, backspace, ctrl_reset) = ctx.input(|i| {
            let texts: Vec<String> = i
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) => Some(t.clone()),
                    _ => None,
                })
                .collect();
            let enter = i.key_pressed(Key::Enter);
            let backspace = i.key_pressed(Key::Backspace);
            let mods = i.modifiers;
            // Ctrl+U/C/A/L (or cmd) reset the believed line position.
            let ctrl_reset = (mods.ctrl || mods.command)
                && [Key::U, Key::C, Key::A, Key::L]
                    .iter()
                    .any(|k| i.key_pressed(*k));
            (texts, enter, backspace, ctrl_reset)
        });

        if !self.open {
            if ctrl_reset || enter {
                self.line_col = 0;
            } else if backspace {
                self.line_col = self.line_col.saturating_sub(1);
            }
            for t in &texts {
                if t == "/" && self.line_col == 0 {
                    self.open = true;
                    self.query = "/".into();
                    self.selected = 0;
                    self.navigated = false;
                }
                self.line_col += t.chars().count();
            }
            return action;
        }

        // Palette open: navigation keys are consumed, printables pass through.
        let list = items(profiles, &self.query);
        let count = list.len();

        let down = tab_pressed
            || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::ArrowDown));
        let up = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::ArrowUp));
        let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));

        if down {
            self.navigated = true;
            self.selected = if count == 0 {
                0
            } else {
                (self.selected + 1) % count
            };
        }
        if up {
            self.navigated = true;
            self.selected = self.selected.saturating_sub(1);
        }
        if esc {
            self.close();
            return action;
        }

        if enter {
            self.line_col = 0;
            if self.navigated && count > 0 {
                // Consume Enter so the shell doesn't run the typed `/xyz`.
                ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
                if let Some(item) = list.get(self.selected.min(count - 1)) {
                    action.run = Some(item.command.clone());
                }
            }
            self.close();
            return action;
        }

        if backspace {
            self.line_col = self.line_col.saturating_sub(1);
            if self.query.chars().count() <= 1 {
                self.close();
            } else {
                self.query.pop();
                self.selected = 0;
                self.navigated = false;
            }
            return action;
        }

        for t in &texts {
            for ch in t.chars() {
                self.query.push(ch);
                self.line_col += 1;
            }
            self.selected = 0;
            self.navigated = false;
        }

        action
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query = "/".into();
        self.selected = 0;
        self.navigated = false;
    }

    /// Paint the overlay at the bottom-left of the active pane. Returns a
    /// command if an item was clicked.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        pane_rect: Rect,
        profiles: &[String],
        theme: &TermTheme,
    ) -> Option<Command> {
        if !self.open {
            return None;
        }
        let list = items(profiles, &self.query);
        self.selected = self.selected.min(list.len().saturating_sub(1));

        let width = (pane_rect.width() - 32.0).min(520.0).max(200.0);
        let row_h = 26.0;
        let head_h = 30.0;
        let body_h = (list.len().max(1) as f32) * row_h;
        let panel = Rect::from_min_size(
            egui::Pos2::new(
                pane_rect.min.x + 16.0,
                pane_rect.max.y - 16.0 - head_h - body_h,
            ),
            Vec2::new(width, head_h + body_h),
        );

        let painter = ui.painter();
        painter.rect_filled(panel, 8.0, Color32::from_rgba_unmultiplied(20, 22, 28, 247));
        painter.rect_stroke(
            panel,
            8.0,
            Stroke::new(1.0, Color32::from_white_alpha(30)),
            egui::StrokeKind::Inside,
        );

        // Head: query + key hints.
        let head = Rect::from_min_size(panel.min, Vec2::new(width, head_h));
        painter.line_segment(
            [
                egui::Pos2::new(panel.min.x, head.max.y),
                egui::Pos2::new(panel.max.x, head.max.y),
            ],
            Stroke::new(1.0, Color32::from_white_alpha(20)),
        );
        painter.text(
            head.left_center() + Vec2::new(12.0, 0.0),
            egui::Align2::LEFT_CENTER,
            &self.query,
            egui::FontId::monospace(13.0),
            Color32::from_rgb(0x9c, 0xd2, 0xff),
        );
        painter.text(
            head.right_center() - Vec2::new(12.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            "↑↓ navigate · Enter select · Esc dismiss",
            egui::FontId::monospace(11.0),
            Color32::from_rgb(0x7a, 0x82, 0x8b),
        );

        let mut clicked = None;
        if list.is_empty() {
            painter.text(
                egui::Pos2::new(panel.min.x + 12.0, head.max.y + row_h / 2.0),
                egui::Align2::LEFT_CENTER,
                "No matching commands",
                egui::FontId::proportional(12.0),
                Color32::from_rgb(0x7a, 0x82, 0x8b),
            );
        }
        for (i, item) in list.iter().enumerate() {
            let row = Rect::from_min_size(
                egui::Pos2::new(panel.min.x, head.max.y + i as f32 * row_h),
                Vec2::new(width, row_h),
            );
            let response = ui.interact(row, ui.id().with(("palette", i)), Sense::click());
            if response.hovered() {
                self.selected = i;
            }
            if i == self.selected {
                ui.painter().rect_filled(
                    row.shrink2(Vec2::new(4.0, 1.0)),
                    4.0,
                    Color32::from_white_alpha(18),
                );
            }
            ui.painter().text(
                row.left_center() + Vec2::new(12.0, 0.0),
                egui::Align2::LEFT_CENTER,
                &item.label,
                egui::FontId::monospace(12.0),
                Color32::from_rgb(0xe6, 0xed, 0xf3),
            );
            ui.painter().text(
                row.right_center() - Vec2::new(12.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                &item.hint,
                egui::FontId::proportional(11.0),
                Color32::from_rgb(0x7a, 0x82, 0x8b),
            );
            if response.clicked() {
                clicked = Some(item.command.clone());
            }
        }

        // Swallow clicks under the panel.
        ui.interact(panel, ui.id().with("palette-panel"), Sense::click());

        if clicked.is_some() {
            self.close();
        }
        let _ = theme;
        clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_filter_by_prefix() {
        let profiles = vec!["default".to_string(), "work ssh".to_string()];
        assert_eq!(items(&profiles, "/").len(), 2);
        assert_eq!(items(&profiles, "/prof").len(), 2);
        assert_eq!(items(&profiles, "/profile w").len(), 1);
        assert_eq!(items(&profiles, "/zzz").len(), 0);
    }
}
