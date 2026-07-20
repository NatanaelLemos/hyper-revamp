use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::search::RegexSearch;
use egui::{Align2, Color32, Key, Rect, Sense, Stroke, Ui, Vec2};
use hyper_term::Session;

use crate::theme::TermTheme;

/// Search overlay state (one, over the active pane — ⌘F / Esc).
///
/// Options mirror the xterm.js search addon the original drove from its
/// SearchBox: case-sensitivity, whole-word, and regex, all off by default.
#[derive(Default)]
pub struct SearchState {
    pub open: bool,
    pub query: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex_mode: bool,
    focus_field: bool,
    last_match: Option<(Point, Point)>,
    no_match: bool,
}

impl SearchState {
    pub fn open(&mut self) {
        self.open = true;
        self.focus_field = true;
        self.no_match = false;
    }

    pub fn close(&mut self, session: Option<&Session>) {
        self.open = false;
        self.last_match = None;
        if let Some(session) = session {
            session.term.lock().selection = None;
        }
    }

    /// The regex actually handed to the terminal for the current query and
    /// options.
    ///
    /// alacritty applies *smart case* (case-insensitive only while the pattern
    /// is all-lowercase), but the original defaulted to `caseSensitive: false`
    /// unconditionally — searching "Error" there still found "error". An
    /// explicit `(?i)` restores that, and dropping it makes the
    /// case-sensitivity toggle mean what it says.
    fn pattern(&self) -> String {
        let body = if self.regex_mode {
            self.query.clone()
        } else {
            regex::escape(&self.query)
        };
        let body = if self.whole_word {
            format!(r"\b(?:{body})\b")
        } else {
            body
        };
        if self.case_sensitive {
            body
        } else {
            format!("(?i){body}")
        }
    }

    /// Find and highlight the next/previous match; scrolls it into view by
    /// selecting it (painted with the selection color).
    pub fn search(&mut self, session: &Session, direction: Direction) {
        if self.query.is_empty() {
            self.no_match = false;
            return;
        }
        let Ok(mut regex) = RegexSearch::new(&self.pattern()) else {
            // In regex mode the query is half-typed for most keystrokes;
            // an invalid pattern is just "no match yet", not an error.
            self.no_match = true;
            return;
        };

        let mut term = session.term.lock();
        let origin = match (&self.last_match, direction) {
            (Some((start, end)), Direction::Right) => {
                end.add(&*term, Boundary::None, 1).max(*start)
            }
            (Some((start, _)), Direction::Left) => {
                start.sub(&*term, Boundary::None, 1)
            }
            (None, _) => {
                // Start from the top of the visible area.
                Point::new(Line(-(term.grid().display_offset() as i32)), Column(0))
            }
        };

        match term.search_next(&mut regex, origin, direction, Side::Left, None) {
            Some(found) => {
                let (start, end) = (*found.start(), *found.end());
                self.last_match = Some((start, end));
                self.no_match = false;

                let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
                selection.update(end, Side::Right);
                term.selection = Some(selection);

                // Scroll the match into view.
                let display_offset = term.grid().display_offset() as i32;
                let screen_lines = term.screen_lines() as i32;
                let top = -display_offset;
                let bottom = screen_lines - 1 - display_offset;
                let line = start.line.0;
                if line < top {
                    term.scroll_display(Scroll::Delta(top - line));
                } else if line > bottom {
                    term.scroll_display(Scroll::Delta(bottom - line));
                }
            }
            None => {
                self.no_match = true;
            }
        }
    }

    /// Render the floating search panel at the top-right of `pane_rect`.
    /// Returns true while the overlay has keyboard focus.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        pane_rect: Rect,
        session: &Session,
        theme: &TermTheme,
    ) -> bool {
        if !self.open {
            return false;
        }

        let panel_size = Vec2::new(360.0, 34.0);
        let panel_rect = Rect::from_min_size(
            egui::Pos2::new(pane_rect.max.x - panel_size.x - 12.0, pane_rect.min.y + 8.0),
            panel_size,
        );

        let painter = ui.painter();
        painter.rect_filled(panel_rect, 6.0, theme.background.gamma_multiply(0.92).to_opaque());
        painter.rect_stroke(
            panel_rect,
            6.0,
            Stroke::new(1.0, theme.border),
            egui::StrokeKind::Inside,
        );

        let mut focused = false;
        let inner = panel_rect.shrink2(Vec2::new(8.0, 5.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        child.horizontal_centered(|ui| {
            // field + 3 option toggles + prev/next/close
            let field_width = inner.width() - 6.0 * 24.0 - 12.0;
            let field = egui::TextEdit::singleline(&mut self.query)
                .desired_width(field_width)
                .hint_text("Search")
                .text_color(if self.no_match {
                    Color32::from_rgb(0xff, 0x6b, 0x6b)
                } else {
                    theme.foreground
                })
                .show(ui);

            if self.focus_field {
                field.response.request_focus();
                self.focus_field = false;
            }
            focused = field.response.has_focus();

            if field.response.changed() {
                self.last_match = None;
                self.search(session, Direction::Right);
            }

            let shift = ui.input(|i| i.modifiers.shift);
            if field.response.lost_focus()
                && ui.input(|i| i.key_pressed(Key::Enter))
            {
                self.search(
                    session,
                    if shift {
                        Direction::Left
                    } else {
                        Direction::Right
                    },
                );
                field.response.request_focus();
                focused = true;
            }

            // Option toggles, as in the original's SearchBox.
            let mut options_changed = false;
            for (label, tooltip, value) in [
                ("Aa", "Match case", &mut self.case_sensitive),
                ("|ab|", "Whole word", &mut self.whole_word),
                (".*", "Regular expression", &mut self.regex_mode),
            ] {
                let btn = ui.add_sized(
                    Vec2::splat(22.0),
                    egui::Button::selectable(*value, label),
                );
                if btn.on_hover_text(tooltip).clicked() {
                    *value = !*value;
                    options_changed = true;
                }
            }
            if options_changed {
                self.last_match = None;
                self.search(session, Direction::Right);
            }

            for (label, direction) in [("▲", Direction::Left), ("▼", Direction::Right)] {
                let btn = ui.add_sized(
                    Vec2::splat(22.0),
                    egui::Button::new(label).frame(false),
                );
                if btn.clicked() {
                    self.search(session, direction);
                }
            }
            // U+00D7: in the fonts' Latin-1 coverage, unlike ✕ (U+2715).
            let close = ui.add_sized(Vec2::splat(22.0), egui::Button::new("×").frame(false));
            if close.clicked() {
                self.close(Some(session));
            }
        });

        // Swallow clicks under the panel.
        ui.interact(panel_rect, ui.id().with("search-panel"), Sense::click());

        let _ = Align2::CENTER_CENTER;
        focused
    }
}
