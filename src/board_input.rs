//! Board input: the classic mouse rules, turning pointer frames into moves on a [`Game`].
//!
//! It owns the gesture in progress, so the rules live here and nowhere else: releasing the
//! left button reveals, a lone right press marks at once, and both buttons together (or the
//! middle one) chord once, on the first release. It knows nothing of egui; the UI hands it
//! one [`PointerFrame`] per frame.

use std::time::Instant;

use crate::game::Game;

/// One flag per mouse button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buttons {
    pub left: bool,
    pub right: bool,
    pub middle: bool,
}

impl Buttons {
    const fn any(self) -> bool {
        self.left || self.right || self.middle
    }
}

/// What the board sees of the pointer in one frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct PointerFrame {
    /// The board cell under the pointer, if any.
    pub cell: Option<usize>,
    /// The pointer is over the board and nothing else (an open menu, say) lies over it there.
    /// Gestures only start when this holds.
    pub on_board: bool,
    pub down: Buttons,
    pub pressed: Buttons,
    pub released: Buttons,
}

/// How the board should look this frame while buttons are held.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HoldFeedback {
    /// Cells drawn pushed in.
    pub pressed_cells: Vec<usize>,
    /// A button is held down on the board. The face gasps at this while the game is on.
    pub holding: bool,
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

#[derive(Debug, Default)]
pub struct BoardInput {
    gesture: Gesture,
}

impl BoardInput {
    /// Advances the gesture by one frame, applies any move it completes to `game` at `now`,
    /// and says how the board should look.
    pub fn frame(&mut self, game: &mut Game, pointer: PointerFrame, now: Instant) -> HoldFeedback {
        self.advance(game, pointer, now);
        self.feedback(game, pointer)
    }

    fn advance(&mut self, game: &mut Game, pointer: PointerFrame, now: Instant) {
        let PointerFrame {
            cell,
            on_board,
            down,
            pressed,
            released,
        } = pointer;

        if self.gesture == Gesture::Idle && pressed.any() && on_board {
            self.gesture = Gesture::Press;
        }
        if self.gesture == Gesture::Press {
            // A lone right press marks immediately; with the left button held it starts a chord.
            if pressed.right
                && !down.left
                && !pressed.left
                && let Some(c) = cell
            {
                game.cycle_mark(c);
            }
            if (down.left && down.right) || down.middle || pressed.middle {
                self.gesture = Gesture::Chord;
            }
        }
        if released.any() {
            match self.gesture {
                Gesture::Chord => {
                    if let Some(c) = cell {
                        game.chord(c, now);
                    }
                    self.gesture = Gesture::Spent;
                }
                Gesture::Press if released.left => {
                    if let Some(c) = cell {
                        game.reveal(c, now);
                    }
                }
                _ => {}
            }
        }
        if !down.any() {
            self.gesture = Gesture::Idle;
        }
    }

    /// Cells look pushed in while held, as in the original: the one under the pointer, or
    /// it and its neighbours for a chord.
    fn feedback(&self, game: &Game, pointer: PointerFrame) -> HoldFeedback {
        let holding = match self.gesture {
            Gesture::Press => pointer.down.left,
            Gesture::Chord => true,
            Gesture::Idle | Gesture::Spent => false,
        };
        let pressed_cells = match pointer.cell {
            Some(cell) if holding && !game.is_over() => {
                if self.gesture == Gesture::Chord {
                    std::iter::once(cell)
                        .chain(game.board().neighbours(cell))
                        .collect()
                } else {
                    vec![cell]
                }
            }
            _ => Vec::new(),
        };
        HoldFeedback {
            pressed_cells,
            holding,
        }
    }
}

// =============================================================================
// Tests: mouse sequences in, moves and feedback out.
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Board, CellView, GameStatus, MineLayout};

    const NONE: Buttons = Buttons {
        left: false,
        right: false,
        middle: false,
    };
    const LEFT: Buttons = Buttons { left: true, ..NONE };
    const RIGHT: Buttons = Buttons {
        right: true,
        ..NONE
    };
    const MIDDLE: Buttons = Buttons {
        middle: true,
        ..NONE
    };

    /// A mouse driving board input on the 3x3 board below, where the middle cell is a 2:
    ///
    /// ```text
    /// * . .
    /// . . .
    /// . . *
    /// ```
    struct Mouse {
        input: BoardInput,
        game: Game,
        cell: Option<usize>,
        held: Buttons,
        /// Something lies over the board, so the pointer is over a cell but not on the board.
        obscured: bool,
    }

    impl Mouse {
        fn new() -> Self {
            Self {
                input: BoardInput::default(),
                game: Game::new(Board::new(3, 3), MineLayout::Fixed(vec![0, 8])),
                cell: None,
                held: NONE,
                obscured: false,
            }
        }

        fn step(&mut self, pressed: Buttons, released: Buttons) -> HoldFeedback {
            let next = |held: bool, p: bool, r: bool| (held || p) && !r;
            self.held = Buttons {
                left: next(self.held.left, pressed.left, released.left),
                right: next(self.held.right, pressed.right, released.right),
                middle: next(self.held.middle, pressed.middle, released.middle),
            };
            let pointer = PointerFrame {
                cell: self.cell,
                on_board: self.cell.is_some() && !self.obscured,
                down: self.held,
                pressed,
                released,
            };
            self.input.frame(&mut self.game, pointer, Instant::now())
        }

        fn move_to(&mut self, cell: Option<usize>) -> HoldFeedback {
            self.cell = cell;
            self.step(NONE, NONE)
        }

        fn press(&mut self, buttons: Buttons) -> HoldFeedback {
            self.step(buttons, NONE)
        }

        fn release(&mut self, buttons: Buttons) -> HoldFeedback {
            self.step(NONE, buttons)
        }

        /// A whole gesture on one cell: press `buttons` over `cell`, then release them.
        fn press_and_release(&mut self, cell: usize, buttons: Buttons) {
            self.move_to(Some(cell));
            self.press(buttons);
            self.release(buttons);
        }

        /// Reveals the 2 in the middle and flags both mines, ready to chord.
        fn ready_to_chord(&mut self) {
            self.press_and_release(4, LEFT);
            self.press_and_release(0, RIGHT);
            self.press_and_release(8, RIGHT);
            self.move_to(Some(4));
        }
    }

    fn sorted(mut cells: Vec<usize>) -> Vec<usize> {
        cells.sort_unstable();
        cells
    }

    #[test]
    fn left_release_reveals() {
        let mut mouse = Mouse::new();
        mouse.move_to(Some(4));
        let held = mouse.press(LEFT);
        assert_eq!(
            held,
            HoldFeedback {
                pressed_cells: vec![4],
                holding: true
            }
        );
        assert_eq!(mouse.game.view(4), CellView::Covered);

        assert_eq!(mouse.release(LEFT), HoldFeedback::default());
        assert_eq!(mouse.game.view(4), CellView::Open(2));
    }

    #[test]
    fn release_reveals_the_cell_under_the_pointer() {
        let mut mouse = Mouse::new();
        mouse.move_to(Some(4));
        mouse.press(LEFT);
        assert_eq!(mouse.move_to(Some(1)).pressed_cells, [1]);
        mouse.release(LEFT);
        assert_eq!(mouse.game.view(1), CellView::Open(1));
        assert_eq!(mouse.game.view(4), CellView::Covered);
    }

    #[test]
    fn releasing_off_the_board_reveals_nothing() {
        let mut mouse = Mouse::new();
        mouse.move_to(Some(4));
        mouse.press(LEFT);
        let dragged_off = mouse.move_to(None);
        assert_eq!(
            dragged_off,
            HoldFeedback {
                pressed_cells: Vec::new(),
                holding: true
            }
        );
        mouse.release(LEFT);
        assert_eq!(mouse.game.status(), GameStatus::Ready);
    }

    #[test]
    fn presses_that_start_off_the_board_are_ignored() {
        let mut mouse = Mouse::new();
        mouse.move_to(None);
        mouse.press(LEFT);
        assert_eq!(mouse.move_to(Some(4)), HoldFeedback::default());
        mouse.release(LEFT);
        assert_eq!(mouse.game.status(), GameStatus::Ready);
    }

    #[test]
    fn presses_on_something_over_the_board_are_ignored() {
        let mut mouse = Mouse::new();
        mouse.obscured = true;
        mouse.press_and_release(4, LEFT);
        mouse.press_and_release(1, RIGHT);
        assert_eq!(mouse.game.view(4), CellView::Covered);
        assert_eq!(mouse.game.view(1), CellView::Covered);
    }

    #[test]
    fn right_press_marks_at_once_and_its_release_does_nothing() {
        let mut mouse = Mouse::new();
        mouse.move_to(Some(4));
        assert_eq!(mouse.press(RIGHT), HoldFeedback::default());
        assert_eq!(mouse.game.view(4), CellView::Flag);
        mouse.release(RIGHT);
        assert_eq!(mouse.game.view(4), CellView::Flag);

        mouse.press_and_release(4, RIGHT);
        assert_eq!(mouse.game.view(4), CellView::Question);
    }

    #[test]
    fn left_and_right_together_chord_on_the_first_release() {
        let mut mouse = Mouse::new();
        mouse.ready_to_chord();
        mouse.press(LEFT);
        let held = mouse.press(RIGHT);
        assert!(held.holding);
        assert_eq!(sorted(held.pressed_cells), [0, 1, 2, 3, 4, 5, 6, 7, 8]);

        mouse.release(RIGHT);
        assert!(matches!(mouse.game.status(), GameStatus::Won { .. }));
    }

    #[test]
    fn middle_button_chords() {
        let mut mouse = Mouse::new();
        mouse.ready_to_chord();
        mouse.press(MIDDLE);
        mouse.release(MIDDLE);
        assert!(matches!(mouse.game.status(), GameStatus::Won { .. }));
    }

    #[test]
    fn right_press_while_left_is_held_starts_a_chord_instead_of_marking() {
        let mut mouse = Mouse::new();
        mouse.move_to(Some(1));
        mouse.press(LEFT);
        let held = mouse.press(RIGHT);
        assert_eq!(sorted(held.pressed_cells), [0, 1, 2, 3, 4, 5]);
        mouse.release(LEFT);
        mouse.release(RIGHT);
        assert_eq!(mouse.game.view(1), CellView::Covered);
    }

    #[test]
    fn after_a_chord_the_remaining_release_does_not_reveal() {
        let mut mouse = Mouse::new();
        mouse.press_and_release(4, LEFT); // the 2, with no flags around it: this chord does nothing
        mouse.press(LEFT);
        mouse.press(RIGHT);
        let spent = mouse.release(RIGHT);
        assert_eq!(spent, HoldFeedback::default());

        mouse.move_to(Some(2));
        mouse.release(LEFT);
        assert_eq!(mouse.game.view(2), CellView::Covered);
    }

    #[test]
    fn nothing_looks_pressed_once_the_game_is_over() {
        let mut mouse = Mouse::new();
        mouse.press_and_release(0, LEFT); // a mine
        assert!(matches!(mouse.game.status(), GameStatus::Lost { .. }));
        mouse.move_to(Some(4));
        assert_eq!(mouse.press(LEFT).pressed_cells, Vec::<usize>::new());
    }
}
