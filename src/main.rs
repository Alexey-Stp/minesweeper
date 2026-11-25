#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! A clone of classic Windows Minesweeper built with `eframe`/`egui`.
//!
//! Controls:
//! * Left click: reveal a cell. The first click is always safe.
//! * Right click: cycle a covered cell through flag, question mark and blank.
//! * Middle click, or left+right together, on a number: "chord", which reveals
//!   every unflagged neighbour once the matching number of flags is placed.
//! * F2 or the smiley: start a new game.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Color32, FontId, Painter, PointerButton, Pos2, Rect, Sense, Shape, Stroke, Vec2,
    ViewportCommand, pos2, vec2,
};
use rand::Rng;
use rand::seq::SliceRandom;

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
// Game model: pure logic, independent of the UI.
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Difficulty {
    Beginner,
    Intermediate,
    Expert,
}

impl Difficulty {
    const ALL: [Self; 3] = [Self::Beginner, Self::Intermediate, Self::Expert];

    /// Board dimensions and mine count as `(columns, rows, mines)`.
    const fn spec(self) -> (usize, usize, usize) {
        match self {
            Self::Beginner => (9, 9, 10),
            Self::Intermediate => (16, 16, 40),
            Self::Expert => (30, 16, 99),
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Beginner => "Beginner",
            Self::Intermediate => "Intermediate",
            Self::Expert => "Expert",
        }
    }
}

/// What the player has done to a cell. Whether the cell holds a mine is separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CellState {
    Hidden,
    Flagged,
    Questioned,
    Revealed,
}

impl CellState {
    /// Flags protect a cell from being opened; question marks do not.
    const fn is_openable(self) -> bool {
        matches!(self, Self::Hidden | Self::Questioned)
    }
}

#[derive(Clone, Copy, Debug)]
struct Cell {
    mine: bool,
    /// Number of mines among the (up to eight) surrounding cells.
    adjacent: u8,
    state: CellState,
}

impl Cell {
    const FRESH: Self = Self {
        mine: false,
        adjacent: 0,
        state: CellState::Hidden,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GameStatus {
    /// Waiting for the first reveal. No mines exist yet, which is what makes
    /// the first click safe.
    Ready,
    Playing {
        started: Instant,
    },
    Won {
        seconds: u64,
    },
    /// `exploded` is the index of the mine that ended the game.
    Lost {
        seconds: u64,
        exploded: usize,
    },
}

/// How a cell is drawn, derived from its state and the game status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CellView {
    Hidden,
    Flag,
    Question,
    /// A revealed safe cell and its adjacent-mine count (0 draws blank).
    Open(u8),
    Mine,
    ExplodedMine,
    WrongFlag,
}

struct Game {
    cols: usize,
    rows: usize,
    mines: usize,
    cells: Vec<Cell>,
    status: GameStatus,
    flags: usize,
    revealed: usize,
}

impl Game {
    fn new(cols: usize, rows: usize, mines: usize) -> Self {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let len = cols * rows;
        Self {
            cols,
            rows,
            // One cell must stay free for the guaranteed-safe first click.
            mines: mines.min(len - 1),
            cells: vec![Cell::FRESH; len],
            status: GameStatus::Ready,
            flags: 0,
            revealed: 0,
        }
    }

    fn with_difficulty(difficulty: Difficulty) -> Self {
        let (cols, rows, mines) = difficulty.spec();
        Self::new(cols, rows, mines)
    }

    fn is_over(&self) -> bool {
        matches!(
            self.status,
            GameStatus::Won { .. } | GameStatus::Lost { .. }
        )
    }

    /// The counter shown to the player: negative when there are more flags than mines.
    fn mines_left(&self) -> i64 {
        self.mines as i64 - self.flags as i64
    }

    fn elapsed_seconds(&self) -> u64 {
        match self.status {
            GameStatus::Ready => 0,
            GameStatus::Playing { started } => started.elapsed().as_secs(),
            GameStatus::Won { seconds } | GameStatus::Lost { seconds, .. } => seconds,
        }
    }

    /// In-bounds neighbours of `idx`: 3 in a corner, 5 on an edge, 8 inside.
    /// The iterator does not borrow `self`, so callers may mutate while iterating.
    fn neighbors(&self, idx: usize) -> impl Iterator<Item = usize> + use<> {
        let (cols, rows) = (self.cols, self.rows);
        let (x, y) = (idx % cols, idx / cols);
        (-1isize..=1)
            .flat_map(|dy| (-1isize..=1).map(move |dx| (dx, dy)))
            .filter(|&offset| offset != (0, 0))
            .filter_map(move |(dx, dy)| {
                let nx = x.checked_add_signed(dx).filter(|&nx| nx < cols)?;
                let ny = y.checked_add_signed(dy).filter(|&ny| ny < rows)?;
                Some(ny * cols + nx)
            })
    }

    /// Scatters the mines, keeping `safe` and, when there is room, its neighbours clear.
    fn place_mines<R: Rng + ?Sized>(&mut self, safe: usize, rng: &mut R) {
        let mut protected: Vec<usize> = std::iter::once(safe).chain(self.neighbors(safe)).collect();
        if self.cells.len() - protected.len() < self.mines {
            // Too crowded to clear the whole 3x3 block: protect only the clicked cell.
            protected.truncate(1);
        }
        let mut candidates: Vec<usize> = (0..self.cells.len())
            .filter(|i| !protected.contains(i))
            .collect();
        candidates.shuffle(rng);
        for &i in candidates.iter().take(self.mines) {
            self.cells[i].mine = true;
        }
        for i in 0..self.cells.len() {
            let count = self.neighbors(i).filter(|&n| self.cells[n].mine).count();
            self.cells[i].adjacent = count as u8;
        }
    }

    /// Left click: open a cell. The first reveal places the mines and starts the clock.
    fn reveal(&mut self, idx: usize) {
        self.reveal_with(idx, &mut rand::rng());
    }

    fn reveal_with<R: Rng + ?Sized>(&mut self, idx: usize, rng: &mut R) {
        let openable = self.cells.get(idx).is_some_and(|c| c.state.is_openable());
        if self.is_over() || !openable {
            return;
        }
        if self.status == GameStatus::Ready {
            self.place_mines(idx, rng);
            self.status = GameStatus::Playing {
                started: Instant::now(),
            };
        }
        if self.cells[idx].mine {
            self.lose(idx);
        } else {
            self.flood_reveal(idx);
            self.check_win();
        }
    }

    /// Right click: Hidden -> Flagged -> Questioned -> Hidden.
    fn cycle_mark(&mut self, idx: usize) {
        if self.is_over() {
            return;
        }
        let Some(cell) = self.cells.get_mut(idx) else {
            return;
        };
        cell.state = match cell.state {
            CellState::Hidden => {
                self.flags += 1;
                CellState::Flagged
            }
            CellState::Flagged => {
                self.flags -= 1;
                CellState::Questioned
            }
            CellState::Questioned => CellState::Hidden,
            CellState::Revealed => return,
        };
    }

    /// Middle click (or left+right): when a revealed number has exactly that many
    /// flags around it, open every other covered neighbour.
    fn chord(&mut self, idx: usize) {
        if !matches!(self.status, GameStatus::Playing { .. }) {
            return;
        }
        let Some(&cell) = self.cells.get(idx) else {
            return;
        };
        if cell.state != CellState::Revealed || cell.adjacent == 0 {
            return;
        }
        let flags = self
            .neighbors(idx)
            .filter(|&n| self.cells[n].state == CellState::Flagged)
            .count();
        if flags != usize::from(cell.adjacent) {
            return;
        }

        let mut exploded = None;
        for n in self.neighbors(idx) {
            let neighbor = self.cells[n];
            if !neighbor.state.is_openable() {
                continue;
            }
            if neighbor.mine {
                exploded.get_or_insert(n);
            } else {
                self.flood_reveal(n);
            }
        }
        match exploded {
            Some(mine) => self.lose(mine),
            None => self.check_win(),
        }
    }

    /// Opens `start` and, through blank cells, everything connected to it (BFS).
    /// Numbered cells on the border are opened but not expanded; flags are never opened.
    fn flood_reveal(&mut self, start: usize) {
        let mut queue = VecDeque::from([start]);
        while let Some(i) = queue.pop_front() {
            let cell = &mut self.cells[i];
            if cell.mine || !cell.state.is_openable() {
                continue;
            }
            cell.state = CellState::Revealed;
            let blank = cell.adjacent == 0;
            self.revealed += 1;
            if blank {
                queue.extend(
                    self.neighbors(i)
                        .filter(|&n| self.cells[n].state.is_openable()),
                );
            }
        }
    }

    fn lose(&mut self, exploded: usize) {
        self.status = GameStatus::Lost {
            seconds: self.elapsed_seconds(),
            exploded,
        };
    }

    fn check_win(&mut self) {
        if self.revealed + self.mines != self.cells.len() {
            return;
        }
        for cell in self.cells.iter_mut().filter(|c| c.mine) {
            cell.state = CellState::Flagged;
        }
        self.flags = self.mines;
        self.status = GameStatus::Won {
            seconds: self.elapsed_seconds(),
        };
    }

    fn view(&self, idx: usize) -> CellView {
        let cell = self.cells[idx];
        if let GameStatus::Lost { exploded, .. } = self.status {
            let flagged = cell.state == CellState::Flagged;
            if idx == exploded {
                return CellView::ExplodedMine;
            }
            if cell.mine && !flagged {
                return CellView::Mine;
            }
            if !cell.mine && flagged {
                return CellView::WrongFlag;
            }
        }
        match cell.state {
            CellState::Hidden => CellView::Hidden,
            CellState::Flagged => CellView::Flag,
            CellState::Questioned => CellView::Question,
            CellState::Revealed => CellView::Open(cell.adjacent),
        }
    }
}

// =============================================================================
// Presentation: layout, input and painting.
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

/// One flag per mouse button.
#[derive(Clone, Copy, Debug, Default)]
struct Buttons {
    left: bool,
    right: bool,
    middle: bool,
}

impl Buttons {
    fn from_fn(f: impl Fn(PointerButton) -> bool) -> Self {
        Self {
            left: f(PointerButton::Primary),
            right: f(PointerButton::Secondary),
            middle: f(PointerButton::Middle),
        }
    }

    const fn any(self) -> bool {
        self.left || self.right || self.middle
    }
}

/// The pointer state the board needs for one frame.
#[derive(Clone, Copy, Debug, Default)]
struct PointerFrame {
    pos: Option<Pos2>,
    down: Buttons,
    pressed: Buttons,
    released: Buttons,
}

impl PointerFrame {
    fn read(ctx: &egui::Context) -> Self {
        ctx.input(|i| {
            let p = &i.pointer;
            Self {
                pos: p.hover_pos(),
                down: Buttons::from_fn(|b| p.button_down(b)),
                pressed: Buttons::from_fn(|b| p.button_pressed(b)),
                released: Buttons::from_fn(|b| p.button_released(b)),
            }
        })
    }
}

/// The mouse gesture in progress on the board.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Gesture {
    /// No button held, or the press started somewhere other than the board.
    #[default]
    Idle,
    /// Pressed on the board; releasing the left button reveals.
    Press,
    /// Both buttons, or the middle one, are held; releasing any of them chords.
    Chord,
    /// The chord fired; ignore the remaining releases until every button is up.
    Spent,
}

struct MinesweeperApp {
    difficulty: Difficulty,
    game: Game,
    gesture: Gesture,
    /// The window must be resized to fit the board on the next frame.
    resize_pending: bool,
}

impl MinesweeperApp {
    fn new(cc: &eframe::CreationContext<'_>, difficulty: Difficulty) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Light);
        Self {
            difficulty,
            game: Game::with_difficulty(difficulty),
            gesture: Gesture::Idle,
            resize_pending: true,
        }
    }

    fn new_game(&mut self) {
        self.game = Game::with_difficulty(self.difficulty);
        self.gesture = Gesture::Idle;
    }

    fn set_difficulty(&mut self, difficulty: Difficulty) {
        self.difficulty = difficulty;
        self.new_game();
        self.resize_pending = true;
    }

    fn handle_board_input(&mut self, pointer: PointerFrame, over_board: bool, cell: Option<usize>) {
        let PointerFrame {
            down,
            pressed,
            released,
            ..
        } = pointer;

        if self.gesture == Gesture::Idle && pressed.any() && over_board {
            self.gesture = Gesture::Press;
        }
        if self.gesture == Gesture::Press {
            // A lone right press marks immediately; with the left button held it starts a chord.
            if pressed.right
                && !down.left
                && !pressed.left
                && let Some(c) = cell
            {
                self.game.cycle_mark(c);
            }
            if (down.left && down.right) || down.middle || pressed.middle {
                self.gesture = Gesture::Chord;
            }
        }
        if released.any() {
            match self.gesture {
                Gesture::Chord => {
                    if let Some(c) = cell {
                        self.game.chord(c);
                    }
                    self.gesture = Gesture::Spent;
                }
                Gesture::Press if released.left => {
                    if let Some(c) = cell {
                        self.game.reveal(c);
                    }
                }
                _ => {}
            }
        }
        if !down.any() {
            self.gesture = Gesture::Idle;
        }
    }

    /// Cells drawn pushed-in while a button is held, as in the original.
    fn pressed_cells(&self, pointer: PointerFrame, cell: Option<usize>) -> Vec<usize> {
        let Some(cell) = cell.filter(|_| !self.game.is_over()) else {
            return Vec::new();
        };
        match self.gesture {
            Gesture::Press if pointer.down.left => vec![cell],
            Gesture::Chord => std::iter::once(cell)
                .chain(self.game.neighbors(cell))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn face(&self, pointer: PointerFrame) -> &'static str {
        let holding = match self.gesture {
            Gesture::Press => pointer.down.left,
            Gesture::Chord => true,
            Gesture::Idle | Gesture::Spent => false,
        };
        match self.game.status {
            GameStatus::Won { .. } => "😎",
            GameStatus::Lost { .. } => "😵",
            GameStatus::Ready | GameStatus::Playing { .. } if holding => "😮",
            // egui's bundled emoji font has no U+1F642 🙂, so use the classic U+263A smiley.
            GameStatus::Ready | GameStatus::Playing { .. } => "☺",
        }
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

    fn game_area(&mut self, ui: &mut egui::Ui) {
        let (cols, rows) = (self.game.cols, self.game.rows);
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

        let pointer = PointerFrame::read(ui.ctx());
        let cell = pointer.pos.and_then(|p| layout.cell_at(p));
        self.handle_board_input(pointer, board.hovered(), cell);

        let painter = ui.painter_at(layout.area);
        paint_frame(&painter, &layout);
        paint_lcd(&painter, layout.mines_lcd, self.game.mines_left());
        paint_lcd(
            &painter,
            layout.timer_lcd,
            self.game.elapsed_seconds() as i64,
        );
        paint_face(
            &painter,
            layout.face,
            self.face(pointer),
            face.is_pointer_button_down_on(),
        );
        let pressed = self.pressed_cells(pointer, cell);
        for idx in 0..self.game.cells.len() {
            paint_cell(
                &painter,
                layout.cell_rect(idx),
                self.game.view(idx),
                pressed.contains(&idx),
            );
        }
    }
}

impl eframe::App for MinesweeperApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.key_pressed(egui::Key::F2)) {
            self.new_game();
        }

        let menu = egui::Panel::top("menu_bar").show(ui, |ui| self.menu_bar(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(SURFACE))
            .show(ui, |ui| self.game_area(ui));

        if std::mem::take(&mut self.resize_pending) {
            let size = window_size(self.difficulty, menu.response.rect.height());
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
        }
        if matches!(self.game.status, GameStatus::Playing { .. }) {
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
    use rand::rngs::StdRng;
    use rand::{RngExt, SeedableRng};

    /// A game in progress built from rows of text, where `*` is a mine.
    fn game_from(layout: &[&str]) -> Game {
        let (rows, cols) = (layout.len(), layout[0].len());
        let mines: Vec<usize> = layout
            .concat()
            .char_indices()
            .filter(|&(_, ch)| ch == '*')
            .map(|(i, _)| i)
            .collect();
        let mut game = Game::new(cols, rows, mines.len());
        for &i in &mines {
            game.cells[i].mine = true;
        }
        for i in 0..game.cells.len() {
            let count = game.neighbors(i).filter(|&n| game.cells[n].mine).count();
            game.cells[i].adjacent = count as u8;
        }
        game.status = GameStatus::Playing {
            started: Instant::now(),
        };
        game
    }

    fn states(game: &Game) -> Vec<CellState> {
        game.cells.iter().map(|c| c.state).collect()
    }

    #[test]
    fn difficulty_presets_match_the_original() {
        assert_eq!(Difficulty::Beginner.spec(), (9, 9, 10));
        assert_eq!(Difficulty::Intermediate.spec(), (16, 16, 40));
        assert_eq!(Difficulty::Expert.spec(), (30, 16, 99));
    }

    #[test]
    fn neighbours_stay_in_bounds() {
        let game = Game::new(4, 3, 0);
        let counts: Vec<usize> = (0..12).map(|i| game.neighbors(i).count()).collect();
        assert_eq!(counts, [3, 5, 5, 3, 5, 8, 8, 5, 3, 5, 5, 3]);
        assert!((0..12).flat_map(|i| game.neighbors(i)).all(|n| n < 12));

        let line = Game::new(1, 1, 0);
        assert_eq!(line.neighbors(0).count(), 0);
    }

    #[test]
    fn first_click_and_its_neighbours_are_always_safe() {
        for difficulty in Difficulty::ALL {
            let (cols, rows, mines) = difficulty.spec();
            for seed in 0..200 {
                let mut game = Game::with_difficulty(difficulty);
                let mut rng = StdRng::seed_from_u64(seed);
                let click = rng.random_range(0..cols * rows);
                game.reveal_with(click, &mut rng);

                assert!(!matches!(game.status, GameStatus::Lost { .. }));
                assert_eq!(game.cells.iter().filter(|c| c.mine).count(), mines);
                assert!(!game.cells[click].mine);
                assert!(game.neighbors(click).all(|n| !game.cells[n].mine));
                // With no adjacent mines, the first click always opens an area.
                assert_eq!(game.cells[click].adjacent, 0);
                assert!(game.revealed > 1);
            }
        }
    }

    #[test]
    fn crowded_board_still_protects_the_first_click() {
        let mut game = Game::new(3, 3, 8);
        game.reveal_with(4, &mut StdRng::seed_from_u64(7));
        assert!(!game.cells[4].mine);
        assert_eq!(game.cells.iter().filter(|c| c.mine).count(), 8);
        assert!(matches!(game.status, GameStatus::Won { .. }));
    }

    #[test]
    fn mine_count_is_capped_to_leave_a_safe_cell() {
        assert_eq!(Game::new(2, 2, 50).mines, 3);
    }

    #[test]
    fn flood_fill_opens_the_blank_region_and_its_numbered_border() {
        let mut game = game_from(&[
            "....", //
            "....", //
            "..**", //
            "..*.", //
        ]);
        game.reveal(0);
        use CellState::{Hidden as H, Revealed as R};
        assert_eq!(
            states(&game),
            [
                R, R, R, R, //
                R, R, R, R, //
                R, R, H, H, //
                R, R, H, H, //
            ]
        );
        assert_eq!(game.revealed, 12);
        assert!(matches!(game.status, GameStatus::Playing { .. }));
    }

    #[test]
    fn flood_fill_skips_flags_but_opens_question_marks() {
        let mut game = game_from(&[
            "....", //
            "....", //
            "....", //
            "...*", //
        ]);
        game.cycle_mark(1); // flag
        game.cycle_mark(2);
        game.cycle_mark(2); // question mark
        game.reveal(12); // blank region that touches both marked cells
        assert_eq!(game.cells[1].state, CellState::Flagged);
        assert_eq!(game.cells[2].state, CellState::Revealed);
        assert_eq!(game.cells[0].state, CellState::Revealed);
        assert_eq!(game.revealed, 14);
    }

    #[test]
    fn right_click_cycles_marks_and_updates_the_counter() {
        let mut game = game_from(&["*..", "...", "..."]);
        assert_eq!(game.mines_left(), 1);
        game.cycle_mark(4);
        assert_eq!(
            (game.cells[4].state, game.mines_left()),
            (CellState::Flagged, 0)
        );
        game.cycle_mark(5);
        assert_eq!(game.mines_left(), -1);
        game.cycle_mark(4);
        assert_eq!(
            (game.cells[4].state, game.mines_left()),
            (CellState::Questioned, 0)
        );
        game.cycle_mark(4);
        assert_eq!(
            (game.cells[4].state, game.mines_left()),
            (CellState::Hidden, 0)
        );
    }

    #[test]
    fn flagged_cells_cannot_be_revealed_and_revealed_cells_cannot_be_marked() {
        let mut game = game_from(&["*..", "...", "..."]);
        game.cycle_mark(0);
        game.reveal(0); // flagged: no explosion
        assert!(matches!(game.status, GameStatus::Playing { .. }));

        game.reveal(8);
        let before = game.cells[8].state;
        game.cycle_mark(8);
        assert_eq!(game.cells[8].state, before);
    }

    #[test]
    fn chord_opens_neighbours_when_flags_match() {
        let mut game = game_from(&["*..", "...", "..*"]);
        game.reveal(4); // a "2"
        assert_eq!(game.cells[4].adjacent, 2);

        game.chord(4); // no flags yet: nothing happens
        assert_eq!(game.revealed, 1);

        game.cycle_mark(0);
        game.chord(4); // one flag of two: still nothing
        assert_eq!(game.revealed, 1);

        game.cycle_mark(8);
        game.chord(4);
        assert!(matches!(game.status, GameStatus::Won { .. }));
        assert_eq!(game.revealed, 7);
    }

    #[test]
    fn chord_with_a_misplaced_flag_loses() {
        let mut game = game_from(&["*..", "...", "..."]);
        game.reveal(4); // a "1"
        game.cycle_mark(2); // wrong guess
        game.chord(4);
        assert!(matches!(game.status, GameStatus::Lost { exploded: 0, .. }));
        assert_eq!(game.view(0), CellView::ExplodedMine);
        assert_eq!(game.view(2), CellView::WrongFlag);
    }

    #[test]
    fn losing_reveals_mines_and_wrong_flags() {
        let mut game = game_from(&["*.*", "...", "*.."]);
        game.cycle_mark(0); // correct flag
        game.cycle_mark(8); // wrong flag
        game.reveal(2); // boom

        assert!(matches!(game.status, GameStatus::Lost { exploded: 2, .. }));
        assert_eq!(game.view(0), CellView::Flag);
        assert_eq!(game.view(2), CellView::ExplodedMine);
        assert_eq!(game.view(6), CellView::Mine);
        assert_eq!(game.view(8), CellView::WrongFlag);
        assert_eq!(game.view(4), CellView::Hidden);

        // The board is frozen once the game is over.
        let frozen = states(&game);
        game.reveal(4);
        game.cycle_mark(4);
        game.chord(4);
        assert_eq!(states(&game), frozen);
    }

    #[test]
    fn winning_flags_every_remaining_mine() {
        let mut game = game_from(&["*..", "...", "..*"]);
        game.cycle_mark(8); // question mark on a mine stays covered
        game.cycle_mark(8);
        for i in 1..8 {
            game.reveal(i);
        }
        assert!(matches!(game.status, GameStatus::Won { .. }));
        assert_eq!(game.mines_left(), 0);
        assert_eq!(game.view(0), CellView::Flag);
        assert_eq!(game.view(8), CellView::Flag);
    }

    #[test]
    fn out_of_range_indices_are_ignored() {
        let mut game = Game::with_difficulty(Difficulty::Beginner);
        game.reveal(usize::MAX);
        game.cycle_mark(81);
        game.chord(1_000);
        assert_eq!(game.status, GameStatus::Ready);
        assert_eq!(game.flags, 0);
    }

    #[test]
    fn timer_is_frozen_before_start_and_after_the_end() {
        let mut game = game_from(&["*.", ".."]);
        game.status = GameStatus::Ready;
        assert_eq!(game.elapsed_seconds(), 0);
        game.status = GameStatus::Won { seconds: 42 };
        assert_eq!(game.elapsed_seconds(), 42);
    }

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
}
