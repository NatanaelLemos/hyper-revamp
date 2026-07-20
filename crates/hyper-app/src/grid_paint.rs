use std::num::NonZeroUsize;
use std::sync::Arc;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Point;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::CursorShape;
use egui::text::LayoutJob;
use egui::{Color32, FontId, Galley, Pos2, Rect, Stroke, Vec2};
use lru::LruCache;

use crate::fonts::CellMetrics;
use crate::theme::TermTheme;

#[derive(Clone, Copy, PartialEq)]
pub struct SnapCell {
    pub ch: char,
    pub fg: Color32,
    /// None means "default background" (window fill shows through).
    pub bg: Option<Color32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikeout: bool,
    pub hidden: bool,
    pub wide: bool,
    pub spacer: bool,
    pub selected: bool,
}

impl SnapCell {
    fn blank() -> Self {
        SnapCell {
            ch: ' ',
            fg: Color32::TRANSPARENT,
            bg: None,
            bold: false,
            italic: false,
            underline: false,
            strikeout: false,
            hidden: false,
            wide: false,
            spacer: false,
            selected: false,
        }
    }
}

pub struct CursorSnap {
    pub row: usize,
    pub col: usize,
    pub shape: CursorShape,
    pub wide: bool,
}

/// A copy of everything needed to paint one frame, extracted under a brief
/// terminal lock so galley building never blocks the PTY reader.
pub struct GridSnapshot {
    pub cols: usize,
    pub lines: usize,
    pub cells: Vec<SnapCell>,
    pub cursor: Option<CursorSnap>,
    pub display_offset: usize,
    pub history: usize,
    pub mode: TermMode,
}

impl GridSnapshot {
    pub fn capture<T: EventListener>(term: &Term<T>, theme: &TermTheme) -> GridSnapshot {
        let content = term.renderable_content();
        let cols = term.columns();
        let lines = term.screen_lines();
        let display_offset = content.display_offset;
        let history = term.grid().history_size();
        let mode = content.mode;

        let mut cells = vec![SnapCell::blank(); cols * lines];
        let selection = content.selection;

        for indexed in content.display_iter {
            let point: Point = indexed.point;
            let row = (point.line.0 + display_offset as i32) as usize;
            let col = point.column.0;
            if row >= lines || col >= cols {
                continue;
            }
            let cell = &indexed.cell;
            let flags = cell.flags;

            let mut fg = theme.resolve_fg(cell.fg, content.colors, flags.contains(Flags::BOLD));
            let mut bg = match cell.bg {
                alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Background,
                ) => None,
                other => Some(theme.resolve(other, content.colors)),
            };

            if flags.contains(Flags::INVERSE) {
                let old_fg = fg;
                fg = bg.unwrap_or(theme.background);
                bg = Some(old_fg);
            }
            if flags.contains(Flags::DIM) {
                fg = Color32::from_rgba_unmultiplied(fg.r(), fg.g(), fg.b(), 170);
            }

            let selected = selection.map_or(false, |range| range.contains(point));

            cells[row * cols + col] = SnapCell {
                ch: cell.c,
                fg,
                bg,
                bold: flags.contains(Flags::BOLD),
                italic: flags.contains(Flags::ITALIC),
                underline: flags.intersects(
                    Flags::UNDERLINE
                        | Flags::DOUBLE_UNDERLINE
                        | Flags::UNDERCURL
                        | Flags::DOTTED_UNDERLINE
                        | Flags::DASHED_UNDERLINE,
                ),
                strikeout: flags.contains(Flags::STRIKEOUT),
                hidden: flags.contains(Flags::HIDDEN),
                wide: flags.contains(Flags::WIDE_CHAR),
                spacer: flags.contains(Flags::WIDE_CHAR_SPACER),
                selected,
            };
        }

        let cursor = {
            let point = content.cursor.point;
            let row = point.line.0 + display_offset as i32;
            if content.cursor.shape == CursorShape::Hidden
                || row < 0
                || row as usize >= lines
            {
                None
            } else {
                let wide = cells
                    .get(row as usize * cols + point.column.0)
                    .map_or(false, |c| c.wide);
                Some(CursorSnap {
                    row: row as usize,
                    col: point.column.0,
                    shape: content.cursor.shape,
                    wide,
                })
            }
        };

        GridSnapshot {
            cols,
            lines,
            cells,
            cursor,
            display_offset,
            history,
            mode,
        }
    }

    pub fn cell(&self, row: usize, col: usize) -> &SnapCell {
        &self.cells[row * self.cols + col]
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct GalleyKey {
    text: String,
    face: u8,
    /// Font size in tenths of a point (f32 isn't hashable).
    size_decipoints: u32,
}

/// Paints grid snapshots; caches galleys since TUI frames repeat text heavily.
pub struct GridPainter {
    cache: LruCache<GalleyKey, Arc<Galley>>,
}

impl Default for GridPainter {
    fn default() -> Self {
        GridPainter {
            cache: LruCache::new(NonZeroUsize::new(8192).unwrap()),
        }
    }
}

impl GridPainter {
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    fn galley(
        &mut self,
        ui: &egui::Ui,
        text: &str,
        font_id: &FontId,
        face: u8,
    ) -> Arc<Galley> {
        let key = GalleyKey {
            text: text.to_owned(),
            face,
            size_decipoints: (font_id.size * 10.0) as u32,
        };
        if let Some(galley) = self.cache.get(&key) {
            return galley.clone();
        }
        let job = LayoutJob::simple_singleline(
            text.to_owned(),
            font_id.clone(),
            Color32::PLACEHOLDER,
        );
        let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));
        self.cache.put(key, galley.clone());
        galley
    }

    /// Paint the snapshot into `rect` (already excludes padding).
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        ui: &egui::Ui,
        rect: Rect,
        snap: &GridSnapshot,
        metrics: &CellMetrics,
        theme: &TermTheme,
        focused: bool,
        cursor_visible: bool,
    ) {
        let painter = ui.painter_at(rect);
        let origin = rect.min;
        let (cw, ch) = (metrics.width, metrics.height);
        let per_cell = theme.letter_spacing != 0.0;

        // Pass 1: backgrounds, merged into horizontal runs.
        for row in 0..snap.lines {
            let y = origin.y + row as f32 * ch;
            let mut col = 0;
            while col < snap.cols {
                let cell = snap.cell(row, col);
                let bg = cell.bg;
                if bg.is_none() {
                    col += 1;
                    continue;
                }
                let start = col;
                while col < snap.cols && snap.cell(row, col).bg == bg {
                    col += 1;
                }
                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(origin.x + start as f32 * cw, y),
                        Vec2::new((col - start) as f32 * cw, ch),
                    ),
                    0.0,
                    bg.unwrap(),
                );
            }
        }

        // Pass 2: selection overlay.
        for row in 0..snap.lines {
            let y = origin.y + row as f32 * ch;
            let mut col = 0;
            while col < snap.cols {
                if !snap.cell(row, col).selected {
                    col += 1;
                    continue;
                }
                let start = col;
                while col < snap.cols && snap.cell(row, col).selected {
                    col += 1;
                }
                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(origin.x + start as f32 * cw, y),
                        Vec2::new((col - start) as f32 * cw, ch),
                    ),
                    0.0,
                    theme.selection,
                );
            }
        }

        // Pass 3: text as style-merged runs.
        let mut run = String::with_capacity(snap.cols);
        for row in 0..snap.lines {
            let y = origin.y + row as f32 * ch + metrics.glyph_offset;
            let mut col = 0;
            while col < snap.cols {
                let cell = snap.cell(row, col);
                if cell.spacer || cell.hidden || (cell.ch == ' ' && !cell.underline && !cell.strikeout) {
                    col += 1;
                    continue;
                }

                let style = (cell.fg, cell.bold, cell.italic, cell.underline, cell.strikeout);
                let start = col;
                run.clear();

                if cell.wide || !cell.ch.is_ascii() || per_cell {
                    // Non-ASCII / wide: single-cell placement at exact column.
                    run.push(cell.ch);
                    col += if cell.wide { 2 } else { 1 };
                } else {
                    while col < snap.cols {
                        let c = snap.cell(row, col);
                        if c.spacer
                            || c.hidden
                            || c.wide
                            || !c.ch.is_ascii()
                            || (c.fg, c.bold, c.italic, c.underline, c.strikeout) != style
                        {
                            break;
                        }
                        run.push(c.ch);
                        col += 1;
                    }
                }

                let (font_id, face) = match (cell.bold, cell.italic) {
                    (false, false) => (&metrics.regular, 0u8),
                    (true, false) => (&metrics.bold, 1),
                    (false, true) => (&metrics.italic, 2),
                    (true, true) => (&metrics.bold_italic, 3),
                };
                let trimmed = run.trim_end();
                if !trimmed.is_empty() {
                    let galley = self.galley(ui, trimmed, font_id, face);
                    painter.galley(
                        Pos2::new(origin.x + start as f32 * cw, y),
                        galley,
                        cell.fg,
                    );
                }

                let run_px = (col - start) as f32 * cw;
                if cell.underline {
                    let uy = origin.y + (row + 1) as f32 * ch - 1.5;
                    painter.line_segment(
                        [
                            Pos2::new(origin.x + start as f32 * cw, uy),
                            Pos2::new(origin.x + start as f32 * cw + run_px, uy),
                        ],
                        Stroke::new(1.0, cell.fg),
                    );
                }
                if cell.strikeout {
                    let sy = origin.y + row as f32 * ch + ch * 0.5;
                    painter.line_segment(
                        [
                            Pos2::new(origin.x + start as f32 * cw, sy),
                            Pos2::new(origin.x + start as f32 * cw + run_px, sy),
                        ],
                        Stroke::new(1.0, cell.fg),
                    );
                }
            }
        }

        // Pass 4: cursor.
        if let Some(cursor) = &snap.cursor {
            let width = if cursor.wide { 2.0 * cw } else { cw };
            let cell_rect = Rect::from_min_size(
                Pos2::new(
                    origin.x + cursor.col as f32 * cw,
                    origin.y + cursor.row as f32 * ch,
                ),
                Vec2::new(width, ch),
            );
            if !focused {
                painter.rect_stroke(
                    cell_rect,
                    0.0,
                    Stroke::new(1.0, theme.cursor),
                    egui::StrokeKind::Inside,
                );
            } else if cursor_visible {
                match cursor.shape {
                    CursorShape::Block | CursorShape::HollowBlock => {
                        painter.rect_filled(cell_rect, 0.0, theme.cursor);
                        // Repaint the covered glyph in the accent color.
                        let cell = snap.cell(cursor.row, cursor.col);
                        if cell.ch != ' ' && !cell.hidden {
                            let (font_id, face) = match (cell.bold, cell.italic) {
                                (false, false) => (&metrics.regular, 0u8),
                                (true, false) => (&metrics.bold, 1),
                                (false, true) => (&metrics.italic, 2),
                                (true, true) => (&metrics.bold_italic, 3),
                            };
                            let mut buf = [0u8; 4];
                            let text = cell.ch.encode_utf8(&mut buf);
                            let galley = self.galley(ui, text, font_id, face);
                            painter.galley(
                                Pos2::new(cell_rect.min.x, cell_rect.min.y + metrics.glyph_offset),
                                galley,
                                theme.cursor_accent,
                            );
                        }
                    }
                    CursorShape::Beam => {
                        painter.rect_filled(
                            Rect::from_min_size(cell_rect.min, Vec2::new(2.0, ch)),
                            0.0,
                            theme.cursor,
                        );
                    }
                    CursorShape::Underline => {
                        painter.rect_filled(
                            Rect::from_min_size(
                                Pos2::new(cell_rect.min.x, cell_rect.max.y - 2.0),
                                Vec2::new(width, 2.0),
                            ),
                            0.0,
                            theme.cursor,
                        );
                    }
                    CursorShape::Hidden => {}
                }
            }
        }

        // Scrollback position hint while scrolled up.
        if snap.display_offset > 0 && snap.history > 0 {
            let track_h = rect.height();
            let frac_offset = snap.display_offset as f32 / (snap.history + snap.lines) as f32;
            let frac_visible = snap.lines as f32 / (snap.history + snap.lines) as f32;
            let thumb_h = (track_h * frac_visible).max(24.0);
            let thumb_top =
                rect.max.y - thumb_h - (track_h - thumb_h) * frac_offset / (1.0 - frac_visible).max(0.001);
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(rect.max.x - 4.0, thumb_top),
                    Vec2::new(3.0, thumb_h),
                ),
                2.0,
                Color32::from_white_alpha(60),
            );
        }
    }
}
