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
    /// The pointer is over the board and nothing else (an open menu, say) covers it there.
    /// Gestures only start when this holds.
    pub on_board: bool,
    pub down: Buttons,
    pub pressed: Buttons,
    pub released: Buttons,
}

/// How the board should look this frame while buttons are held.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Feedback {
    /// Cells drawn pushed in.
    pub pressed_cells: Vec<usize>,
    /// A button is held down on the board, so the face gasps.
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
    pub fn frame(&mut self, game: &mut Game, pointer: PointerFrame, now: Instant) -> Feedback {
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
    fn feedback(&self, game: &Game, pointer: PointerFrame) -> Feedback {
        let holding = match self.gesture {
            Gesture::Press => pointer.down.left,
            Gesture::Chord => true,
            Gesture::Idle | Gesture::Spent => false,
        };
        let pressed_cells = match pointer.cell {
            Some(cell) if holding && !game.is_over() => {
                if self.gesture == Gesture::Chord {
                    std::iter::once(cell).chain(game.neighbors(cell)).collect()
                } else {
                    vec![cell]
                }
            }
            _ => Vec::new(),
        };
        Feedback {
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
    use crate::game::{CellView, GameStatus, MineLayout};

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
        /// Something covers the board, so the pointer is over a cell but not on the board.
        covered: bool,
    }

    impl Mouse {
        fn new() -> Self {
            Self {
                input: BoardInput::default(),
                game: Game::new(3, 3, MineLayout::Fixed(vec![0, 8])),
                cell: None,
                held: NONE,
                covered: false,
            }
        }

        fn step(&mut self, pressed: Buttons, released: Buttons) -> Feedback {
            let next = |held: bool, p: bool, r: bool| (held || p) && !r;
            self.held = Buttons {
                left: next(self.held.left, pressed.left, released.left),
                right: next(self.held.right, pressed.right, released.right),
                middle: next(self.held.middle, pressed.middle, released.middle),
            };
            let pointer = PointerFrame {
                cell: self.cell,
                on_board: self.cell.is_some() && !self.covered,
                down: self.held,
                pressed,
                released,
            };
            self.input.frame(&mut self.game, pointer, Instant::now())
        }

        fn move_to(&mut self, cell: Option<usize>) -> Feedback {
            self.cell = cell;
            self.step(NONE, NONE)
        }

        fn press(&mut self, buttons: Buttons) -> Feedback {
            self.step(buttons, NONE)
        }

        fn release(&mut self, buttons: Buttons) -> Feedback {
            self.step(NONE, buttons)
        }

        fn click(&mut self, cell: usize, buttons: Buttons) {
            self.move_to(Some(cell));
            self.press(buttons);
            self.release(buttons);
        }

        /// Opens the 2 in the middle and flags both mines, ready to chord.
        fn ready_to_chord(&mut self) {
            self.click(4, LEFT);
            self.click(0, RIGHT);
            self.click(8, RIGHT);
            self.move_to(Some(4));
        }
    }

    fn sorted(mut cells: Vec<usize>) -> Vec<usize> {
        cells.sort_unstable();
        cells
    }

    #[test]
    fn left_click_reveals_on_release() {
        let mut mouse = Mouse::new();
        mouse.move_to(Some(4));
        let held = mouse.press(LEFT);
        assert_eq!(
            held,
            Feedback {
                pressed_cells: vec![4],
                holding: true
            }
        );
        assert_eq!(mouse.game.view(4), CellView::Hidden);

        assert_eq!(mouse.release(LEFT), Feedback::default());
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
        assert_eq!(mouse.game.view(4), CellView::Hidden);
    }

    #[test]
    fn releasing_off_the_board_reveals_nothing() {
        let mut mouse = Mouse::new();
        mouse.move_to(Some(4));
        mouse.press(LEFT);
        let dragged_off = mouse.move_to(None);
        assert_eq!(
            dragged_off,
            Feedback {
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
        assert_eq!(mouse.move_to(Some(4)), Feedback::default());
        mouse.release(LEFT);
        assert_eq!(mouse.game.status(), GameStatus::Ready);
    }

    #[test]
    fn presses_on_something_covering_the_board_are_ignored() {
        let mut mouse = Mouse::new();
        mouse.covered = true;
        mouse.click(4, LEFT);
        mouse.click(1, RIGHT);
        assert_eq!(mouse.game.view(4), CellView::Hidden);
        assert_eq!(mouse.game.view(1), CellView::Hidden);
    }

    #[test]
    fn right_press_marks_at_once_and_its_release_does_nothing() {
        let mut mouse = Mouse::new();
        mouse.move_to(Some(4));
        assert_eq!(mouse.press(RIGHT), Feedback::default());
        assert_eq!(mouse.game.view(4), CellView::Flag);
        mouse.release(RIGHT);
        assert_eq!(mouse.game.view(4), CellView::Flag);

        mouse.click(4, RIGHT);
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
    fn middle_click_chords() {
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
        assert_eq!(mouse.game.view(1), CellView::Hidden);
    }

    #[test]
    fn after_a_chord_the_remaining_release_does_not_reveal() {
        let mut mouse = Mouse::new();
        mouse.click(4, LEFT); // the 2, with no flags around it: this chord does nothing
        mouse.press(LEFT);
        mouse.press(RIGHT);
        let spent = mouse.release(RIGHT);
        assert_eq!(spent, Feedback::default());

        mouse.move_to(Some(2));
        mouse.release(LEFT);
        assert_eq!(mouse.game.view(2), CellView::Hidden);
    }

    #[test]
    fn nothing_looks_pressed_once_the_game_is_over() {
        let mut mouse = Mouse::new();
        mouse.click(0, LEFT); // a mine
        assert!(matches!(mouse.game.status(), GameStatus::Lost { .. }));
        mouse.move_to(Some(4));
        assert_eq!(mouse.press(LEFT).pressed_cells, Vec::<usize>::new());
    }
}
