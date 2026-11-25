//! The game model: the rules of play, independent of the UI.
//!
//! Callers play through [`Game`]'s interface: moves in ([`Game::reveal`], [`Game::cycle_mark`],
//! [`Game::chord`]), views out ([`Game::view`], [`Game::status`] and the counters). Where the
//! mines go comes in across the [`MineLayout`] seam, and the time comes in with each call, so a
//! game is fully determined by its inputs.

use std::collections::VecDeque;
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

/// Where the mines go. Chosen when the game is created and laid on the first reveal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MineLayout {
    /// `mines` cells picked by a random generator seeded with `seed`, keeping the first
    /// revealed cell clear and, when there is room, its neighbours too.
    Random { mines: usize, seed: u64 },
    /// Exactly these cells hold mines; indices off the board are ignored. The first reveal
    /// gets no protection.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the tests' adapter; play always uses `Random`")
    )]
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
pub enum GameStatus {
    /// Waiting for the first reveal. No mines are laid yet, which is what makes the first
    /// click safe.
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
pub enum CellView {
    Hidden,
    Flag,
    Question,
    /// A revealed safe cell and its adjacent-mine count (0 draws blank).
    Open(u8),
    Mine,
    ExplodedMine,
    WrongFlag,
}

pub struct Game {
    cols: usize,
    rows: usize,
    mines: usize,
    cells: Vec<Cell>,
    status: GameStatus,
    flags: usize,
    revealed: usize,
    /// The mines still to be laid: `Some` exactly while the game is `Ready`.
    layout: Option<MineLayout>,
}

impl Game {
    pub fn new(cols: usize, rows: usize, layout: MineLayout) -> Self {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let len = cols * rows;
        let layout = match layout {
            // One cell must stay free for the guaranteed-safe first click.
            MineLayout::Random { mines, seed } => MineLayout::Random {
                mines: mines.min(len - 1),
                seed,
            },
            MineLayout::Fixed(mut cells) => {
                cells.retain(|&i| i < len);
                cells.sort_unstable();
                cells.dedup();
                MineLayout::Fixed(cells)
            }
        };
        let mines = match &layout {
            MineLayout::Random { mines, .. } => *mines,
            MineLayout::Fixed(cells) => cells.len(),
        };
        Self {
            cols,
            rows,
            mines,
            cells: vec![Cell::FRESH; len],
            status: GameStatus::Ready,
            flags: 0,
            revealed: 0,
            layout: Some(layout),
        }
    }

    pub fn with_difficulty(difficulty: Difficulty) -> Self {
        let (cols, rows, mines) = difficulty.spec();
        Self::new(cols, rows, MineLayout::random(mines))
    }

    /// Board dimensions as `(columns, rows)`. Cells are indexed row by row from the top left.
    pub fn size(&self) -> (usize, usize) {
        (self.cols, self.rows)
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

    /// In-bounds neighbours of `idx`: 3 in a corner, 5 on an edge, 8 inside.
    /// The iterator does not borrow `self`, so callers may mutate while iterating.
    pub fn neighbors(&self, idx: usize) -> impl Iterator<Item = usize> + use<> {
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

    /// Left click: open a cell. The first reveal lays the mines and starts the clock at `now`.
    pub fn reveal(&mut self, idx: usize, now: Instant) {
        let openable = self.cells.get(idx).is_some_and(|c| c.state.is_openable());
        if self.is_over() || !openable {
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

    /// Right click: Hidden -> Flagged -> Questioned -> Hidden.
    pub fn cycle_mark(&mut self, idx: usize) {
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
    pub fn chord(&mut self, idx: usize, now: Instant) {
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
            CellState::Hidden => CellView::Hidden,
            CellState::Flagged => CellView::Flag,
            CellState::Questioned => CellView::Question,
            CellState::Revealed => CellView::Open(cell.adjacent),
        }
    }

    /// Lays the mines for a first reveal of `first`, then counts every cell's neighbouring mines.
    fn lay_mines(&mut self, layout: MineLayout, first: usize) {
        let mines = match layout {
            MineLayout::Random { mines, seed } => {
                self.scatter(mines, first, &mut StdRng::seed_from_u64(seed))
            }
            MineLayout::Fixed(cells) => cells,
        };
        for i in mines {
            self.cells[i].mine = true;
        }
        for i in 0..self.cells.len() {
            let count = self.neighbors(i).filter(|&n| self.cells[n].mine).count();
            self.cells[i].adjacent = count as u8;
        }
    }

    /// `mines` random cells, keeping `first` and, when there is room, its neighbours clear.
    fn scatter(&self, mines: usize, first: usize, rng: &mut StdRng) -> Vec<usize> {
        let mut protected: Vec<usize> = std::iter::once(first)
            .chain(self.neighbors(first))
            .collect();
        if self.cells.len() - protected.len() < mines {
            // Too crowded to clear the whole 3x3 block: protect only the clicked cell.
            protected.truncate(1);
        }
        let mut candidates: Vec<usize> = (0..self.cells.len())
            .filter(|i| !protected.contains(i))
            .collect();
        candidates.shuffle(rng);
        candidates.truncate(mines);
        candidates
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
        Game::new(rows[0].len(), rows.len(), MineLayout::Fixed(mines))
    }

    /// The board as the player sees it, one string per row: `#` covered, `F` flag,
    /// `?` question mark, `.` open blank, `1`-`8` open number, `*` mine, `X` the mine
    /// that exploded, `x` a wrong flag.
    fn render(game: &Game) -> Vec<String> {
        let (cols, rows) = game.size();
        (0..rows)
            .map(|y| {
                (0..cols)
                    .map(|x| match game.view(y * cols + x) {
                        CellView::Hidden => '#',
                        CellView::Flag => 'F',
                        CellView::Question => '?',
                        CellView::Open(0) => '.',
                        CellView::Open(n) => char::from(b'0' + n),
                        CellView::Mine => '*',
                        CellView::ExplodedMine => 'X',
                        CellView::WrongFlag => 'x',
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn difficulty_presets_match_the_original() {
        assert_eq!(Difficulty::Beginner.spec(), (9, 9, 10));
        assert_eq!(Difficulty::Intermediate.spec(), (16, 16, 40));
        assert_eq!(Difficulty::Expert.spec(), (30, 16, 99));
    }

    #[test]
    fn neighbours_stay_in_bounds() {
        let game = Game::new(4, 3, MineLayout::Fixed(Vec::new()));
        let counts: Vec<usize> = (0..12).map(|i| game.neighbors(i).count()).collect();
        assert_eq!(counts, [3, 5, 5, 3, 5, 8, 8, 5, 3, 5, 5, 3]);
        assert!((0..12).flat_map(|i| game.neighbors(i)).all(|n| n < 12));

        let single = Game::new(1, 1, MineLayout::Fixed(Vec::new()));
        assert_eq!(single.neighbors(0).count(), 0);
    }

    #[test]
    fn first_click_and_its_neighbours_are_always_safe() {
        let now = Instant::now();
        for difficulty in Difficulty::ALL {
            let (cols, rows, mines) = difficulty.spec();
            for seed in 0..200 {
                let click = StdRng::seed_from_u64(seed).random_range(0..cols * rows);
                let mut game = Game::new(cols, rows, MineLayout::Random { mines, seed });
                game.reveal(click, now);

                // No neighbouring mines, so the first click always opens an area.
                assert_eq!(game.view(click), CellView::Open(0));
                assert!(
                    game.neighbors(click)
                        .all(|n| matches!(game.view(n), CellView::Open(_)))
                );

                // Step on every covered cell in turn until the game ends: every mine is then
                // on show, as a mine if it was lost or as a flag if it was won.
                for i in 0..cols * rows {
                    game.reveal(i, now);
                }
                assert!(game.is_over());
                let shown = (0..cols * rows)
                    .filter(|&i| {
                        matches!(
                            game.view(i),
                            CellView::Mine | CellView::ExplodedMine | CellView::Flag
                        )
                    })
                    .count();
                assert_eq!(shown, mines);
            }
        }
    }

    #[test]
    fn crowded_board_still_protects_the_first_click() {
        let mut game = Game::new(3, 3, MineLayout::Random { mines: 8, seed: 7 });
        game.reveal(4, Instant::now());
        assert!(matches!(game.status(), GameStatus::Won { .. }));
        assert_eq!(render(&game), ["FFF", "F8F", "FFF"]);
    }

    #[test]
    fn mine_count_is_capped_to_leave_a_safe_cell() {
        assert_eq!(Game::new(2, 2, MineLayout::random(50)).mines_left(), 3);
    }

    #[test]
    fn fixed_layout_ignores_cells_off_the_board_and_repeats() {
        let game = Game::new(2, 2, MineLayout::Fixed(vec![3, 3, 9]));
        assert_eq!(game.mines_left(), 1);
    }

    #[test]
    fn flood_fill_opens_the_blank_region_and_its_numbered_border() {
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
    fn right_click_cycles_marks_and_updates_the_counter() {
        let mut game = game_from(&["*..", "...", "..."]);
        assert_eq!(game.mines_left(), 1);
        game.cycle_mark(4);
        assert_eq!((game.view(4), game.mines_left()), (CellView::Flag, 0));
        game.cycle_mark(5);
        assert_eq!(game.mines_left(), -1);
        game.cycle_mark(4);
        assert_eq!((game.view(4), game.mines_left()), (CellView::Question, 0));
        game.cycle_mark(4);
        assert_eq!((game.view(4), game.mines_left()), (CellView::Hidden, 0));
    }

    #[test]
    fn flagged_cells_cannot_be_revealed_and_revealed_cells_cannot_be_marked() {
        let now = Instant::now();
        let mut game = game_from(&["*..", "...", "..*"]);
        game.cycle_mark(0);
        game.reveal(0, now); // flagged: no explosion, and not even a first reveal
        assert_eq!(game.status(), GameStatus::Ready);

        game.reveal(2, now);
        game.cycle_mark(1); // already open
        assert_eq!(render(&game), ["F1.", "#21", "###"]);
    }

    #[test]
    fn chord_opens_neighbours_when_flags_match() {
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
    fn losing_reveals_mines_and_wrong_flags() {
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
    fn out_of_range_indices_are_ignored() {
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
