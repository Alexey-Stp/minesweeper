//! The game model: the rules of play, independent of the UI.
//!
//! Callers play through [`Game`]'s interface: moves in ([`Game::reveal`], [`Game::cycle_mark`],
//! [`Game::chord`]), views out ([`Game::view`], [`Game::status`] and the counters). Where the
//! mines go comes in across the [`MineLayout`] seam, and the time comes in with every move that
//! can start or end the game, so a game is fully determined by its inputs.

use std::collections::VecDeque;
use std::ops::Range;
use std::time::Instant;

use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Difficulty {
    Beginner,
    Intermediate,
    Expert,
}

impl Difficulty {
    pub const ALL: [Self; 3] = [Self::Beginner, Self::Intermediate, Self::Expert];

    /// Board dimensions and mine count as `(columns, rows, mines)`.
    pub const fn spec(self) -> (usize, usize, usize) {
        match self {
            Self::Beginner => (9, 9, 10),
            Self::Intermediate => (16, 16, 40),
            Self::Expert => (30, 16, 99),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Beginner => "Beginner",
            Self::Intermediate => "Intermediate",
            Self::Expert => "Expert",
        }
    }
}

/// The shape of a board: its size, and how its cells are numbered (row by row from the top
/// left, starting at 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Board {
    cols: usize,
    rows: usize,
}

impl Board {
    /// A board of `cols` × `rows` cells; each side is at least one cell.
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols: cols.max(1),
            rows: rows.max(1),
        }
    }

    pub fn cols(self) -> usize {
        self.cols
    }

    pub fn rows(self) -> usize {
        self.rows
    }

    /// Every cell, in order.
    pub fn cells(self) -> Range<usize> {
        0..self.cols * self.rows
    }

    /// The cell at column `col` of row `row`.
    pub fn cell(self, col: usize, row: usize) -> usize {
        row * self.cols + col
    }

    /// The column and row of `cell`.
    pub fn position(self, cell: usize) -> (usize, usize) {
        (cell % self.cols, cell / self.cols)
    }

    /// The neighbours of `cell`: 3 in a corner, 5 on an edge, 8 inside.
    pub fn neighbours(self, cell: usize) -> impl Iterator<Item = usize> {
        let (col, row) = self.position(cell);
        (-1isize..=1)
            .flat_map(|dy| (-1isize..=1).map(move |dx| (dx, dy)))
            .filter(|&offset| offset != (0, 0))
            .filter_map(move |(dx, dy)| {
                let c = col.checked_add_signed(dx).filter(|&c| c < self.cols)?;
                let r = row.checked_add_signed(dy).filter(|&r| r < self.rows)?;
                Some(self.cell(c, r))
            })
    }
}

/// Where the mines go. Chosen when the game is created and laid on the first reveal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MineLayout {
    /// `mines` cells picked by a random generator seeded with `seed`, keeping the first
    /// revealed cell clear and, when there is room, its neighbours too.
    Random { mines: usize, seed: u64 },
    /// Exactly these cells hold mines; cells off the board and repeats are ignored. The first
    /// reveal gets no protection. Play always uses `Random`; tests use this for known boards.
    Fixed(Vec<usize>),
}

impl MineLayout {
    /// `mines` cells picked at random.
    pub fn random(mines: usize) -> Self {
        Self::Random {
            mines,
            seed: rand::random(),
        }
    }

    /// The layout fitted to `board`: a random layout leaves at least one cell free for the
    /// first reveal, and a fixed one drops cells off the board and repeats.
    fn fitted(self, board: Board) -> Self {
        let len = board.cells().len();
        match self {
            Self::Random { mines, seed } => Self::Random {
                mines: mines.min(len - 1),
                seed,
            },
            Self::Fixed(mut cells) => {
                cells.retain(|&cell| cell < len);
                cells.sort_unstable();
                cells.dedup();
                Self::Fixed(cells)
            }
        }
    }

    /// How many mines the layout lays.
    fn mine_count(&self) -> usize {
        match self {
            Self::Random { mines, .. } => *mines,
            Self::Fixed(cells) => cells.len(),
        }
    }

    /// The cells to mine on `board` when `first` is the first cell revealed.
    fn mined_cells(self, board: Board, first: usize) -> Vec<usize> {
        match self {
            Self::Random { mines, seed } => {
                let mut protected: Vec<usize> = std::iter::once(first)
                    .chain(board.neighbours(first))
                    .collect();
                if board.cells().len() - protected.len() < mines {
                    // Too crowded to clear the whole 3x3 block: protect only the first cell.
                    protected.truncate(1);
                }
                let mut candidates: Vec<usize> = board
                    .cells()
                    .filter(|cell| !protected.contains(cell))
                    .collect();
                candidates.shuffle(&mut StdRng::seed_from_u64(seed));
                candidates.truncate(mines);
                candidates
            }
            Self::Fixed(cells) => cells,
        }
    }
}

/// What the player has done to a cell. Whether the cell holds a mine is separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CellState {
    Covered,
    Flagged,
    Questioned,
    Revealed,
}

impl CellState {
    /// Flags protect a cell from being revealed; question marks do not.
    const fn can_be_revealed(self) -> bool {
        matches!(self, Self::Covered | Self::Questioned)
    }
}

#[derive(Clone, Copy, Debug)]
struct Cell {
    mine: bool,
    /// How many of the cell's neighbours hold mines.
    number: u8,
    state: CellState,
}

impl Cell {
    const FRESH: Self = Self {
        mine: false,
        number: 0,
        state: CellState::Covered,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameStatus {
    /// Waiting for the first reveal. No mines are laid yet, which is what makes the first
    /// reveal safe.
    Ready,
    Playing {
        started: Instant,
    },
    Won {
        seconds: u64,
    },
    /// `exploded` is the mine whose reveal ended the game.
    Lost {
        seconds: u64,
        exploded: usize,
    },
}

/// How a cell is drawn, derived from its state and the game status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellView {
    Covered,
    Flag,
    Question,
    /// A revealed safe cell and its number (0 draws blank).
    Open(u8),
    Mine,
    ExplodedMine,
    WrongFlag,
}

pub struct Game {
    board: Board,
    mines: usize,
    cells: Vec<Cell>,
    status: GameStatus,
    flags: usize,
    revealed: usize,
    /// The mines still to be laid: `Some` exactly while the game is `Ready`.
    layout: Option<MineLayout>,
}

impl Game {
    pub fn new(board: Board, layout: MineLayout) -> Self {
        let layout = layout.fitted(board);
        Self {
            board,
            mines: layout.mine_count(),
            cells: vec![Cell::FRESH; board.cells().len()],
            status: GameStatus::Ready,
            flags: 0,
            revealed: 0,
            layout: Some(layout),
        }
    }

    pub fn with_difficulty(difficulty: Difficulty) -> Self {
        let (cols, rows, mines) = difficulty.spec();
        Self::new(Board::new(cols, rows), MineLayout::random(mines))
    }

    pub fn board(&self) -> Board {
        self.board
    }

    pub fn status(&self) -> GameStatus {
        self.status
    }

    pub fn is_over(&self) -> bool {
        matches!(
            self.status,
            GameStatus::Won { .. } | GameStatus::Lost { .. }
        )
    }

    /// The counter shown to the player: negative when there are more flags than mines.
    pub fn mines_left(&self) -> i64 {
        self.mines as i64 - self.flags as i64
    }

    /// The timer at `now`: zero before the first reveal, frozen once the game is over.
    pub fn elapsed_seconds(&self, now: Instant) -> u64 {
        match self.status {
            GameStatus::Ready => 0,
            GameStatus::Playing { started } => now.saturating_duration_since(started).as_secs(),
            GameStatus::Won { seconds } | GameStatus::Lost { seconds, .. } => seconds,
        }
    }

    /// Reveals a covered cell. The first reveal lays the mines and starts the timer at `now`.
    pub fn reveal(&mut self, idx: usize, now: Instant) {
        let can_reveal = self
            .cells
            .get(idx)
            .is_some_and(|c| c.state.can_be_revealed());
        if self.is_over() || !can_reveal {
            return;
        }
        if let Some(layout) = self.layout.take() {
            self.lay_mines(layout, idx);
            self.status = GameStatus::Playing { started: now };
        }
        if self.cells[idx].mine {
            self.lose(idx, now);
        } else {
            self.flood_reveal(idx);
            self.check_win(now);
        }
    }

    /// Marks a covered cell, cycling flag -> question mark -> unmarked. Revealed cells
    /// cannot be marked.
    pub fn cycle_mark(&mut self, idx: usize) {
        if self.is_over() {
            return;
        }
        let Some(cell) = self.cells.get_mut(idx) else {
            return;
        };
        cell.state = match cell.state {
            CellState::Covered => {
                self.flags += 1;
                CellState::Flagged
            }
            CellState::Flagged => {
                self.flags -= 1;
                CellState::Questioned
            }
            CellState::Questioned => CellState::Covered,
            CellState::Revealed => return,
        };
    }

    /// Chords on a revealed number: when exactly that many of its neighbours are flagged,
    /// reveals every other covered neighbour.
    pub fn chord(&mut self, idx: usize, now: Instant) {
        if !matches!(self.status, GameStatus::Playing { .. }) {
            return;
        }
        let Some(&cell) = self.cells.get(idx) else {
            return;
        };
        if cell.state != CellState::Revealed || cell.number == 0 {
            return;
        }
        let flags = self
            .board
            .neighbours(idx)
            .filter(|&n| self.cells[n].state == CellState::Flagged)
            .count();
        if flags != usize::from(cell.number) {
            return;
        }

        let mut exploded = None;
        for n in self.board.neighbours(idx) {
            let neighbour = self.cells[n];
            if !neighbour.state.can_be_revealed() {
                continue;
            }
            if neighbour.mine {
                exploded.get_or_insert(n);
            } else {
                self.flood_reveal(n);
            }
        }
        match exploded {
            Some(mine) => self.lose(mine, now),
            None => self.check_win(now),
        }
    }

    pub fn view(&self, idx: usize) -> CellView {
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
            CellState::Covered => CellView::Covered,
            CellState::Flagged => CellView::Flag,
            CellState::Questioned => CellView::Question,
            CellState::Revealed => CellView::Open(cell.number),
        }
    }

    /// Lays the mines for a first reveal of `first`, then works out every cell's number.
    fn lay_mines(&mut self, layout: MineLayout, first: usize) {
        for cell in layout.mined_cells(self.board, first) {
            self.cells[cell].mine = true;
        }
        for i in self.board.cells() {
            let mines = self
                .board
                .neighbours(i)
                .filter(|&n| self.cells[n].mine)
                .count();
            self.cells[i].number = mines as u8;
        }
    }

    /// Reveals `start` and, through blank cells, everything connected to it (BFS).
    /// Numbers on the border are revealed but not expanded; flagged cells are never revealed.
    fn flood_reveal(&mut self, start: usize) {
        let mut queue = VecDeque::from([start]);
        while let Some(i) = queue.pop_front() {
            let cell = &mut self.cells[i];
            if cell.mine || !cell.state.can_be_revealed() {
                continue;
            }
            cell.state = CellState::Revealed;
            let blank = cell.number == 0;
            self.revealed += 1;
            if blank {
                queue.extend(
                    self.board
                        .neighbours(i)
                        .filter(|&n| self.cells[n].state.can_be_revealed()),
                );
            }
        }
    }

    fn lose(&mut self, exploded: usize, now: Instant) {
        self.status = GameStatus::Lost {
            seconds: self.elapsed_seconds(now),
            exploded,
        };
    }

    fn check_win(&mut self, now: Instant) {
        if self.revealed + self.mines != self.cells.len() {
            return;
        }
        for cell in self.cells.iter_mut().filter(|c| c.mine) {
            cell.state = CellState::Flagged;
        }
        self.flags = self.mines;
        self.status = GameStatus::Won {
            seconds: self.elapsed_seconds(now),
        };
    }
}

// =============================================================================
// Tests: through the interface only. A known board comes in as a fixed mine layout,
// and results are read back through `view`, `status` and the counters.
// =============================================================================

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rand::RngExt;

    use super::*;

    /// A new game on a known board, one string per row, where `*` is a mine.
    fn game_from(rows: &[&str]) -> Game {
        let mines = rows
            .concat()
            .char_indices()
            .filter(|&(_, ch)| ch == '*')
            .map(|(i, _)| i)
            .collect();
        let board = Board::new(rows[0].len(), rows.len());
        Game::new(board, MineLayout::Fixed(mines))
    }

    /// The board as the player sees it, one string per row: `#` covered, `F` flag,
    /// `?` question mark, `.` open blank, `1`-`8` open number, `*` mine, `X` the mine
    /// that exploded, `x` a wrong flag.
    fn render(game: &Game) -> Vec<String> {
        let board = game.board();
        let symbols: Vec<char> = board
            .cells()
            .map(|cell| match game.view(cell) {
                CellView::Covered => '#',
                CellView::Flag => 'F',
                CellView::Question => '?',
                CellView::Open(0) => '.',
                CellView::Open(n) => char::from(b'0' + n),
                CellView::Mine => '*',
                CellView::ExplodedMine => 'X',
                CellView::WrongFlag => 'x',
            })
            .collect();
        symbols
            .chunks(board.cols())
            .map(|row| row.iter().collect())
            .collect()
    }

    #[test]
    fn difficulty_presets_match_the_original() {
        assert_eq!(Difficulty::Beginner.spec(), (9, 9, 10));
        assert_eq!(Difficulty::Intermediate.spec(), (16, 16, 40));
        assert_eq!(Difficulty::Expert.spec(), (30, 16, 99));
    }

    #[test]
    fn cells_are_numbered_row_by_row() {
        let board = Board::new(4, 3);
        assert_eq!(board.cells(), 0..12);
        assert_eq!(board.cell(1, 2), 9);
        assert_eq!(board.position(9), (1, 2));
        assert_eq!(Board::new(0, 0).cells(), 0..1);
    }

    #[test]
    fn neighbours_stay_on_the_board() {
        let board = Board::new(4, 3);
        let counts: Vec<usize> = board.cells().map(|c| board.neighbours(c).count()).collect();
        assert_eq!(counts, [3, 5, 5, 3, 5, 8, 8, 5, 3, 5, 5, 3]);
        assert!(
            board
                .cells()
                .flat_map(|c| board.neighbours(c))
                .all(|n| n < 12)
        );
        assert_eq!(Board::new(1, 1).neighbours(0).count(), 0);
    }

    #[test]
    fn first_reveal_and_its_neighbours_are_always_safe() {
        let now = Instant::now();
        for difficulty in Difficulty::ALL {
            let (cols, rows, mines) = difficulty.spec();
            let board = Board::new(cols, rows);
            for seed in 0..200 {
                let first = StdRng::seed_from_u64(seed).random_range(board.cells());
                let mut game = Game::new(board, MineLayout::Random { mines, seed });
                game.reveal(first, now);

                // No neighbouring mines, so the first reveal always spreads.
                assert_eq!(game.view(first), CellView::Open(0));
                assert!(
                    board
                        .neighbours(first)
                        .all(|n| matches!(game.view(n), CellView::Open(_)))
                );

                // Reveal every covered cell in turn until the game ends: every mine is then
                // on show, as a mine if it was lost or as a flag if it was won.
                for cell in board.cells() {
                    game.reveal(cell, now);
                }
                assert!(game.is_over());
                let shown = board
                    .cells()
                    .filter(|&cell| {
                        matches!(
                            game.view(cell),
                            CellView::Mine | CellView::ExplodedMine | CellView::Flag
                        )
                    })
                    .count();
                assert_eq!(shown, mines);
            }
        }
    }

    #[test]
    fn crowded_board_still_protects_the_first_reveal() {
        let mut game = Game::new(Board::new(3, 3), MineLayout::Random { mines: 8, seed: 7 });
        game.reveal(4, Instant::now());
        assert!(matches!(game.status(), GameStatus::Won { .. }));
        assert_eq!(render(&game), ["FFF", "F8F", "FFF"]);
    }

    #[test]
    fn mine_count_is_capped_to_leave_a_safe_cell() {
        let game = Game::new(Board::new(2, 2), MineLayout::random(50));
        assert_eq!(game.mines_left(), 3);
    }

    #[test]
    fn fixed_layout_ignores_cells_off_the_board_and_repeats() {
        let game = Game::new(Board::new(2, 2), MineLayout::Fixed(vec![3, 3, 9]));
        assert_eq!(game.mines_left(), 1);
    }

    #[test]
    fn flood_fill_reveals_the_blank_region_and_its_numbered_border() {
        let mut game = game_from(&[
            "....", //
            "....", //
            "..**", //
            "..*.", //
        ]);
        game.reveal(0, Instant::now());
        assert_eq!(
            render(&game),
            [
                "....", //
                ".122", //
                ".2##", //
                ".2##", //
            ]
        );
        assert!(matches!(game.status(), GameStatus::Playing { .. }));
    }

    #[test]
    fn flood_fill_skips_flags_but_reveals_question_marks() {
        let mut game = game_from(&[
            "....", //
            "....", //
            "....", //
            "...*", //
        ]);
        game.cycle_mark(1); // flag
        game.cycle_mark(2);
        game.cycle_mark(2); // question mark
        game.reveal(12, Instant::now()); // blank region that touches both marked cells
        assert_eq!(
            render(&game),
            [
                ".F..", //
                "....", //
                "..11", //
                "..1#", //
            ]
        );
    }

    #[test]
    fn marking_cycles_and_updates_the_counter() {
        let mut game = game_from(&["*..", "...", "..."]);
        assert_eq!(game.mines_left(), 1);
        game.cycle_mark(4);
        assert_eq!((game.view(4), game.mines_left()), (CellView::Flag, 0));
        game.cycle_mark(5);
        assert_eq!(game.mines_left(), -1);
        game.cycle_mark(4);
        assert_eq!((game.view(4), game.mines_left()), (CellView::Question, 0));
        game.cycle_mark(4);
        assert_eq!((game.view(4), game.mines_left()), (CellView::Covered, 0));
    }

    #[test]
    fn flagged_cells_cannot_be_revealed_and_revealed_cells_cannot_be_marked() {
        let now = Instant::now();
        let mut game = game_from(&["*..", "...", "..*"]);
        game.cycle_mark(0);
        game.reveal(0, now); // flagged: no explosion, and not even a first reveal
        assert_eq!(game.status(), GameStatus::Ready);

        game.reveal(2, now);
        game.cycle_mark(1); // already revealed
        assert_eq!(render(&game), ["F1.", "#21", "###"]);
    }

    #[test]
    fn chord_reveals_neighbours_when_flags_match() {
        let now = Instant::now();
        let mut game = game_from(&["*..", "...", "..*"]);
        game.reveal(4, now);
        assert_eq!(game.view(4), CellView::Open(2));

        game.chord(4, now); // no flags yet: nothing happens
        game.cycle_mark(0);
        game.chord(4, now); // one flag of two: still nothing
        assert_eq!(render(&game), ["F##", "#2#", "###"]);

        game.cycle_mark(8);
        game.chord(4, now);
        assert!(matches!(game.status(), GameStatus::Won { .. }));
        assert_eq!(render(&game), ["F1.", "121", ".1F"]);
    }

    #[test]
    fn chord_with_a_misplaced_flag_loses() {
        let now = Instant::now();
        let mut game = game_from(&["*..", "...", "..."]);
        game.reveal(4, now); // a "1"
        game.cycle_mark(2); // wrong guess
        game.chord(4, now);
        assert!(matches!(
            game.status(),
            GameStatus::Lost { exploded: 0, .. }
        ));
        assert_eq!(render(&game), ["X1x", "11.", "..."]);
    }

    #[test]
    fn losing_shows_mines_and_wrong_flags() {
        let now = Instant::now();
        let mut game = game_from(&["*.*", "...", "*.."]);
        game.cycle_mark(0); // correct flag
        game.cycle_mark(8); // wrong flag
        game.reveal(2, now); // boom: a fixed layout does not protect the first reveal

        assert!(matches!(
            game.status(),
            GameStatus::Lost { exploded: 2, .. }
        ));
        assert_eq!(render(&game), ["F#X", "###", "*#x"]);

        // The board is frozen once the game is over.
        game.reveal(4, now);
        game.cycle_mark(4);
        game.chord(4, now);
        assert_eq!(render(&game), ["F#X", "###", "*#x"]);
    }

    #[test]
    fn winning_flags_every_remaining_mine() {
        let now = Instant::now();
        let mut game = game_from(&["*..", "...", "..*"]);
        game.cycle_mark(8); // question mark on a mine stays covered
        game.cycle_mark(8);
        for i in 1..8 {
            game.reveal(i, now);
        }
        assert!(matches!(game.status(), GameStatus::Won { .. }));
        assert_eq!(game.mines_left(), 0);
        assert_eq!(render(&game), ["F1.", "121", ".1F"]);
    }

    #[test]
    fn out_of_range_cells_are_ignored() {
        let now = Instant::now();
        let mut game = Game::with_difficulty(Difficulty::Beginner);
        game.reveal(usize::MAX, now);
        game.cycle_mark(81);
        game.chord(1_000, now);
        assert_eq!(game.status(), GameStatus::Ready);
        assert_eq!(game.mines_left(), 10);
    }

    #[test]
    fn timer_runs_from_the_first_reveal_and_stops_on_a_loss() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut game = game_from(&["*.", ".."]);
        assert_eq!(game.elapsed_seconds(at(30)), 0); // not started

        game.reveal(3, at(1));
        assert_eq!(game.elapsed_seconds(at(6)), 5);

        game.reveal(0, at(10));
        assert!(matches!(game.status(), GameStatus::Lost { seconds: 9, .. }));
        assert_eq!(game.elapsed_seconds(at(500)), 9);
    }

    #[test]
    fn timer_stops_on_a_win() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut game = game_from(&["*.", ".."]);
        game.reveal(1, at(0));
        game.reveal(2, at(3));
        game.reveal(3, at(7));
        assert_eq!(game.status(), GameStatus::Won { seconds: 7 });
        assert_eq!(game.elapsed_seconds(at(60)), 7);
    }
}
