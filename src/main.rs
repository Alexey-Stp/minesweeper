#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! A clone of classic Windows Minesweeper built with `eframe`/`egui`.
//!
//! Controls:
//! * Left click: reveal a cell. The first click is always safe.
//! * Right click: cycle a covered cell through flag, question mark and unmarked.
//! * Middle click, or left+right together, on a number: "chord", which reveals
//!   every unflagged neighbour once the matching number of flags is placed.
//! * F2 or the face: start a new game.
//!
//! The rules of play live in [`game`] and the mouse rules in [`board_input`]. This file is the
//! egui side: window, layout, reading the pointer, and painting.

mod board_input;
mod game;

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Color32, FontId, Painter, PointerButton, Pos2, Rect, Sense, Shape, Stroke, Vec2,
    ViewportCommand, pos2, vec2,
};

use board_input::{BoardInput, Buttons, PointerFrame};
use game::{CellView, Difficulty, Game, GameStatus};

fn main() -> eframe::Result {
    let difficulty = Difficulty::Beginner;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Minesweeper")
            .with_inner_size(window_size(difficulty, MENU_HEIGHT_GUESS))
            .with_resizable(false)
            .with_maximize_button(false),
        ..Default::default()
    };
    eframe::run_native(
        "Minesweeper",
        options,
        Box::new(move |cc| Ok(Box::new(MinesweeperApp::new(cc, difficulty)))),
    )
}

// =============================================================================
// Presentation: layout, pointer reading and painting.
// =============================================================================

const CELL: f32 = 24.0;
const CELL_BEVEL: f32 = 3.0;
const FRAME_BEVEL: f32 = 3.0;
const HEADER_BEVEL: f32 = 2.0;
const MARGIN: f32 = 8.0;
/// Height of the status header inside its bevel.
const HEADER_HEIGHT: f32 = 40.0;
const FACE_SIZE: f32 = 32.0;
const DIGIT_SIZE: Vec2 = vec2(13.0, 23.0);
const DIGIT_GAP: f32 = 2.0;
const SEGMENT_THICKNESS: f32 = 3.0;
const LCD_PADDING: f32 = 3.0;
/// Used only for the very first window size; corrected on the first frame.
const MENU_HEIGHT_GUESS: f32 = 24.0;

const SURFACE: Color32 = Color32::from_rgb(0xC0, 0xC0, 0xC0);
const HIGHLIGHT: Color32 = Color32::WHITE;
const SHADOW: Color32 = Color32::from_rgb(0x80, 0x80, 0x80);
const EXPLODED_RED: Color32 = Color32::from_rgb(0xFF, 0x00, 0x00);
const FLAG_RED: Color32 = Color32::from_rgb(0xD0, 0x00, 0x00);
const FACE_YELLOW: Color32 = Color32::from_rgb(0xFF, 0xE0, 0x20);
const LCD_LIT: Color32 = Color32::from_rgb(0xFF, 0x00, 0x00);
const LCD_UNLIT: Color32 = Color32::from_rgb(0x48, 0x00, 0x00);

/// Classic Windows colours for the numbers 1 through 8.
const NUMBER_COLORS: [Color32; 8] = [
    Color32::from_rgb(0x00, 0x00, 0xFF), // 1: blue
    Color32::from_rgb(0x00, 0x80, 0x00), // 2: green
    Color32::from_rgb(0xFF, 0x00, 0x00), // 3: red
    Color32::from_rgb(0x00, 0x00, 0x80), // 4: dark blue
    Color32::from_rgb(0x80, 0x00, 0x00), // 5: maroon
    Color32::from_rgb(0x00, 0x80, 0x80), // 6: teal
    Color32::from_rgb(0x00, 0x00, 0x00), // 7: black
    Color32::from_rgb(0x80, 0x80, 0x80), // 8: gray
];

/// Seven-segment encodings of 0-9. Bits 0..=6 are segments a (top), b (top right),
/// c (bottom right), d (bottom), e (bottom left), f (top left) and g (middle).
const DIGIT_SEGMENTS: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F];
const MINUS_SEGMENTS: u8 = 0x40;

/// The three glyphs of an LCD counter, clamped to the displayable range -99..=999.
fn lcd_glyphs(value: i64) -> [u8; 3] {
    let v = value.clamp(-99, 999);
    let digit = |d: i64| DIGIT_SEGMENTS[(d.unsigned_abs() % 10) as usize];
    if v < 0 {
        [MINUS_SEGMENTS, digit(v / 10), digit(v)]
    } else {
        [digit(v / 100), digit(v / 10), digit(v)]
    }
}

fn lcd_size() -> Vec2 {
    vec2(
        2.0 * LCD_PADDING + 3.0 * DIGIT_SIZE.x + 2.0 * DIGIT_GAP,
        2.0 * LCD_PADDING + DIGIT_SIZE.y,
    )
}

/// Size of everything below the menu bar: header, board and the frame around them.
fn game_area_size(cols: usize, rows: usize) -> Vec2 {
    let inset = 2.0 * (FRAME_BEVEL + MARGIN);
    let board = vec2(cols as f32, rows as f32) * CELL + Vec2::splat(2.0 * FRAME_BEVEL);
    let header = HEADER_HEIGHT + 2.0 * HEADER_BEVEL + MARGIN;
    vec2(inset + board.x, inset + header + board.y)
}

fn window_size(difficulty: Difficulty, menu_height: f32) -> Vec2 {
    let (cols, rows, _) = difficulty.spec();
    game_area_size(cols, rows) + vec2(0.0, menu_height)
}

/// Screen rectangles of every part of the game area for one frame.
struct Layout {
    area: Rect,
    header: Rect,
    mines_lcd: Rect,
    face: Rect,
    timer_lcd: Rect,
    board_frame: Rect,
    board: Rect,
    cols: usize,
    rows: usize,
}

impl Layout {
    fn new(min: Pos2, cols: usize, rows: usize) -> Self {
        let area = Rect::from_min_size(min, game_area_size(cols, rows));
        let inner = area.shrink(FRAME_BEVEL + MARGIN);
        let header = Rect::from_min_size(
            inner.min,
            vec2(inner.width(), HEADER_HEIGHT + 2.0 * HEADER_BEVEL),
        );
        let board_frame = Rect::from_min_max(pos2(inner.min.x, header.max.y + MARGIN), inner.max);

        let lcd = lcd_size();
        let side = HEADER_BEVEL + 5.0;
        let lcd_top = (header.center().y - lcd.y / 2.0).round();
        Self {
            area,
            header,
            mines_lcd: Rect::from_min_size(pos2(header.min.x + side, lcd_top), lcd),
            face: Rect::from_center_size(header.center(), Vec2::splat(FACE_SIZE)),
            timer_lcd: Rect::from_min_size(pos2(header.max.x - side - lcd.x, lcd_top), lcd),
            board_frame,
            board: board_frame.shrink(FRAME_BEVEL),
            cols,
            rows,
        }
    }

    fn cell_rect(&self, idx: usize) -> Rect {
        let (x, y) = (idx % self.cols, idx / self.cols);
        Rect::from_min_size(
            self.board.min + vec2(x as f32, y as f32) * CELL,
            Vec2::splat(CELL),
        )
    }

    fn cell_at(&self, pos: Pos2) -> Option<usize> {
        if !self.board.contains(pos) {
            return None;
        }
        let rel = (pos - self.board.min) / CELL;
        let x = (rel.x as usize).min(self.cols - 1);
        let y = (rel.y as usize).min(self.rows - 1);
        Some(y * self.cols + x)
    }
}

/// Reads this frame's pointer in board terms. `on_board` is egui's answer to whether the
/// board itself is under the pointer, which is false where an open menu covers it.
fn read_pointer(ctx: &egui::Context, layout: &Layout, on_board: bool) -> PointerFrame {
    ctx.input(|i| {
        let p = &i.pointer;
        PointerFrame {
            cell: p.hover_pos().and_then(|pos| layout.cell_at(pos)),
            on_board,
            down: buttons(|b| p.button_down(b)),
            pressed: buttons(|b| p.button_pressed(b)),
            released: buttons(|b| p.button_released(b)),
        }
    })
}

fn buttons(f: impl Fn(PointerButton) -> bool) -> Buttons {
    Buttons {
        left: f(PointerButton::Primary),
        right: f(PointerButton::Secondary),
        middle: f(PointerButton::Middle),
    }
}

/// The face for the game's status, gasping while a button is held on the board.
fn face_glyph(status: GameStatus, holding: bool) -> &'static str {
    match status {
        GameStatus::Won { .. } => "😎",
        GameStatus::Lost { .. } => "😵",
        GameStatus::Ready | GameStatus::Playing { .. } if holding => "😮",
        // egui's bundled emoji font has no U+1F642 🙂, so use the classic U+263A smiley.
        GameStatus::Ready | GameStatus::Playing { .. } => "☺",
    }
}

struct MinesweeperApp {
    difficulty: Difficulty,
    game: Game,
    input: BoardInput,
    /// The window must be resized to fit the board on the next frame.
    resize_pending: bool,
}

impl MinesweeperApp {
    fn new(cc: &eframe::CreationContext<'_>, difficulty: Difficulty) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Light);
        Self {
            difficulty,
            game: Game::with_difficulty(difficulty),
            input: BoardInput::default(),
            resize_pending: true,
        }
    }

    fn new_game(&mut self) {
        self.game = Game::with_difficulty(self.difficulty);
        self.input = BoardInput::default();
    }

    fn set_difficulty(&mut self, difficulty: Difficulty) {
        self.difficulty = difficulty;
        self.new_game();
        self.resize_pending = true;
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Game", |ui| {
                if ui
                    .add(egui::Button::new("New").shortcut_text("F2"))
                    .clicked()
                {
                    self.new_game();
                    ui.close();
                }
                ui.separator();
                for difficulty in Difficulty::ALL {
                    let (cols, rows, mines) = difficulty.spec();
                    let chosen = ui
                        .radio(self.difficulty == difficulty, difficulty.name())
                        .on_hover_text(format!("{cols} × {rows}, {mines} mines"));
                    if chosen.clicked() {
                        self.set_difficulty(difficulty);
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                }
            });
        });
    }

    fn game_area(&mut self, ui: &mut egui::Ui, now: Instant) {
        let (cols, rows) = self.game.size();
        let full = ui.max_rect();
        let width = game_area_size(cols, rows).x;
        let min = pos2((full.center().x - width / 2.0).max(full.min.x), full.min.y).round();
        let layout = Layout::new(min, cols, rows);

        ui.allocate_rect(layout.area, Sense::hover());
        let face = ui.interact(layout.face, ui.scope_id().with("face"), Sense::click());
        let board = ui.interact(layout.board, ui.scope_id().with("board"), Sense::click());
        if face.clicked() {
            self.new_game();
        }

        let pointer = read_pointer(ui.ctx(), &layout, board.hovered());
        let feedback = self.input.frame(&mut self.game, pointer, now);

        let painter = ui.painter_at(layout.area);
        paint_frame(&painter, &layout);
        paint_lcd(&painter, layout.mines_lcd, self.game.mines_left());
        paint_lcd(
            &painter,
            layout.timer_lcd,
            self.game.elapsed_seconds(now) as i64,
        );
        paint_face(
            &painter,
            layout.face,
            face_glyph(self.game.status(), feedback.holding),
            face.is_pointer_button_down_on(),
        );
        for idx in 0..cols * rows {
            paint_cell(
                &painter,
                layout.cell_rect(idx),
                self.game.view(idx),
                feedback.pressed_cells.contains(&idx),
            );
        }
    }
}

impl eframe::App for MinesweeperApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        if ctx.input(|i| i.key_pressed(egui::Key::F2)) {
            self.new_game();
        }

        let menu = egui::Panel::top("menu_bar").show(ui, |ui| self.menu_bar(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(SURFACE))
            .show(ui, |ui| self.game_area(ui, now));

        if std::mem::take(&mut self.resize_pending) {
            let size = window_size(self.difficulty, menu.response.rect.height());
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
        }
        if matches!(self.game.status(), GameStatus::Playing { .. }) {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }
}

// ----------------------------------------------------------------------------
// Painting helpers
// ----------------------------------------------------------------------------

fn fill(painter: &Painter, rect: Rect, color: Color32) {
    painter.rect_filled(rect, 0.0, color);
}

/// A classic 3-D edge: `light` along the top and left, `dark` along the bottom and right,
/// meeting diagonally in the corners.
fn paint_bevel(painter: &Painter, rect: Rect, width: f32, light: Color32, dark: Color32) {
    let (min, max, w) = (rect.min, rect.max, width);
    let (tr, bl) = (pos2(max.x, min.y), pos2(min.x, max.y));
    let (inner_min, inner_max) = (min + Vec2::splat(w), max - Vec2::splat(w));
    let (inner_tr, inner_bl) = (
        pos2(inner_max.x, inner_min.y),
        pos2(inner_min.x, inner_max.y),
    );
    let quads = [
        ([min, tr, inner_tr, inner_min], light),
        ([min, inner_min, inner_bl, bl], light),
        ([bl, inner_bl, inner_max, max], dark),
        ([tr, max, inner_max, inner_tr], dark),
    ];
    for (points, color) in quads {
        painter.add(Shape::convex_polygon(points.to_vec(), color, Stroke::NONE));
    }
}

fn paint_raised(painter: &Painter, rect: Rect, width: f32) {
    paint_bevel(painter, rect, width, HIGHLIGHT, SHADOW);
}

fn paint_sunken(painter: &Painter, rect: Rect, width: f32) {
    paint_bevel(painter, rect, width, SHADOW, HIGHLIGHT);
}

fn paint_frame(painter: &Painter, layout: &Layout) {
    fill(painter, layout.area, SURFACE);
    paint_raised(painter, layout.area, FRAME_BEVEL);
    paint_sunken(painter, layout.header, HEADER_BEVEL);
    paint_sunken(painter, layout.board_frame, FRAME_BEVEL);
}

fn paint_lcd(painter: &Painter, rect: Rect, value: i64) {
    fill(painter, rect, Color32::BLACK);
    paint_sunken(painter, rect, 1.0);
    for (i, segments) in lcd_glyphs(value).into_iter().enumerate() {
        let offset = vec2(
            LCD_PADDING + i as f32 * (DIGIT_SIZE.x + DIGIT_GAP),
            LCD_PADDING,
        );
        paint_digit(
            painter,
            Rect::from_min_size(rect.min + offset, DIGIT_SIZE),
            segments,
        );
    }
}

fn paint_digit(painter: &Painter, rect: Rect, segments: u8) {
    let half = SEGMENT_THICKNESS / 2.0;
    let (left, right) = (rect.left() + half, rect.right() - half);
    let (top, mid, bottom) = (rect.top() + half, rect.center().y, rect.bottom() - half);
    // Centre lines of segments a..=g, in bit order.
    let lines = [
        (pos2(left, top), pos2(right, top)),
        (pos2(right, top), pos2(right, mid)),
        (pos2(right, mid), pos2(right, bottom)),
        (pos2(left, bottom), pos2(right, bottom)),
        (pos2(left, mid), pos2(left, bottom)),
        (pos2(left, top), pos2(left, mid)),
        (pos2(left, mid), pos2(right, mid)),
    ];
    for (bit, (from, to)) in lines.into_iter().enumerate() {
        let color = if segments & (1 << bit) != 0 {
            LCD_LIT
        } else {
            LCD_UNLIT
        };
        painter.add(Shape::convex_polygon(
            segment_hexagon(from, to),
            color,
            Stroke::NONE,
        ));
    }
}

/// A seven-segment bar along `from -> to` with pointed ends, wound clockwise.
fn segment_hexagon(from: Pos2, to: Pos2) -> Vec<Pos2> {
    const GAP: f32 = 0.8;
    let dir = (to - from).normalized();
    let along = dir * (SEGMENT_THICKNESS / 2.0);
    let side = dir.rot90() * (SEGMENT_THICKNESS / 2.0);
    let (a, b) = (from + dir * GAP, to - dir * GAP);
    vec![
        a,
        a + along - side,
        b - along - side,
        b,
        b - along + side,
        a + along + side,
    ]
}

fn paint_face(painter: &Painter, rect: Rect, face: &str, held: bool) {
    fill(painter, rect, SHADOW);
    let inner = rect.shrink(1.0);
    fill(painter, inner, SURFACE);
    let nudge = if held {
        paint_sunken(painter, inner, 1.0);
        vec2(1.0, 1.0)
    } else {
        paint_raised(painter, inner, 2.0);
        Vec2::ZERO
    };
    let center = inner.center() + nudge;

    // The bundled emoji font is monochrome, so fill the face yellow underneath it. Centre the
    // glyph's ink rather than its line box so the fill lines up with the emoji's round outline.
    let galley =
        painter.layout_no_wrap(face.to_owned(), FontId::proportional(22.0), Color32::BLACK);
    let ink = ink_rect(&galley);
    let radius = (ink.size().min_elem() / 2.0 - 1.0).max(0.0);
    painter.circle_filled(center, radius, FACE_YELLOW);
    painter.galley(center - ink.center().to_vec2(), galley, Color32::BLACK);
}

fn paint_cell(painter: &Painter, rect: Rect, view: CellView, pressed: bool) {
    let raised = match view {
        CellView::Flag => true,
        CellView::Hidden | CellView::Question => !pressed,
        _ => false,
    };
    if raised {
        fill(painter, rect, SURFACE);
        paint_raised(painter, rect, CELL_BEVEL);
    } else {
        let background = if view == CellView::ExplodedMine {
            EXPLODED_RED
        } else {
            SURFACE
        };
        fill(painter, rect, background);
        // Thin grid lines along the top and left edges, like the original.
        fill(
            painter,
            Rect::from_min_size(rect.min, vec2(rect.width(), 1.0)),
            SHADOW,
        );
        fill(
            painter,
            Rect::from_min_size(rect.min, vec2(1.0, rect.height())),
            SHADOW,
        );
    }

    let center = rect.center();
    match view {
        CellView::Open(n @ 1..=8) => {
            paint_bold(
                painter,
                center,
                &n.to_string(),
                NUMBER_COLORS[usize::from(n - 1)],
            );
        }
        CellView::Question => paint_bold(painter, center, "?", Color32::BLACK),
        CellView::Flag => paint_symbol(painter, center, "🚩", FLAG_RED),
        CellView::Mine | CellView::ExplodedMine => {
            paint_symbol(painter, center, "💣", Color32::BLACK)
        }
        CellView::WrongFlag => paint_symbol(painter, center, "❌", FLAG_RED),
        CellView::Hidden | CellView::Open(_) => {}
    }
}

/// The area covered by the first glyph's ink, relative to the galley.
///
/// Emoji come from fallback fonts whose line box is not centred on the drawing itself,
/// so symbols are positioned by their ink instead.
fn ink_rect(galley: &egui::Galley) -> Rect {
    galley
        .rows
        .first()
        .and_then(|row| {
            let glyph = row.glyphs.first()?;
            let min = row.pos + glyph.pos.to_vec2() + glyph.uv_rect.offset;
            Some(Rect::from_min_size(min, glyph.uv_rect.size))
        })
        .filter(Rect::is_positive)
        .unwrap_or(galley.rect)
}

/// Paints `text` with its ink centred on `center`.
fn paint_centered(painter: &Painter, center: Pos2, text: &str, font: FontId, color: Color32) {
    let galley = painter.layout_no_wrap(text.to_owned(), font, color);
    let pos = center - ink_rect(&galley).center().to_vec2();
    painter.galley(pos, galley, color);
}

/// Text drawn twice, one point apart, for the heavy look of the original digits.
fn paint_bold(painter: &Painter, center: Pos2, text: &str, color: Color32) {
    for dx in [-0.5, 0.5] {
        paint_centered(
            painter,
            center + vec2(dx, 0.0),
            text,
            FontId::monospace(18.0),
            color,
        );
    }
}

fn paint_symbol(painter: &Painter, center: Pos2, symbol: &str, color: Color32) {
    paint_centered(painter, center, symbol, FontId::proportional(16.0), color);
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcd_shows_three_digits_with_a_minus_sign_and_clamps() {
        let d = DIGIT_SEGMENTS;
        assert_eq!(lcd_glyphs(0), [d[0], d[0], d[0]]);
        assert_eq!(lcd_glyphs(42), [d[0], d[4], d[2]]);
        assert_eq!(lcd_glyphs(1234), [d[9], d[9], d[9]]);
        assert_eq!(lcd_glyphs(-7), [MINUS_SEGMENTS, d[0], d[7]]);
        assert_eq!(lcd_glyphs(-150), [MINUS_SEGMENTS, d[9], d[9]]);
    }

    #[test]
    fn layout_maps_points_to_cells_inside_the_board_only() {
        let layout = Layout::new(Pos2::ZERO, 9, 9);
        let first = layout.cell_rect(0);
        let last = layout.cell_rect(80);
        assert_eq!(layout.cell_at(first.center()), Some(0));
        assert_eq!(layout.cell_at(last.center()), Some(80));
        assert_eq!(layout.cell_at(layout.board.max), Some(80));
        assert_eq!(layout.cell_at(layout.board.min - vec2(1.0, 1.0)), None);
        assert_eq!(layout.cell_at(layout.face.center()), None);
        assert!(layout.area.contains_rect(layout.board_frame));
    }

    #[test]
    fn face_gasps_only_while_the_game_is_on() {
        assert_eq!(face_glyph(GameStatus::Ready, false), "☺");
        assert_eq!(face_glyph(GameStatus::Ready, true), "😮");
        let lost = GameStatus::Lost {
            seconds: 3,
            exploded: 0,
        };
        assert_eq!(face_glyph(lost, true), "😵");
        assert_eq!(face_glyph(GameStatus::Won { seconds: 3 }, true), "😎");
    }
}
