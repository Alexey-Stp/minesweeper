# Minesweeper

A clone of classic Windows Minesweeper written in Rust with [`eframe`/`egui`](https://github.com/emilk/egui).
There are no image assets: the bevels, seven-segment counters and grid lines are vector-drawn, and the symbols are
Unicode emoji.

| File | What lives there |
| --- | --- |
| [`src/game.rs`](src/game.rs) | The rules of play: the board's shape, mine layout, reveal and flood fill, marks, chords, win and loss, the timer |
| [`src/board_input.rs`](src/board_input.rs) | The mouse rules: how presses and releases on the board become reveals, marks and chords |
| [`src/main.rs`](src/main.rs) | The egui side: window, layout, reading the pointer and painting |

The game's vocabulary (reveal, mark, chord, mine layout, …) is defined in [`GLOSSARY.md`](GLOSSARY.md).

## Run

```sh
cargo run --release
```

In VS Code, press <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>B</kbd> to run the game, or use **Terminal → Run Task…** and pick
**Run**, **Run (release)**, **Build** or **Test** (defined in [`.vscode/tasks.json`](.vscode/tasks.json)).

Requires a recent stable Rust toolchain (edition 2024). On Linux, eframe also needs the system windowing and
OpenGL development libraries; see the [egui README](https://github.com/emilk/egui#demo) for the package list.

## How to play

| Input | Action |
| --- | --- |
| Left button | Reveal a cell. The first reveal is always safe, and so are its neighbours when there is room. |
| Right button | Cycle a covered cell: flag 🚩 → question mark `?` → unmarked |
| Middle button, or left + right together | Chord: on a revealed number whose flags are all placed, reveal its other neighbours |
| The face above the board, or <kbd>F2</kbd> | New game |
| **Game** menu | New game, Beginner (9×9, 10 mines), Intermediate (16×16, 40), Expert (30×16, 99) |

The left counter shows mines minus flags (it can go negative). The right counter is the elapsed time, which
starts on your first reveal and stops when the game ends.

When you hit a mine, it turns red, the other mines are shown 💣 and misplaced flags are marked ❌.
When you clear every safe cell, the remaining mines are flagged for you and the face puts on sunglasses 😎.

## Tests

```sh
cargo test
```

Each file has its tests at the bottom, written against that module's interface:

- `game.rs` plays known boards through a fixed mine layout and reads back what the player would see, including
  the timer, which takes the time as an input.
- `board_input.rs` replays mouse sequences (press, drag, release, both buttons at once) and checks the moves and
  the pressed-in cells they produce.
- `main.rs` covers the counter digits, the face and mapping screen points to cells.

## License

[MIT](LICENSE)
